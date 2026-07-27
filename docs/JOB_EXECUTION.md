# Unified Job Execution

## Status

This document describes the implemented selected-job architecture. Apply and
dry-run expose complete or exact job selection through the CLI and consume one
owned, selected, resolved `ExecutionPlan`.

## Goals

- Keep one `ExecutionPlan` as the fact source for dry-run and apply.
- Represent providers, packages, actions, and links as typed jobs.
- Validate the complete configuration statically before selection.
- Validate every requested selector before runtime expression evaluation.
- Resolve only the selected execution closure.
- Preserve deterministic serial execution and failure isolation.
- Keep providers as internal requirements rather than public selectors.

## Non-goals

- Selecting a provider directly.
- Selecting one concrete name inside a provider Batch.
- Arbitrary job dependencies, topological scheduling, or concurrency.
- Inferring execution order from TOML declaration order or selector order.
- Persistent job state, retry, rollback, or resume.
- Turning dry-run into simulation or provider check into planning.

## CLI selection

Apply and dry-run accept the same scope and job selectors:

```text
dot apply
    [--target TARGET]
    [--profile PROFILE]
    [--job package:ID]...
    [--job action:ID]...
    [--job link:ID]...

dot dry-run
    [--target TARGET]
    [--profile PROFILE]
    [--job package:ID]...
    [--job action:ID]...
    [--job link:ID]...
```

`--job` is repeatable. Omitting it means all effective jobs. Supplying one or
more occurrences means exactly those typed package, action, and link jobs,
plus any providers required by selected provider-backed packages. Bare IDs,
unknown kinds, `provider:ID`, malformed IDs, and duplicate occurrences are
argument errors. Providers are not selectable.

The profile is optional. Omission and `--profile @root` both select the target
root; otherwise the value names one globally unique profile node in the target
tree. A target may be omitted only when exactly one configured target is
compatible with the current platform.

Selector values use their canonical spelling:

```text
package:editors
action:configure
link:nvim
```

Selector input order does not affect plan or execution order.

## Typed identity and selection

Internal and public job identities are deliberately different:

```rust
enum JobId {
    Provider(Identifier),
    Package(SelectorIdentifier),
    Action(SelectorIdentifier),
    Link(SelectorIdentifier),
}

enum JobSelector {
    Package(SelectorIdentifier),
    Action(SelectorIdentifier),
    Link(SelectorIdentifier),
}

enum JobSelection {
    All,
    Only(BTreeSet<JobSelector>),
}
```

`JobId` includes provider jobs because they are part of execution and reports.
`JobSelector` excludes them because providers are dependency closure, not user
work. Typed variants also keep equal text such as `package:setup`,
`action:setup`, and `link:setup` distinct.

Target, profile, package, action, and link IDs use the selector-safe grammar
from `SCHEMA.txt`. Provider IDs remain broad identifiers.

## Architecture and data flow

The command pipeline is:

```text
dot.toml
   |
   | parse + whole-configuration static validation
   v
Config
   |
   | select compatible target, select profile path, merge whole records
   v
EffectiveManifest                    (unresolved)
   |
   | validate JobSelection, compute provider closure,
   | resolve only selected providers and jobs
   v
ExecutionPlan                        (owned, selected, resolved)
   |
   +---- dry-run ----> CommandReport
   |
   `---- apply ------> JobRunner
                         |
                         v
                    JobExecutionReport
                         |
                         v
                    CommandReport
```

`EffectiveManifest` is the unresolved structural source after target/profile
merge. It can project borrowed package, action, and link identities for list
commands without resolving values.

`ExecutionPlanner` receives both `&EffectiveManifest` and `&JobSelection`.
Selection is therefore part of planning, not a filter applied after a larger
plan exists. The planner returns exactly one owned `ExecutionPlan` containing
only the selected resolved closure. There is no secondary selected-plan
wrapper, no second owned catalog IR, and no post-plan selection view.

`JobRunner::run` consumes `&ExecutionPlan` directly. Dry-run and apply project
that same plan; dry-run never invokes `JobRunner`.

## Static validation versus runtime resolution

Configuration loading statically validates the complete configuration before
scope or job selection:

- all TOML and schema shapes;
- target/profile/package/action/link selector IDs and broad provider IDs;
- profile-name uniqueness;
- every source expression's syntax, resolver name and payload, result type, and
  package-list-variable placement;
- provider references, Batch shape, and provider-args requirements in every
  target-root and root-to-profile effective merge.

This traversal includes unselected targets, unselected profiles and jobs, and
ancestor declarations replaced in a deeper effective merge. An unknown
resolver or malformed expression in any of them rejects the load.

Static validation does not evaluate runtime resolver values. It does not look
up a named environment variable, resolve a `dot` or `xdg` path, apply provider
activation, inspect filesystem state, or run a command.

After static validation and target/profile merge, the planner performs these
atomic steps:

1. Validate the complete `JobSelection` against unresolved typed identities.
2. For `Only`, add the minimum provider closure and reject a selected
   provider-backed package whose effective provider is absent.
3. Resolve selected providers.
4. Resolve selected provider-backed packages and provider install commands.
5. Resolve selected manual packages.
6. Resolve selected actions.
7. Resolve selected links and require absolute resolved targets.
8. Return the complete owned `ExecutionPlan`.

Every selector is checked before step 3 begins. A selection or resolution
failure returns no plan, so apply executes nothing. An unselected runtime error,
such as a missing environment value, is never evaluated. The same error in a
selected provider or job fails planning.

## Provider closure

`JobSelection::All` includes:

```text
all effective providers
all effective provider-backed packages
all effective manual packages
all effective actions
all effective links
```

Every effective provider is included even if no package references it.

`JobSelection::Only` includes only:

- each selected manual package, action, or link;
- each selected provider-backed package;
- the provider required by each selected provider-backed package.

Multiple selected packages sharing one provider include that provider once.
Selecting an action or link adds no earlier phase. Selecting one
provider-backed package does not add other packages that share its provider.
A package Batch remains one job and cannot be split by name.

## Stable plan and execution order

Both complete and exact plans use the same phase and subphase order:

```text
1. providers                 by provider ID
2. provider-backed packages by package ID
3. manual packages          by package ID
4. actions                  by action ID
5. links                    by link ID
```

The maps are `BTreeMap`-backed. Exact selection filters this canonical sequence
without reordering it. Provider-backed and manual packages remain separate
subphases rather than one mixed package-ID order.

`JobRunner` executes synchronously and serially:

1. Every selected provider completes its readiness lifecycle before any
   package starts: apply activation, probe, conditionally run ensure in
   declaration order, then reapply activation and probe once when ensure
   succeeds.
2. Every selected provider-backed package runs after the provider phase. It
   receives the successful in-memory `ProviderStatus` for its declared
   provider.
3. Selected manual-package actions run.
4. Selected global actions run.
5. Selected link targets are normalized first so duplicate resolved targets
   can be detected. Without a duplicate, target-normalization and source
   preparation errors remain attached to their individual links, and later
   nonblocked links reconcile in canonical order.

`ProviderStatus` is transient dependency output. It carries readiness and the
activated child environment needed by provider installs; it is not persisted
or exposed as a selectable job.

Job results are keyed by typed `JobId`. Reports are projected by traversing the
plan sequence, so result-map ordering cannot become a second order source.

## Failure behavior

Planning is all-or-nothing. TOML errors, complete static validation errors,
scope errors, unknown selectors, missing provider requirements, or selected
runtime resolution errors happen before any execution.

Runtime failures remain local where execution can continue safely:

- a provider failure blocks only selected packages that require that provider;
- an unrelated provider and its selected packages still run;
- failure of one provider install unit does not block later install units,
  including another unit using the same provider;
- manual-package and action failures do not stop unrelated later jobs;
- only duplicate resolved targets block the complete selected link phase before
  link mutation;
- target-normalization, source-preparation, and reconciliation failures fail
  only that link and do not stop later nonblocked links.

Every planned job receives exactly one completed or blocked result. Apply
returns a failed report when any selected job fails or is blocked.

Provider commands retain their existing stream behavior: probes and checks use
captured output, while ensure, install, and action exec processes inherit the
terminal streams. Execution remains blocking, foreground, and serial.

## Apply, dry-run, and provider check

Apply and dry-run share this construction:

```text
load and statically validate the complete configuration
    -> select and merge target/profile
    -> validate selectors and provider closure
    -> resolve selected runtime values
    -> construct one ExecutionPlan
```

Apply passes `&ExecutionPlan` directly to `JobRunner`, then combines plan facts
with typed execution outcomes. Dry-run projects the same plan directly into
`PLANNED` report items and performs no execution. It does not probe providers,
run checks or exec actions, inspect link state, or mutate the filesystem.

`dot check providers` is a separate diagnostic path. It does not accept job
selectors, construct an `ExecutionPlan`, or invoke `JobRunner`. After complete
static validation and target/profile selection, it independently attempts
every effective provider. Activation resolution/application or probe
resolution/preparation can produce that provider's `NOT_READY` result before
process launch. When preparation succeeds, the probe process executes at most
once. Any provider-local activation, interpolation, preparation,
process-launch, or probe failure does not stop later providers from being
attempted. Provider check does not runtime-resolve or execute ensure, install,
packages, actions, or links.

The structural list commands also do not construct an `ExecutionPlan`.
`list jobs` stops at the unresolved merged manifest and lists only packages,
actions, and links; providers never appear as selectable rows.

## Reporting

Dry-run and apply reports contain exactly the plan closure. A selected
provider-backed package therefore causes its provider to appear before it,
while a selected manual package, action, or link appears alone.

The presentation-independent report vocabulary remains:

- provider results: `READY` or `NOT_READY`;
- provider-package results: `INSTALLED`, `FAILED`, or `BLOCKED`;
- manual-package results: `SATISFIED`, `INSTALLED`, or `FAILED`;
- global-action results: `SATISFIED`, `EXECUTED`, or `FAILED`;
- link results: `SATISFIED`, `CREATED`, `REPLACED`, `SKIPPED`, `FAILED`, or
  `BLOCKED`.

The human table renderer is not a stable serialized interface. The stable
machine-facing selection interface is the headerless TSV emitted by the list
commands.

## Module boundaries

```text
config.rs        complete parsing and static validation entry point
validation.rs    whole-tree expression and effective-merge validation
manifest.rs      scope selection, profile merge, unresolved job projection
job.rs           JobId, JobSelector, and JobSelection
plan.rs          selection, provider closure, runtime resolution, ExecutionPlan
job_runner.rs    serial phase orchestration over &ExecutionPlan
job_executor.rs  typed provider/action/link execution adapter
dry_run.rs       deterministic ExecutionPlan report projection
check.rs         independent provider-only diagnostic path
```

These boundaries keep configuration facts, selected resolved intent, runtime
outcomes, and presentation separate while preserving one execution order and
one owned plan.

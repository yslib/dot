# Unified Job Execution

## Status

This document records the approved internal design for replacing dot's
phase-specific apply loops with one typed, serial job model.

The first implementation does not change the CLI. It prepares the execution
model for a later CLI revision that can select one package, action, or link by
ID.

## Goals

- Keep one resolved `ExecutionPlan` as the fact source for dry-run and apply.
- Represent providers, packages, actions, and links as typed jobs.
- Execute a complete apply serially in provider, package, action, and link
  order.
- Allow a future request to select one package, action, or link without adding
  another execution path.
- Automatically include the required provider when a provider-backed package
  is selected.
- Continue unrelated jobs after an ordinary runtime failure.
- Preserve stable report ordering.

## Non-goals

- Concurrent or parallel execution.
- Lanes, worker pools, async runtimes, or scheduling resource declarations.
- User-defined dependency graphs.
- Selecting a provider directly.
- Runtime job registration or plugins.
- Signal handling, process-tree supervision, Job Objects, or recursive kill.
- Persisted job state, retry, resume, rollback, or receipts.
- New task-selection CLI syntax in the first implementation.

dot continues to rely on normal operating-system foreground-process behavior
for Ctrl-C. Commands continue to inherit stdin, stdout, and stderr as they do
today. Processes that detach, change their process group/session, or ignore
normal interruption are outside dot's guarantees.

## Why execution is serial

A provider ID is not a concurrency boundary. Different providers can mutate
the same external resource. For example, pacman and paru both install through
the same libalpm package database and cannot safely run installation
transactions concurrently.

Modeling safe concurrency would require new configuration concepts such as
shared resource or mutex identifiers. That is outside dot's bootstrap-focused
scope. Package batches already provide the useful optimization by installing
multiple package names in one provider invocation.

## Architecture

`ExecutionPlan` becomes a single typed job collection. There is no additional
job IR.

```text
dot.toml
   |
   v
EffectiveManifest
   | select, merge, validate, resolve
   v
ExecutionPlan { jobs: Vec<PlannedJob> }
   |
   +-- JobSelection::All
   `-- JobSelection::Only(package/action/link)
                 |
                 | include a true provider requirement
                 v
        SelectedExecutionPlan
                 |
        +--------+--------+
        |                 |
        v                 v
   DryRunRenderer      JobRunner
                         |
                         v
                    JobExecutor
                         |
                         v
                    JobOutcome
                         |
                         v
                   CommandReport
```

`SelectedExecutionPlan` is a view over the source plan. It retains references or
indices into the plan rather than copying and re-resolving job data:

```rust
struct SelectedExecutionPlan<'a> {
    source: &'a ExecutionPlan,
    selected: BTreeSet<JobId>,
}
```

## Typed jobs and identity

The job model is closed:

```rust
enum PlannedJob {
    Provider(PlannedProviderJob),
    Package(PlannedPackageJob),
    Action(PlannedActionJob),
    Link(PlannedLinkJob),
}

enum JobId {
    Provider(Identifier),
    Package(Identifier),
    Action(Identifier),
    Link(Identifier),
}
```

The enum variant scopes the declaration ID. A package, action, and link may use
the same textual key without becoming the same job.

Provider-backed package singles and batches remain distinct strong variants
inside `PlannedPackageJob`. Manual packages are also package jobs but have no
provider requirement.

Provider and action command lifecycles remain internal to one job:

```text
provider job:
activate -> probe -> [ensure steps -> activate -> final probe]

action or manual-package job:
[initial check] -> exec -> [post-check]
```

Those command stages are not separate selectable jobs.

## Selection

The internal selection model supports a set so it does not constrain a future
CLI, although the first future CLI revision accepts only one selector:

```rust
enum JobSelector {
    Package(Identifier),
    Action(Identifier),
    Link(Identifier),
}

enum JobSelection {
    All,
    Only(BTreeSet<JobSelector>),
}
```

Providers are internal requirements and cannot be selected by the user.

Selection computes only the minimum executable closure:

- Selecting a provider-backed package includes that package and its provider.
- Selecting a manual package includes only that package.
- Selecting an action includes only that action.
- Selecting a link includes only that link.
- Selecting one provider-backed package does not include other packages using
  the same provider.

For example:

```text
package:cli-tools
    `-- requires provider:scoop
```

An unknown selector or missing provider requirement is reported before any
selected job executes.

Duplicate typed job IDs are prevented structurally rather than diagnosed as a
second user-facing condition. Each effective domain collection is a TOML table,
profile merging replaces values by key, and planning creates exactly one job
from each effective entry. `JobId` adds the domain variant, so equal spelling in
different domains remains unambiguous.

The first CLI always constructs `JobSelection::All`. Adding CLI selectors later
only changes request construction.

## Serial execution order

A full apply uses this stable order:

```text
all selected providers
    -> all selected packages
    -> all selected actions
    -> all selected links
```

Within each kind, jobs retain the deterministic order assigned by planning.
Every planned job has a stable ordinal used by execution and reporting.

An exact selection runs only its minimum closure:

```text
selected provider-backed package:
provider -> package

selected manual package:
package

selected action:
action

selected link:
link
```

Phase order is not an implicit dependency. Selecting an action does not select
all providers and packages that precede it in a full apply.

## JobRunner and JobExecutor

`JobRunner` is the single execution entry point. It owns serial traversal,
closed variant dispatch, requirement checks, and result collection. It does not
contain provider, action, or link lifecycle implementation details.

```rust
struct JobRunner<'a> {
    executor: JobExecutor<'a>,
}

impl JobRunner<'_> {
    fn run(&self, plan: &SelectedExecutionPlan<'_>) -> JobExecutionReport;
}
```

`JobExecutor` is a closed domain adapter. Its methods delegate to existing
provider, action, and link runners:

```rust
impl JobExecutor<'_> {
    fn ensure_provider(&self, job: &PlannedProvider) -> ProviderStatus;
    fn install_provider_package(
        &self,
        job: &PlannedProviderInstall,
        provider: &ProviderStatus,
    ) -> ProviderInstallStatus;
    fn install_manual_package(&self, job: &PlannedManualPackage) -> ActionResult;
    fn run_action(&self, job: &PlannedAction) -> ActionResult;
    fn reconcile_links(&self, jobs: &[&PlannedLink]) -> LinkPhaseResult;
}
```

The `PlannedJob` match exists once in `JobRunner`. `JobExecutor` deliberately
does not add a generic dependency container or a second task graph. Links use a
phase method because duplicate-target validation must inspect the complete
selected link set before mutation.

A successful provider job produces an in-memory output containing its activated
child environment. A dependent package receives that status directly from the
runner's typed result map. This data exists only for the current invocation and
is not persisted.

Provider and action runners continue to use the existing blocking
`ProcessExecutor`. Link jobs continue to use Rust filesystem APIs.

## Failure behavior

Runtime failure does not stop unrelated jobs:

- A provider failure blocks only packages that require that provider.
- A package failure does not stop later packages.
- An action failure does not stop later actions or links.
- A link failure does not stop later links.

Requirements and serial order are different concepts. Two packages using the
same provider run one after another, but the first package is not a requirement
of the second.

Scheduling state remains separate from typed domain outcomes:

```rust
enum JobState {
    Completed(JobOutcome),
    Blocked(BlockReason),
}

enum JobOutcome {
    Provider(ProviderOutcome),
    Package(PackageOutcome),
    Action(ActionOutcome),
    Link(LinkOutcome),
}
```

Configuration parsing, target/profile selection, interpolation, planning, and
job selection errors happen before execution. Runtime provider, command, and
filesystem errors become per-job report results.

## Links

Links use the same selection and serial result path as other jobs but do not
start child processes.

Link preparation validates the selected link set and normalizes its targets
before mutation. The existing duplicate-target behavior remains unchanged: a
duplicate target is a phase-level error, so every selected link is reported as
blocked and no link mutation starts. This preserves the current safety boundary
without adding partial recovery.

Each prepared link is then reconciled as one `PlannedJob::Link` in stable order.

## Apply, dry-run, and provider check

The initially exposed operations remain unchanged:

```text
dot
    select All
    execute serially

dot --dry-run
    select All
    render the selected plan without execution

dot check providers
    retain its existing probe-only diagnostic path
```

Provider check does not use `ExecutionPlan` or `JobRunner`. It intentionally
resolves only provider activate and probe fields so expression errors in ensure,
package installs, manual packages, actions, and links remain outside its
diagnostic scope. Forcing it through the complete apply plan would break that
contract or require a second partial provider-job shape.

Provider check retains the existing `ProviderChecker` and blocking
`ProcessExecutor`. It remains serial, probes every effective provider once, and
never runs ensure or package installation.

Dry-run never invokes `JobRunner`. It visits the same selected job data and
projects planned report items.

## Reporting

`JobRunner` records results by typed `JobId`. `CommandReport` projects them in
the stable ordinal order from `ExecutionPlan`.

```rust
struct JobExecutionReport {
    results: BTreeMap<JobId, JobState>,
}
```

The map key is the sole stored job identity; `JobState` does not repeat it.
Because execution is serial, live child output remains naturally grouped by
command. The final report retains the existing typed statuses such as READY,
INSTALLED, EXECUTED, CREATED, BLOCKED, and FAILED.

Ctrl-C uses the operating system's default termination behavior. An interrupted
dot process does not construct or render a final report.

## Suggested module boundaries

```text
plan.rs          ExecutionPlan and typed PlannedJob values
job.rs           JobId, selector, selection, state, and outcomes
job_runner.rs    serial traversal, closed dispatch, requirements, and results
job_executor.rs  closed domain adapter
provider.rs      provider lifecycle
action_runner.rs action lifecycle
link.rs          link preparation and one-link reconciliation
report.rs        deterministic report projection
check.rs         existing provider-only diagnostic path
```

No scheduler, lane, cancellation, process-supervisor, platform-process, thread,
channel, or async-runtime module is required.

## Verification

Tests should verify:

1. Full selection preserves provider, package, action, and link order.
2. Exact provider-backed package selection adds only its provider.
3. Exact manual package, action, and link selection adds no unrelated jobs.
4. Unknown selectors fail before execution.
5. A provider failure blocks only its dependent packages.
6. Package, action, and link failures do not stop later unrelated jobs.
7. A failed package does not block the next package using the same provider.
8. Provider output supplies the activated environment to its packages.
9. Dry-run and apply consume the same selected job data.
10. Report order is stable.
11. Existing provider-check tests continue to prove that it resolves only
    activate/probe and remains serial.
12. A duplicate link target blocks the selected link phase before mutation.

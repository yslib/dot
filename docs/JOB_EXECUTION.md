# Unified Job Execution

## Status

`DESIGN.txt` is the canonical definition of runtime semantics. This document
records the implemented internal job model and its module boundaries.

The current CLI does not expose job selection. Its apply and dry-run paths
construct `JobSelection::All`, while the internal model supports exact package,
action, and link selection.

## Goals

- Keep one resolved `ExecutionPlan` as the fact source for dry-run and apply.
- Represent providers, packages, actions, and links as typed jobs.
- Execute a complete apply serially in provider, package, action, and link
  order.
- Keep exact package, action, and link selection in the same execution path.
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
- New job-selection syntax in the current CLI.

dot continues to rely on normal operating-system foreground-process behavior
for Ctrl-C. Provider probes and action checks capture stdout and stderr with
stdin connected to null. Provider ensure and install commands, action exec, and
manual-package exec inherit terminal stdin, stdout, and stderr. Processes that
detach, change their process group/session, or ignore normal interruption are
outside dot's guarantees.

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

`ExecutionPlan` is a single typed job collection. There is no additional
job IR.

```text
dot.toml
   |
   | parse, select, merge
   v
EffectiveManifest
   |
   | validate, resolve, plan
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
          +-----------+-----------+
          |                       |
          v                       v
 dry_run::build_report         JobRunner
          |                       |
          v                       v
    CommandReport         JobExecutionReport
                                  |
                                  v
                     apply report projection
                                  |
                                  v
                            CommandReport
```

`SelectedExecutionPlan` is a view over the source plan. It stores a source-plan
reference and an owned set of selected typed job IDs. Job access filters the
source sequence without copying or re-resolving job data:

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
    Provider(PlannedProvider),
    Package(PlannedPackage),
    Action(PlannedAction),
    Link(PlannedLink),
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
inside `PlannedPackage`. Manual packages are also package jobs but have no
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

The internal selection model supports a set independently of any request
surface:

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

The current dry-run and apply paths always construct `JobSelection::All`.

## Serial execution order

A full apply uses this stable order:

```text
all selected providers
    -> all selected packages
    -> all selected actions
    -> all selected links
```

Within each kind, jobs retain the deterministic order assigned by planning.
Every planned job has a stable position in the single sequence used by
execution and reporting.

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
pub struct JobRunner<'a> {
    executor: JobExecutor<'a>,
}

impl JobRunner<'_> {
    pub fn run(&self, plan: &SelectedExecutionPlan<'_>) -> JobExecutionReport;
}
```

`JobExecutor` is a crate-private closed domain adapter. Its type and methods are
not public; they delegate to existing provider, action, and link runners:

```rust
pub(crate) struct JobExecutor<'a> {
    provider_runner: ProviderRunner<'a>,
    action_runner: ActionRunner<'a>,
}

impl JobExecutor<'_> {
    pub(crate) fn ensure_provider(&self, job: &PlannedProvider) -> ProviderStatus;
    pub(crate) fn install_provider_package(
        &self,
        job: &PlannedProviderInstall,
        provider: &ProviderStatus,
    ) -> ProviderInstallStatus;
    pub(crate) fn install_manual_package(
        &self,
        job: &PlannedManualPackage,
    ) -> Result<ActionOutcome, ActionRunError>;
    pub(crate) fn run_action(
        &self,
        job: &PlannedAction,
    ) -> Result<ActionOutcome, ActionRunError>;
    pub(crate) fn reconcile_links<'p>(
        &self,
        jobs: impl IntoIterator<Item = &'p PlannedLink>,
    ) -> Result<LinkReport, LinkPhaseError>;
}
```

The `PlannedJob` match exists once in `JobRunner`. `JobExecutor` deliberately
does not add a generic dependency container or a second task graph. Links use a
phase method because duplicate-target validation must inspect the complete
selected link set before mutation.

A successful provider job produces an in-memory `ProviderStatus` containing its
activated child environment. `JobRunner` keeps that status only in a typed
transient dependency-output map for the current run, and dependent packages
borrow it from there. `ProviderRunner::install` rejects a provider status whose
provider ID differs from the install job's declared provider. After ordinary
jobs and links finish, the runner consumes each status into its ID-free
`Result<ProviderOutcome, ProviderError>` payload; the activated environment and
the status's provider ID are dropped rather than persisted in the execution
report.

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
    Provider(Result<ProviderOutcome, ProviderError>),
    ProviderPackage(Result<ProviderInstallOutcome, ProviderInstallError>),
    ManualPackage(Result<ActionOutcome, ActionRunError>),
    Action(Result<ActionOutcome, ActionRunError>),
    Link(Result<LinkOutcome, LinkError>),
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

Before associating link results with jobs, `JobRunner` unconditionally verifies
the complete result count and each result ID in order. These checks remain
active in release builds. A mismatch is an internal contract violation and
fails fast; it is not disguised as a link phase error.

## Apply, dry-run, and provider check

The current operations are:

```text
dot
    select All
    pass the selected view to JobRunner for serial execution
    project the typed results in selected/source order

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
the stable selected/source order from `ExecutionPlan`.

```rust
struct JobExecutionReport {
    results: BTreeMap<JobId, JobState>,
    link_phase_error: Option<LinkPhaseError>,
}
```

The result map does not store a second copy of job order. Its typed `JobId` key
is the sole stored report identity; `JobState` stores only ID-free domain
outcomes and does not repeat it. `ProviderStatus`, including the activated
environment needed by provider-package jobs, is transient dependency output
and is never stored in `JobExecutionReport`.

Every result insertion enforces key uniqueness in debug and release builds.
Before returning a report, `JobRunner` also verifies that the result count
equals the selected-job count. A duplicate or incomplete report is an internal
contract violation and fails fast.

Because execution is serial, live child output remains naturally grouped by
command. The final report retains the existing typed statuses such as READY,
INSTALLED, EXECUTED, CREATED, BLOCKED, and FAILED.

Ctrl-C uses the operating system's default termination behavior. An interrupted
dot process does not construct or render a final report.

## Module boundaries

```text
plan.rs          ExecutionPlan and typed PlannedJob values
job.rs           JobId, JobKind, JobSelector, and JobSelection identity vocabulary
job_runner.rs    BlockReason, JobOutcome, JobState, JobExecutionReport, and serial traversal
job_executor.rs  crate-private closed domain adapter
provider.rs      provider lifecycle
action_runner.rs action lifecycle
link.rs          link preparation and one-link reconciliation
report.rs        report types
app/apply.rs     deterministic typed result projection
dry_run.rs       deterministic selected-plan projection
check.rs         existing provider-only diagnostic path
```

No scheduler, lane, cancellation, process-supervisor, platform-process, thread,
channel, or async-runtime module is required.

## Verification

Tests verify:

1. Full selection preserves provider, package, action, and link order.
2. Exact provider-backed package selection adds only its provider.
3. Exact manual package, action, and link selection adds no unrelated jobs.
4. Unknown selectors fail before execution.
5. A provider failure blocks only its dependent packages.
6. Package, action, and link failures do not stop later unrelated jobs.
7. A real runner test proves that a failed package does not block the next
   package using the same provider and that both installs receive the activated
   provider environment.
8. Provider output supplies the activated environment to its packages.
9. Dry-run and apply consume the same selected job data.
10. Report order is stable.
11. Existing provider-check tests continue to prove that it resolves only
    activate/probe and remains serial.
12. A duplicate link target blocks the selected link phase before mutation.

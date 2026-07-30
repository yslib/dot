# Unified Serial Job Execution Implementation Plan

> **Historical document:** This plan was completed for v0.1.0 and is retained
> only as implementation history. It is not an active task list or a normative
> specification. See [JOB_EXECUTION.md](JOB_EXECUTION.md) and
> [DESIGN.txt](DESIGN.txt) for the implemented behavior.

**Goal:** Replace apply's phase-specific data paths with one strongly typed, serial job plan that dry-run, apply, and future exact package/action/link selection can share.

**Architecture:** `ExecutionPlan` owns one ordered `Vec<PlannedJob>`. `SelectedExecutionPlan` filters that fact source and adds only the provider required by a selected provider-backed package. `JobRunner` walks the selected jobs serially, delegates domain behavior to a closed `JobExecutor`, and records typed outcomes for deterministic report projection. Existing process spawning, Ctrl-C behavior, provider-check behavior, and link phase safety remain unchanged.

**Tech Stack:** Rust 2024, standard library collections, existing serde/TOML configuration pipeline, existing blocking `ProcessExecutor`, Rust integration tests and TOML fixtures.

---

## File structure

The finished change should have these responsibilities:

- `src/job.rs`: typed job IDs, future selectors, selection values, execution states, and typed outcomes.
- `src/plan.rs`: resolved `ExecutionPlan`, ordered `PlannedJob` values, and selected-plan view.
- `src/job_executor.rs`: closed typed adapter over existing provider/action/link domain runners.
- `src/job_runner.rs`: strictly serial traversal, provider requirement lookup, blocked-state creation, link phase coordination, and result collection.
- `src/provider.rs`: existing provider behavior plus reusable one-provider and one-install-unit entry points.
- `src/action_runner.rs`: unchanged action lifecycle used by `JobExecutor`.
- `src/link.rs`: existing serial link reconciliation adapted to consume selected link references.
- `src/dry_run.rs`: report projection by visiting selected jobs.
- `src/app/apply.rs`: app facade that builds the all-job selection, invokes `JobRunner`, and projects the result.
- `src/app/dry_run.rs`: app facade that builds the all-job selection without executing it.
- `src/check.rs` and `src/app/check_providers.rs`: intentionally remain on the existing probe-only path.
- `tests/job.rs`: job identity, order, and selection tests.
- `tests/job_runner.rs`: serial execution, dependency blocking, continuation, and exact-selection tests.
- `tests/fixtures/jobs/valid-serial-execution-template.toml`: complete executable job fixture.
- `docs/DESIGN.txt`: canonical internal execution description after implementation.

No CLI file, schema type, TOML field, async runtime, thread, lane, signal handler, or platform process API is added.

### Task 1: Add typed job identity and selection vocabulary

**Files:**
- Create: `src/job.rs`
- Modify: `src/lib.rs`
- Create: `tests/job.rs`

- [ ] **Step 1: Write the failing job identity test**

Create `tests/job.rs` with the identity-level test:

```rust
use dot::job::{JobId, JobKind, JobSelection, JobSelector};
use dot::schema::Identifier;

fn id(value: &str) -> Identifier {
    Identifier::new(value).expect("test identifier should be valid")
}

#[test]
fn job_identity_is_scoped_by_kind() {
    let package = JobId::Package(id("shared"));
    let action = JobId::Action(id("shared"));
    let link = JobId::Link(id("shared"));

    assert_ne!(package, action);
    assert_ne!(action, link);
    assert_eq!(package.kind(), JobKind::Package);
    assert_eq!(package.name(), "shared");
}

#[test]
fn exact_selection_keeps_its_typed_selector() {
    let selection = JobSelection::only(JobSelector::Package(id("cli-tools")));

    assert!(matches!(
        selection,
        JobSelection::Only(ref selectors)
            if selectors.contains(&JobSelector::Package(id("cli-tools")))
    ));
}
```

- [ ] **Step 2: Run the test to verify that the module is absent**

Run:

```bash
cargo test --test job
```

Expected: compilation fails because `dot::job` does not exist.

- [ ] **Step 3: Implement the minimal closed identity model**

Create `src/job.rs`:

```rust
use std::collections::BTreeSet;

use crate::schema::Identifier;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobKind {
    Provider,
    Package,
    Action,
    Link,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum JobId {
    Provider(Identifier),
    Package(Identifier),
    Action(Identifier),
    Link(Identifier),
}

impl JobId {
    pub const fn kind(&self) -> JobKind {
        match self {
            Self::Provider(_) => JobKind::Provider,
            Self::Package(_) => JobKind::Package,
            Self::Action(_) => JobKind::Action,
            Self::Link(_) => JobKind::Link,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Provider(id)
            | Self::Package(id)
            | Self::Action(id)
            | Self::Link(id) => id.as_str(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum JobSelector {
    Package(Identifier),
    Action(Identifier),
    Link(Identifier),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobSelection {
    All,
    Only(BTreeSet<JobSelector>),
}

impl JobSelection {
    pub fn only(selector: JobSelector) -> Self {
        Self::Only(BTreeSet::from([selector]))
    }
}
```

Export it from `src/lib.rs`:

```rust
pub mod job;
```

- [ ] **Step 4: Run the focused test**

Run:

```bash
cargo test --test job
```

Expected: 2 tests pass.

- [ ] **Step 5: Run formatting and commit**

Run:

```bash
cargo fmt
cargo fmt --check
git add src/job.rs src/lib.rs tests/job.rs
git commit -m "feat: add typed job identity"
```

### Task 2: Make ExecutionPlan one ordered typed job collection

**Files:**
- Modify: `src/plan.rs`
- Modify: `src/provider.rs`
- Modify: `src/dry_run.rs`
- Modify: `src/app/apply.rs`
- Modify: `src/link.rs`
- Modify: `tests/dry_run.rs`
- Modify: `tests/link.rs`
- Modify: `tests/provider.rs`
- Modify: `tests/provider_installs.rs`

- [ ] **Step 1: Add a failing ordered-job test**

In `tests/dry_run.rs`, plan `dry-run/valid-human-readable-plan.toml` and assert the single job sequence:

```rust
#[test]
fn execution_plan_exposes_one_ordered_typed_job_sequence() {
    let manifest = select_fixture("dry-run/valid-human-readable-plan.toml");
    let environment = environment();
    let xdg = XdgPaths::detect();
    let platform = platform();
    let plan = ExecutionPlanner::new(&environment, dot_paths(), &xdg, &platform)
        .plan(&manifest)
        .expect("execution should plan");

    let ids = plan
        .jobs()
        .iter()
        .map(|job| (job.id().kind(), job.id().name().to_owned()))
        .collect::<Vec<_>>();

    assert_eq!(
        ids,
        [
            (JobKind::Provider, "system".into()),
            (JobKind::Package, "alpha".into()),
            (JobKind::Package, "manual".into()),
            (JobKind::Action, "configure".into()),
            (JobKind::Link, "gitconfig".into()),
        ]
    );
}
```

Add the required `dot::job::JobKind` import.

- [ ] **Step 2: Run the focused test and verify the missing API**

Run:

```bash
cargo test --test dry_run execution_plan_exposes_one_ordered_typed_job_sequence
```

Expected: compilation fails because `ExecutionPlan::jobs` and `PlannedJob` do not exist.

- [ ] **Step 3: Introduce the closed planned-job variants**

In `src/plan.rs`, replace the phase-specific storage fields with:

```rust
pub struct ExecutionPlan {
    target: String,
    profile: Option<String>,
    platform: PlatformInfo,
    jobs: Vec<PlannedJob>,
}

#[derive(Debug)]
pub enum PlannedJob {
    Provider(PlannedProvider),
    Package(PlannedPackage),
    Action(PlannedAction),
    Link(PlannedLink),
}

#[derive(Debug)]
pub enum PlannedPackage {
    Provider(PlannedProviderInstall),
    Manual(PlannedManualPackage),
}
```

Change planned IDs and provider references from `String` to cloned
`schema::Identifier` values. Preserve the existing `id() -> &str` and
`provider() -> &str` accessors for report code, and add internal typed
accessors:

```rust
impl PlannedProvider {
    pub(crate) fn job_id(&self) -> JobId {
        JobId::Provider(self.id.clone())
    }
}

impl PlannedJob {
    pub fn id(&self) -> JobId {
        match self {
            Self::Provider(provider) => provider.job_id(),
            Self::Package(package) => package.job_id(),
            Self::Action(action) => action.job_id(),
            Self::Link(link) => link.job_id(),
        }
    }
}
```

Construct `jobs` in this exact order inside `ExecutionPlanner::plan`:

```rust
let mut jobs = Vec::new();
jobs.extend(providers.into_iter().map(PlannedJob::Provider));
jobs.extend(
    provider_installs
        .into_iter()
        .map(PlannedPackage::Provider)
        .map(PlannedJob::Package),
);
jobs.extend(
    manual_packages
        .into_iter()
        .map(PlannedPackage::Manual)
        .map(PlannedJob::Package),
);
jobs.extend(actions.into_iter().map(PlannedJob::Action));
jobs.extend(links.into_iter().map(PlannedJob::Link));
```

Expose `jobs() -> &[PlannedJob]` and typed iterator projections such as:

```rust
pub fn providers(&self) -> impl Iterator<Item = &PlannedProvider> {
    self.jobs.iter().filter_map(|job| match job {
        PlannedJob::Provider(provider) => Some(provider),
        _ => None,
    })
}
```

Add equivalent iterators for provider installs, manual packages, actions, and
links. They are compatibility projections over `jobs`, not additional stored
facts.

- [ ] **Step 4: Adapt existing aggregate runners and link reconciliation to iterators**

Make `ProviderRunner::ensure_all` and `install_all` accept iterators of borrowed
planned values:

```rust
pub fn ensure_all<'p>(
    &self,
    providers: impl IntoIterator<Item = &'p PlannedProvider>,
) -> ProviderReadiness;

pub fn install_all<'p>(
    &self,
    installs: impl IntoIterator<Item = &'p PlannedProviderInstall>,
    readiness: &ProviderReadiness,
) -> ProviderInstallExecution;
```

Make `link::reconcile` collect an iterator of borrowed links internally:

```rust
pub fn reconcile<'a>(
    links: impl IntoIterator<Item = &'a PlannedLink>,
) -> Result<LinkReport, LinkPhaseError> {
    let links = links.into_iter().collect::<Vec<_>>();
    // Preserve the current preflight and serial reconciliation behavior.
}
```

Update apply, dry-run, and tests to consume the new iterator projections.
Remove `.iter()` calls that assumed stored phase vectors, and replace indexing
in tests with local `Vec<_>` collections only where an assertion needs random
access.

- [ ] **Step 5: Make dry-run visit PlannedJob directly**

Replace the five independent `items.extend(...)` blocks in `src/dry_run.rs`
with one `plan.jobs().iter().map(...)` match:

```rust
let items = plan
    .jobs()
    .iter()
    .map(|job| match job {
        PlannedJob::Provider(provider) => provider_item(provider),
        PlannedJob::Package(PlannedPackage::Provider(package)) => {
            provider_package_item(package)
        }
        PlannedJob::Package(PlannedPackage::Manual(package)) => {
            manual_package_item(package)
        }
        PlannedJob::Action(action) => action_item(action),
        PlannedJob::Link(link) => link_item(link),
    })
    .collect();
```

Extract only small private item-construction helpers; do not introduce another
report plan.

- [ ] **Step 6: Run the affected tests**

Run:

```bash
cargo test --test dry_run --test dry_run_command
cargo test --test provider --test provider_installs
cargo test --test link
cargo test --test apply_command
```

Expected: all tests pass and the existing report/order assertions remain
unchanged.

- [ ] **Step 7: Commit the unified plan storage**

Run:

```bash
cargo fmt
cargo fmt --check
git add src/plan.rs src/provider.rs src/dry_run.rs src/app/apply.rs src/link.rs \
  tests/dry_run.rs tests/link.rs tests/provider.rs tests/provider_installs.rs
git commit -m "refactor: unify planned jobs"
```

### Task 3: Add selected-plan views and minimum provider closure

**Files:**
- Modify: `src/job.rs`
- Modify: `src/plan.rs`
- Modify: `src/dry_run.rs`
- Modify: `src/app/dry_run.rs`
- Modify: `tests/job.rs`

- [ ] **Step 1: Add failing selection tests**

Extend `tests/job.rs` with a local planner helper using
`dry-run/valid-human-readable-plan.toml`, then add:

```rust
#[test]
fn selecting_provider_package_adds_only_its_provider() {
    let plan = plan_fixture();
    let selected = plan
        .select(&JobSelection::only(JobSelector::Package(id("alpha"))))
        .expect("package should select");

    assert_eq!(
        selected.jobs().map(PlannedJob::id).collect::<Vec<_>>(),
        [
            JobId::Provider(id("system")),
            JobId::Package(id("alpha")),
        ]
    );
}

#[test]
fn selecting_manual_action_or_link_adds_no_provider() {
    let plan = plan_fixture();

    for selector in [
        JobSelector::Package(id("manual")),
        JobSelector::Action(id("configure")),
        JobSelector::Link(id("gitconfig")),
    ] {
        let selected = plan
            .select(&JobSelection::only(selector))
            .expect("job should select");
        assert_eq!(selected.jobs().count(), 1);
    }
}

#[test]
fn unknown_typed_selector_fails_before_execution() {
    let plan = plan_fixture();
    let error = plan
        .select(&JobSelection::only(JobSelector::Action(id("missing"))))
        .expect_err("unknown action should fail");

    assert!(matches!(
        error,
        JobSelectionError::Unknown(JobSelector::Action(ref id))
            if id.as_str() == "missing"
    ));
}
```

- [ ] **Step 2: Run the focused tests and verify the missing selection API**

Run:

```bash
cargo test --test job
```

Expected: compilation fails because `ExecutionPlan::select`,
`SelectedExecutionPlan`, and `JobSelectionError` do not exist.

- [ ] **Step 3: Implement selection without a general dependency graph**

In `src/plan.rs`, add:

```rust
pub struct SelectedExecutionPlan<'a> {
    source: &'a ExecutionPlan,
    selected: BTreeSet<JobId>,
}

impl SelectedExecutionPlan<'_> {
    pub fn source(&self) -> &ExecutionPlan {
        self.source
    }

    pub fn jobs(&self) -> impl Iterator<Item = &PlannedJob> {
        self.source
            .jobs()
            .iter()
            .filter(|job| self.selected.contains(&job.id()))
    }
}
```

Implement `ExecutionPlan::select` with explicit closed rules:

```rust
match selection {
    JobSelection::All => select every planned JobId,
    JobSelection::Only(selectors) => {
        for selector in selectors {
            find the matching package/action/link;
            insert its JobId;
            if it is a provider-backed package, insert exactly its provider JobId;
        }
    }
}
```

Do not add recursive graph traversal or cycle detection. The schema model permits
only the single package-to-provider requirement.

No duplicate-job validation pass is needed. `EffectiveManifest` stores each
domain as a keyed map, profile merging replaces by key, planning emits one job
per effective entry, and `JobId` scopes the key by domain. This makes duplicate
typed IDs structurally unrepresentable through configuration.

Add:

```rust
#[derive(Debug)]
pub enum JobSelectionError {
    Unknown(JobSelector),
    MissingProvider {
        package: Identifier,
        provider: Identifier,
    },
}
```

Planning should already reject the missing-provider case, but selection keeps
the invariant explicit rather than indexing blindly.

- [ ] **Step 4: Route dry-run through the all-job selected view**

Change `dry_run::build_report` to accept `&SelectedExecutionPlan<'_>`. Get
context from `selected.source()` and report items from `selected.jobs()`.

In `src/app/dry_run.rs`:

```rust
let selected = plan.select(&JobSelection::All)?;
Ok(build_report(loaded.path(), &selected))
```

Add `JobSelectionError` to the command error enum and source chain.

- [ ] **Step 5: Run selection and dry-run tests**

Run:

```bash
cargo test --test job
cargo test --test dry_run --test dry_run_command
```

Expected: all tests pass; existing dry-run output is byte-for-byte equivalent
where current assertions cover it.

- [ ] **Step 6: Commit the selection layer**

Run:

```bash
cargo fmt
cargo fmt --check
git add src/job.rs src/plan.rs src/dry_run.rs src/app/dry_run.rs tests/job.rs
git commit -m "feat: select planned jobs"
```

### Task 4: Expose one-provider and one-install execution operations

**Files:**
- Modify: `src/provider.rs`
- Modify: `tests/provider.rs`
- Modify: `tests/provider_installs.rs`

- [ ] **Step 1: Add focused single-operation tests**

Add tests that plan one provider and one provider install, then call the new
single-item API:

```rust
#[test]
fn ensure_returns_one_typed_provider_status() {
    let provider = planned_provider_fixture();
    let runner = ProviderRunner::new(&ExecutionEnvironment::empty());

    let status = runner.ensure(&provider);

    assert_eq!(status.id(), provider.id());
}

#[test]
fn install_uses_the_environment_from_one_ready_provider_status() {
    let (provider, install) = planned_provider_and_install_fixture();
    let runner = ProviderRunner::new(&ExecutionEnvironment::empty());
    let readiness = runner.ensure(&provider);

    let status = runner.install(&install, &readiness);

    assert_eq!(status.id(), install.id());
    assert!(status.is_succeeded());
}
```

Reuse the existing provider test fixtures/helpers rather than adding inline
TOML.

- [ ] **Step 2: Run focused tests and verify the APIs are private/missing**

Run:

```bash
cargo test --test provider ensure_returns_one_typed_provider_status
cargo test --test provider_installs install_uses_the_environment_from_one_ready_provider_status
```

Expected: compilation fails because `ProviderRunner::ensure` and `install` do
not exist.

- [ ] **Step 3: Add public single-item methods and delegate aggregate methods**

Refactor `ProviderRunner`:

```rust
pub fn ensure(&self, provider: &PlannedProvider) -> ProviderStatus {
    let (environment, outcome) = match self.ensure_one(provider) {
        Ok((environment, outcome)) => (Some(environment), Ok(outcome)),
        Err(error) => (None, Err(error)),
    };
    ProviderStatus {
        id: provider.id().to_owned(),
        environment,
        outcome,
    }
}

pub fn install(
    &self,
    install: &PlannedProviderInstall,
    provider: &ProviderStatus,
) -> ProviderInstallStatus {
    let outcome = match provider.environment() {
        Some(environment) => self.install_one(install, environment),
        None => Ok(ProviderInstallOutcome::NotRunProviderUnavailable),
    };
    ProviderInstallStatus {
        id: install.id().to_owned(),
        outcome,
    }
}
```

Make `ProviderStatus` and `ProviderInstallStatus` constructors remain internal.
Have `ensure_all` call `ensure` and `install_all` look up the provider status
then call `install`. Preserve all existing public outcomes and tests.

- [ ] **Step 4: Run all provider tests**

Run:

```bash
cargo test --test provider --test provider_installs
```

Expected: all existing and new tests pass.

- [ ] **Step 5: Commit**

Run:

```bash
cargo fmt
cargo fmt --check
git add src/provider.rs tests/provider.rs tests/provider_installs.rs
git commit -m "refactor: expose single provider operations"
```

### Task 5: Implement the serial JobExecutor and JobRunner

**Files:**
- Create: `src/job_executor.rs`
- Create: `src/job_runner.rs`
- Modify: `src/job.rs`
- Modify: `src/link.rs`
- Modify: `src/lib.rs`
- Create: `tests/job_runner.rs`
- Create: `tests/fixtures/jobs/valid-serial-execution-template.toml`

- [ ] **Step 1: Add the complete executable fixture**

Create `tests/fixtures/jobs/valid-serial-execution-template.toml`:

```toml
[targets.current]
platform = { os = "__OS__" }

[targets.current.providers.ready]
activate = { variables = { DOT_JOB_PROVIDER_ACTIVE = "yes" } }
probe = __PROBE__
install = __INSTALL__

[targets.current.packages.provider-tool]
provider = "ready"

[targets.current.packages.manual-tool.install]
exec = __MANUAL__

[targets.current.actions.configure]
exec = __ACTION__

[targets.current.links.config]
source = "source.txt"
target = "${dot:config_dir}/linked.txt"
```

Use the same temporary-workspace and current-test-executable helper pattern as
`tests/apply_command.rs`. Each helper appends one event to
`${dot:config_dir}/events`.

- [ ] **Step 2: Write failing runner tests**

Create `tests/job_runner.rs` with:

```rust
#[test]
fn runs_selected_jobs_in_stable_serial_order() {
    let workspace = TestWorkspace::from_fixture(
        "jobs/valid-serial-execution-template.toml",
    );
    let plan = workspace.plan();
    let selected = plan
        .select(&JobSelection::All)
        .expect("all jobs should select");

    let report = JobRunner::new(workspace.environment()).run(&selected);

    assert!(report.all_succeeded());
    assert_eq!(
        workspace.recorded_events(),
        ["probe", "provider-install", "manual-install", "action"]
    );
    assert_eq!(
        selected.jobs().map(PlannedJob::id).collect::<Vec<_>>(),
        [
            JobId::Provider(id("ready")),
            JobId::Package(id("provider-tool")),
            JobId::Package(id("manual-tool")),
            JobId::Action(id("configure")),
            JobId::Link(id("config")),
        ]
    );
}

#[test]
fn exact_provider_package_runs_only_its_provider_requirement_and_itself() {
    let workspace = TestWorkspace::from_fixture(
        "jobs/valid-serial-execution-template.toml",
    );
    let plan = workspace.plan();
    let selected = plan
        .select(&JobSelection::only(JobSelector::Package(id(
            "provider-tool",
        ))))
        .expect("package should select");

    let report = JobRunner::new(workspace.environment()).run(&selected);

    assert!(report.all_succeeded());
    assert_eq!(workspace.recorded_events(), ["probe", "provider-install"]);
    assert_eq!(report.len(), 2);
    assert!(!workspace.path("linked.txt").exists());
}
```

Add a third test using a failing provider probe and a succeeding manual
package/action to assert:

```text
provider FAILED
provider package BLOCKED
manual package EXECUTED
action EXECUTED
link still reconciled
```

- [ ] **Step 3: Run the new test and verify the runner is missing**

Run:

```bash
cargo test --test job_runner
```

Expected: compilation fails because `job_executor` and `job_runner` do not
exist.

- [ ] **Step 4: Add typed execution states and outcomes**

In `src/job.rs`, add:

```rust
#[derive(Debug)]
pub enum BlockReason {
    ProviderUnavailable { provider: Identifier },
    LinkPhase { message: String },
}

#[derive(Debug)]
pub enum JobOutcome {
    Provider(ProviderStatus),
    ProviderPackage(ProviderInstallStatus),
    ManualPackage(Result<ActionOutcome, ActionRunError>),
    Action(Result<ActionOutcome, ActionRunError>),
    Link(Result<LinkOutcome, LinkError>),
}

#[derive(Debug)]
pub enum JobState {
    Completed(JobOutcome),
    Blocked(BlockReason),
}
```

If these domain types make `job.rs` depend on too many execution modules, put
`JobOutcome` and `JobState` in `job_runner.rs`; keep only identity and selection
vocabulary in `job.rs`. Do not introduce trait objects merely to avoid a normal
closed-module dependency.

- [ ] **Step 5: Implement the closed JobExecutor**

Create `src/job_executor.rs` with methods that delegate only:

```rust
pub struct JobExecutor<'a> {
    provider: ProviderRunner<'a>,
    action: ActionRunner<'a>,
}

impl JobExecutor<'_> {
    pub fn ensure_provider(&self, job: &PlannedProvider) -> ProviderStatus;

    pub fn install_provider_package(
        &self,
        job: &PlannedProviderInstall,
        provider: &ProviderStatus,
    ) -> ProviderInstallStatus;

    pub fn install_manual_package(
        &self,
        job: &PlannedManualPackage,
    ) -> Result<ActionOutcome, ActionRunError>;

    pub fn run_action(
        &self,
        job: &PlannedAction,
    ) -> Result<ActionOutcome, ActionRunError>;

    pub fn reconcile_links<'p>(
        &self,
        jobs: impl IntoIterator<Item = &'p PlannedLink>,
    ) -> Result<LinkReport, LinkPhaseError>;
}
```

Do not duplicate provider/action/link lifecycle logic in this file.

- [ ] **Step 6: Add ownership accessors for link results**

In `src/link.rs`, add narrow consumption APIs so `JobRunner` can move each
existing typed link outcome into its result map without cloning errors or
repeating link logic:

```rust
impl LinkResult {
    pub(crate) fn into_parts(self) -> (String, Result<LinkOutcome, LinkError>);
}

impl LinkReport {
    pub(crate) fn into_results(self) -> impl Iterator<Item = LinkResult>;
}
```

Keep `reconcile` as the owner of duplicate-target preflight and link mutation.

- [ ] **Step 7: Implement one strictly serial traversal**

Create `src/job_runner.rs`. This is the single dispatch point for
`PlannedJob`; the core flow should be:

```rust
let jobs = selected.jobs().collect::<Vec<_>>();
let mut links = Vec::new();

for job in jobs {
    match job {
        provider:
            execute and store ProviderStatus by typed JobId
        provider-backed package:
            look up its provider result;
            if ready, execute install;
            otherwise store Blocked::ProviderUnavailable
        manual package:
            execute ActionRunner lifecycle
        action:
            execute ActionRunner lifecycle
        link:
            collect its reference for the phase-wide preflight
    }
}

run existing link::reconcile once so duplicate-target preflight remains
phase-wide and happens before link mutation;
project LinkReport results, or mark every selected link blocked and retain the
LinkPhaseError diagnostic.
```

This is one pass over the ordered selected facts; links are deferred only
because their existing safety rule validates the complete selected link set
before any link mutation. The runner must not stop on an ordinary failure.
Store results in a
`BTreeMap<JobId, JobState>` for lookup. The typed map key is the sole stored job
identity:

```rust
pub struct JobExecutionReport {
    results: BTreeMap<JobId, JobState>,
    link_phase_error: Option<LinkPhaseError>,
}

impl JobExecutionReport {
    pub fn get(&self, id: &JobId) -> Option<&JobState>;
    pub fn all_succeeded(&self) -> bool;
}
```

Report projection obtains stable order from `SelectedExecutionPlan::jobs()` and
looks each job up through `JobExecutionReport::get`; the result map must not
store a second order or duplicate the ID inside its value.

Avoid `Pending`, `Running`, cancellation, threads, and channels: they do not
exist in a synchronous serial runner.

- [ ] **Step 8: Run runner and lower-level tests**

Run:

```bash
cargo test --test job_runner
cargo test --test provider --test provider_installs --test action_runner --test link
```

Expected: all tests pass.

- [ ] **Step 9: Commit**

Run:

```bash
cargo fmt
cargo fmt --check
git add src/job.rs src/job_executor.rs src/job_runner.rs src/link.rs src/lib.rs \
  tests/job_runner.rs tests/fixtures/jobs/valid-serial-execution-template.toml
git commit -m "feat: execute selected jobs serially"
```

### Task 6: Make apply a projection of JobRunner results

**Files:**
- Modify: `src/app/apply.rs`
- Modify: `tests/apply_command.rs`
- Modify: `tests/report_schema.rs`

- [ ] **Step 1: Run the existing apply characterization tests**

Run:

```bash
cargo test --test apply_command
```

Expected: current tests pass before the facade refactor. Save the observed event
order and table assertions as the compatibility baseline.

- [ ] **Step 2: Add one report-schema assertion for typed job projection**

Add a focused test that constructs the complete apply report and verifies its
subject order remains:

```text
Provider, Package, Package, Action, Link
```

Use `ReportSubject` matches rather than formatted table text.

- [ ] **Step 3: Replace app-specific orchestration**

In `src/app/apply.rs`, replace `execute`, `ApplyResult`, and
`NamedActionResult` with:

```rust
let selected = plan.select(&JobSelection::All)?;
let execution = JobRunner::new(loaded.environment()).run(&selected);
Ok(build_report(loaded.path(), &selected, &execution))
```

Add `JobSelectionError` to `CommandError`.

Rewrite `build_report` to visit `selected.jobs()` once and match each
`PlannedJob` with its `JobState`. Reuse the existing evidence helpers and
status mapping; do not alter human-visible statuses or messages.

For a phase-level `LinkPhaseError`, preserve current behavior:

- every selected link is `BLOCKED`;
- every link has phase-error evidence;
- one command-level error diagnostic is emitted;
- no selected link is mutated.

Delete the obsolete app-specific aggregate result structs only after the report
tests pass.

- [ ] **Step 4: Run apply and output tests**

Run:

```bash
cargo test --test apply_command
cargo test --test report_schema --test output_table
```

Expected: all tests pass with unchanged command status, item status, evidence,
aggregate counts, and serial event order.

- [ ] **Step 5: Run dry-run and provider-check regressions**

Run:

```bash
cargo test --test dry_run --test dry_run_command
cargo test --test check --test check_command
```

Expected: all pass. In particular,
`check_providers_ignores_expression_errors_outside_activate_and_probe` must
remain green, proving provider check did not get routed through the complete
apply plan.

- [ ] **Step 6: Commit**

Run:

```bash
cargo fmt
cargo fmt --check
git add src/app/apply.rs tests/apply_command.rs tests/report_schema.rs
git commit -m "refactor: apply selected jobs through runner"
```

### Task 7: Update canonical design and perform full verification

**Files:**
- Modify: `docs/DESIGN.txt`
- Modify: `docs/JOB_EXECUTION.md` only if implementation names differ from the approved design

- [ ] **Step 1: Update the canonical execution description**

In `docs/DESIGN.txt`, update the execution-plan and apply-pipeline sections to
state:

```text
ExecutionPlan owns one ordered sequence of typed Provider, Package, Action, and
Link jobs. Dry-run visits an all-job selected view. Apply runs that same view
through the strictly serial JobRunner. A future exact selection may choose one
package, action, or link; a provider-backed package adds only its provider.
Provider check remains a separate probe-only diagnostic path.
```

Add parallel execution, lanes, job dependency DSLs, and process supervision to
the explicit non-goals. Do not document future CLI syntax.

- [ ] **Step 2: Run formatting**

Run:

```bash
cargo fmt
cargo fmt --check
```

Expected: exit 0.

- [ ] **Step 3: Run strict linting**

Run:

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: exit 0 with no warnings.

- [ ] **Step 4: Run both feature test matrices**

Run:

```bash
cargo test
cargo test --all-features
```

Expected: every unit, integration, and doc test passes in both configurations.

- [ ] **Step 5: Verify repository hygiene**

Run:

```bash
git diff --check
git status --short
```

Expected: no whitespace errors; only the intended documentation change remains
before the final commit.

- [ ] **Step 6: Commit documentation**

Run:

```bash
git add docs/DESIGN.txt docs/JOB_EXECUTION.md
git commit -m "docs: document serial job execution"
```

- [ ] **Step 7: Inspect final history and changed-file scope**

Before implementation begins, record its base commit:

```bash
IMPLEMENTATION_BASE="$(git rev-parse HEAD)"
```

After the final documentation commit, run:

```bash
git log --oneline --decorate -8
git diff --stat "${IMPLEMENTATION_BASE}"..HEAD
```

Expected: small, focused commits for identity, unified planning, selection,
provider entry points, serial execution, apply integration, and canonical docs;
no CLI/schema/process-supervision changes.

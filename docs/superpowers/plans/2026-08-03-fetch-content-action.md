# Fetch Content Action Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a deliberately small Fetch Content Action that materializes one public HTTPS response at one local target, while removing the unused `ExecAction.type` field and preserving the existing Action selection, phase, failure-isolation, and reporting contracts.

**Architecture:** Represent source Actions as an untagged structural sum of Command and Fetch Content records. Planning resolves a selected fetch into the one supported typed locator pair (`Url` to `PathBuf`); execution delegates it to a private fetch module with a narrow injectable transport boundary, target preflight, same-directory staging, and explicit commit. Existing command execution remains in its own runner, and the job/report layers dispatch on the planned Action variant.

**Tech Stack:** Rust 2024, Serde/TOML, `url` 2.5 for locator parsing, `ureq` 3.3 with only `rustls` enabled for blocking HTTPS, `tempfile` 3.27 for target-adjacent staging, existing typed plan/job/report pipeline, Cargo tests/Clippy/rustfmt.

---

## Preconditions and fixed decisions

- Work in `/Users/ysl/Code/dot/.worktrees/fetch-content-action` on branch `feature/fetch-content-action`.
- Treat `docs/superpowers/specs/2026-08-03-fetch-content-action-design.md` as authoritative.
- Do not edit `docs/archive/v0.1.0/`.
- Do not add checksum, caching, conditional requests, retries, auth, headers, extraction, permissions, receipts, resource references, or duplicate-target detection.
- Reject URL userinfo during planning: it is inline authentication and conflicts with the approved public-HTTPS-only boundary.
- Build `ureq` without its default `gzip` feature. Fetch Content writes the response bytes supplied by the HTTP layer and does not opt into transparent content decoding.
- Configure `ureq` with `https_only(true)` and an internal redirect limit of five. The limit is not schema.
- Keep the production transport private to the crate. Test doubles live in `src/fetch_content.rs` unit tests; this is not a public plugin API.
- Use `symlink_metadata` for all target classification. Never follow an existing target symlink.

## File map

| Concern | Existing files | New files |
| --- | --- | --- |
| Source schema and structural validation | `src/schema.rs`, `src/validation.rs`, `src/interpolation.rs`, `tests/schema.rs`, `tests/validation.rs`, `tests/fixtures/schema/*` | schema fixtures listed below |
| Selected runtime planning | `src/plan.rs`, `tests/job.rs`, `tests/dry_run.rs`, `tests/check.rs` | `tests/fixtures/dry-run/valid-fetch-content-template.toml`, invalid planning fixtures as needed |
| Structural listing and dry-run presentation | `src/app/list_jobs.rs`, `src/report.rs`, `src/dry_run.rs`, `src/output/table.rs`, `tests/list_command.rs`, `tests/report_schema.rs`, `tests/output_table.rs`, `tests/dry_run.rs` | none |
| Fetch transfer and filesystem reconciliation | `src/lib.rs`, `Cargo.toml`, `Cargo.lock` | `src/fetch_content.rs` with private unit tests |
| Action dispatch and apply reporting | `src/action_runner.rs`, `src/job_executor.rs`, `src/job_runner.rs`, `src/app/apply.rs`, `tests/action_runner.rs`, `tests/job_runner.rs`, `tests/apply_command.rs`, `tests/error_contract.rs` | none |
| User/runtime documentation | `docs/SCHEMA.txt`, `docs/DESIGN.txt`, `docs/CONFIGURATION.md`, `README.md` | none |

## Task 1: Remove the inert `ExecAction.type` field

**Files:**

- Modify: `tests/schema.rs`
- Create: `tests/fixtures/schema/invalid-legacy-exec-type.toml`
- Modify: `tests/fixtures/schema/valid-complete.toml`
- Modify: `src/schema.rs`
- Modify: `src/interpolation.rs`
- Modify command-value constructors in: `tests/action.rs`, `tests/action_runner.rs`, `tests/check.rs`, `tests/provider.rs`, `tests/provider_installs.rs`, `tests/report_schema.rs`

- [ ] **Step 1: Add a failing legacy-field rejection test**

Add a minimal valid target fixture whose provider command contains the legacy field:

```toml
[targets.test.platform]
os = "linux"

[targets.test.providers.tool]
probe = { type = "exec", program = "tool", args = ["--version"] }
install = { program = "tool", args = ["install", "${package:names}"] }
```

Add to `tests/schema.rs`:

```rust
#[test]
fn rejects_legacy_exec_action_type() {
    let input = fixture::read("schema/invalid-legacy-exec-type.toml");
    let error = toml::from_str::<Config>(&input).expect_err("legacy type must be unknown");
    assert!(error.to_string().contains("unknown field `type`"), "{error}");
}
```

- [ ] **Step 2: Run the focused test and confirm it fails for the intended reason**

Run: `cargo test --test schema rejects_legacy_exec_action_type -- --exact`

Expected: FAIL because `type = "exec"` is still accepted.

- [ ] **Step 3: Remove the field and schema enum**

Change `ExecAction` in `src/schema.rs` to:

```rust
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(
    deny_unknown_fields,
    bound(deserialize = "S: Deserialize<'de>, A: Deserialize<'de>")
)]
pub struct ExecAction<S = StringExpressionSource, A = S> {
    pub program: S,
    #[serde(default)]
    pub args: Vec<A>,
    pub cwd: Option<S>,
    pub env: Option<EnvironmentPatch<S>>,
}
```

Delete `ExecActionType`. Remove `kind` copying from both exec-action resolution functions in `src/interpolation.rs`, and remove `kind: None`/`kind: Some(...)` from every command constructor and assertion. Remove both explicit `type = "exec"` values from `tests/fixtures/schema/valid-complete.toml`.

- [ ] **Step 4: Run the focused schema and command suites**

Run:

```bash
cargo test --test schema --test action --test action_runner --test check --test provider --test provider_installs --test report_schema
```

Expected: PASS; the new invalid fixture is rejected and ordinary command actions still deserialize and execute.

- [ ] **Step 5: Confirm no active code or fixture still refers to the deleted extension point**

Run:

```bash
rg -n "ExecActionType|kind: (None|Some)|type = ['\"]exec['\"]" src tests --glob '!docs/archive/**'
```

Expected: no matches.

- [ ] **Step 6: Commit the schema cleanup**

```bash
git add src/schema.rs src/interpolation.rs tests
git commit -m "refactor: remove exec action type field"
```

## Task 2: Model Command and Fetch Content as structural Action variants

**Files:**

- Modify: `docs/SCHEMA.txt`
- Modify: `src/schema.rs`
- Modify: `src/validation.rs`
- Modify: `src/manifest.rs`
- Modify: `src/plan.rs`
- Modify: `src/report.rs`
- Modify: `src/dry_run.rs`
- Modify: `src/action_runner.rs`
- Modify: `src/job_executor.rs`
- Modify: `src/app/apply.rs`
- Modify: `src/app/list_jobs.rs`
- Modify: `tests/schema.rs`
- Modify: `tests/validation.rs`
- Modify: `tests/list_command.rs`
- Modify: `tests/fixtures/list/valid-catalog.toml`
- Modify: `tests/fixtures/schema/valid-complete.toml`
- Create: `tests/fixtures/schema/invalid-mixed-action.toml`
- Create: `tests/fixtures/schema/invalid-incomplete-fetch-action.toml`
- Create: `tests/fixtures/schema/invalid-fetch-action-unknown-field.toml`
- Create: `tests/fixtures/schema/invalid-fetch-conflict.toml`
- Create: `tests/fixtures/validation/invalid-unselected-fetch-expression.toml`

- [ ] **Step 1: Add failing structural-deserialization tests**

Extend the complete schema fixture with both shapes:

```toml
[targets.workstation.actions.setup]
exec = { program = "touch", args = ["/tmp/ready"] }

[targets.workstation.actions.remote-config]
source = "https://example.com/config.toml"
target = "configs/app.toml"
on_conflict = "replace"
```

Add assertions that the first is `Action::Command` and the second is `Action::FetchContent`. Add fixture-backed tests proving that all of these fail TOML deserialization:

```toml
# mixed
source = "https://example.com/a"
target = "a"
exec = { program = "true" }

# incomplete
source = "https://example.com/a"

# unknown
source = "https://example.com/a"
target = "a"
checksum = "deadbeef"
```

Also add a fixture/test with `on_conflict = "overwrite"` and assert the fixed
literal is rejected. Add a test that a manual package `install` containing
`source`/`target` is rejected.

Extend the list catalog with one Fetch Content Action and assert its stable TSV
row preserves unresolved spelling:

```text
action:remote-config\taction\tremote-config\tfetch\thttps://example.com/config.toml -> configs/app.toml
```

- [ ] **Step 2: Run schema tests and observe the missing variant failures**

Run: `cargo test --test schema`

Expected: FAIL because `Action` is still a command-only record.

- [ ] **Step 3: Update the normative schema first, then implement the Rust sum**

Update the current `docs/SCHEMA.txt` Action definitions before changing Rust. Then use these core Rust types in `src/schema.rs`:

```rust
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum Action<S = StringExpressionSource, A = S> {
    Command(CommandAction<S, A>),
    FetchContent(FetchContentAction<S>),
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(
    deny_unknown_fields,
    bound(deserialize = "S: Deserialize<'de>, A: Deserialize<'de>")
)]
pub struct CommandAction<S = StringExpressionSource, A = S> {
    pub check: Option<ExecAction<S, A>>,
    pub exec: ExecAction<S, A>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, bound(deserialize = "S: Deserialize<'de>"))]
pub struct FetchContentAction<S = StringExpressionSource> {
    pub source: S,
    pub target: S,
    pub on_conflict: Option<FetchContentConflict>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FetchContentConflict {
    #[default]
    Error,
    Replace,
}

pub type SourceAction = Action<StringExpressionSource, StringExpressionSource>;
pub type SourceCommandAction = CommandAction<StringExpressionSource, StringExpressionSource>;
pub type ResolvedCommandAction = CommandAction<ResolvedString, ResolvedString>;
```

Change `ManualPackage.install` from `Action` to `CommandAction`; this statically prevents Fetch Content installs. Do not add a resolved Fetch Content schema alias yet—the planner will own its typed URL/path representation.

- [ ] **Step 4: Keep every command-only consumer compiling**

This task changes a central source type, so update every consumer before the
PASS/commit checkpoint:

- `PlannedManualPackage.install` and the temporarily command-only
  `PlannedAction.action` use `ResolvedCommandAction`;
- `ActionRunner::run` accepts `&ResolvedCommandAction`;
- `ActionInfo` command constructors accept `&SourceCommandAction` or
  `&ResolvedCommandAction`;
- dry-run, apply, and manual-package paths continue to project that command
  type;
- `src/app/list_jobs.rs` exhaustively matches both source variants and already
  emits `via=fetch` plus unresolved `source -> target` for Fetch Content;
- `plan_actions` resolves `Action::Command` normally and returns a temporary
  typed `PlanningError::FetchContentNotYetWired { action }` for a selected
  `Action::FetchContent`.

The temporary planning error prevents a panic or silent command coercion and
is deleted in Task 6. Do not document it as product behavior. It exists only
to give this schema/validation/list slice a compiling checkpoint before the
typed fetch executor is wired.

- [ ] **Step 5: Add exhaustive static validation**

Replace command-only `validate_action` with an enum match:

```rust
fn validate_action(
    action: &SourceAction,
    context: &ValidationContext,
) -> Result<(), ConfigValidationError> {
    match action {
        Action::Command(action) => validate_command_action(action, context, ""),
        Action::FetchContent(action) => {
            promote_string_expression(&action.source)
                .map_err(|source| context.expression("source", source))?;
            promote_string_expression(&action.target)
                .map_err(|source| context.expression("target", source))?;
            Ok(())
        }
    }
}
```

Keep a separate `validate_command_action(&SourceCommandAction, ..., prefix)` for manual installs. Add an unselected/replaced Fetch Content fixture with a malformed/unknown resolver and assert `validate_config` still reports `source` or `target` with the correct target/profile/action context.

- [ ] **Step 6: Run schema, validation, list, and command regressions**

Run:

```bash
cargo test --test schema --test validation --test manifest --test list_command --test action_runner --test dry_run
```

Expected: PASS, including profile merge/replacement regressions, unresolved
Fetch list output, and unchanged Command execution/report behavior. Selected
Fetch planning remains the explicit temporary error until Task 6.

- [ ] **Step 7: Commit the structural Action model**

```bash
git add docs/SCHEMA.txt src tests
git commit -m "feat: add fetch content action schema"
```

## Task 3: Build and test typed locator resolution without wiring execution

**Files:**

- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `src/plan.rs`
- Add unit tests in: `src/plan.rs`

- [ ] **Step 1: Add failing unit tests for locator resolution policy**

Add `src/plan.rs` unit tests around one pure resolver that takes a source
`FetchContentAction`, `ResolveContext`, Action ID, and entry directory. Cover:

- valid public absolute HTTPS source;
- omitted `on_conflict` becoming `FetchContentConflict::Error` and explicit
  `replace` remaining `Replace`;
- absolute target unchanged;
- `configs/app.toml` joined to the configuration entry directory;
- `${dot:real_config_dir}/configs/app.toml` resolving to the entity directory;
- `http://`, relative URL, malformed URL, URL userinfo, and an absolute-URL
  target rejected with `action:ID` plus the exact field;
- `${env:...}`, `${dot:...}`, and `${xdg:...}` resolution errors preserving
  normal interpolation sources.

Use distinct entry and real directories in `DotPaths` to prove the base choice.
Selected-only integration, atomic planning, and provider-check tests are added
when `plan_actions` is wired in Task 6; do not change `PlannedAction` in this
task.

- [ ] **Step 2: Run the focused unit tests and confirm the helper is missing**

Run: `cargo test --lib plan::tests::fetch_content`

Expected: FAIL because the typed resolver and resolved Fetch Content type do
not exist.

- [ ] **Step 3: Add the URL dependency and planned types**

Add:

```toml
url = "2.5"
```

Add the owned resolved Fetch Content value, but deliberately leave the existing
command-only `PlannedAction` untouched in this checkpoint:

```rust
#[derive(Debug)]
pub struct PlannedFetchContentAction {
    source: url::Url,
    target: PathBuf,
    on_conflict: FetchContentConflict,
}
```

Expose read-only public getters plus a `pub(crate) fn new(...)` constructor.
The crate-private constructor lets the pure resolver and private fetch module
tests create the value without exposing a public transport/plugin API. The
type becomes a `PlannedActionKind` variant in Task 6. Keep
`PlannedManualPackage.install` as `ResolvedCommandAction`. Set the planned
policy with `action.on_conflict.unwrap_or_default()`; do not defer the default
to execution.

- [ ] **Step 4: Implement only the pure Fetch Content resolver**

Do not touch `plan_actions` or remove Task 2's temporary planning error in this
task. Implement a pure helper that resolves both string expressions, types the
locator pair, and constructs the owned value:

```rust
fn resolve_fetch_content_fields(
    action_id: &SelectorIdentifier,
    action: &FetchContentAction,
    context: &ResolveContext<'_>,
    config_dir: &Path,
) -> Result<PlannedFetchContentAction, PlanningError> {
    let source_value = resolve_string_expression(&action.source, context)
        .map_err(/* action:ID source context */)?;
    let target_value = resolve_string_expression(&action.target, context)
        .map_err(/* action:ID target context */)?;
    Ok(PlannedFetchContentAction::new(
        resolve_fetch_source(action_id.as_str(), source_value.value())?,
        resolve_fetch_target(action_id.as_str(), target_value.value(), config_dir)?,
        action.on_conflict.unwrap_or_default(),
    ))
}
```

The subordinate locator functions are:

```rust
fn resolve_fetch_source(action: &str, value: &str) -> Result<Url, PlanningError> {
    let url = Url::parse(value).map_err(|source| PlanningError::InvalidFetchSource {
        action: action.to_owned(),
        value: value.to_owned(),
        source,
    })?;
    if url.scheme() != "https" || url.host_str().is_none() {
        return Err(PlanningError::UnsupportedFetchSource {
            action: action.to_owned(),
            value: value.to_owned(),
        });
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(PlanningError::AuthenticatedFetchSource {
            action: action.to_owned(),
        });
    }
    Ok(url)
}

fn resolve_fetch_target(
    action: &str,
    value: &str,
    config_dir: &Path,
) -> Result<PathBuf, PlanningError> {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        return Ok(path);
    }
    if Url::parse(value).is_ok() {
        return Err(PlanningError::UnsupportedFetchTarget {
            action: action.to_owned(),
            value: value.to_owned(),
        });
    }
    Ok(config_dir.join(path))
}
```

Pass `dot_paths.config_dir()` as the relative base. Planning errors must
identify `action:ID` plus `source` or `target`. Do not inspect the target or
create an HTTP client here.

- [ ] **Step 5: Re-run locator tests and compile the command-only pipeline**

Run:

```bash
cargo test --lib plan::tests::fetch_content
cargo test --test job --test dry_run --test check
```

Expected: PASS. Existing jobs remain Command-only at this checkpoint; the new
resolver is tested directly and no production path can yet bypass the
temporary planning error.

- [ ] **Step 6: Commit typed fetch planning**

```bash
git add Cargo.toml Cargo.lock src/plan.rs
git commit -m "feat: resolve fetch content locators"
```

## Task 4: Add Fetch Content presentation facts without wiring the plan

**Files:**

- Modify: `src/report.rs`
- Modify: `src/dry_run.rs`
- Modify: `src/app/apply.rs`
- Modify: `src/output/table.rs`
- Modify: `tests/report_schema.rs`
- Modify: `tests/output_table.rs`

- [ ] **Step 1: Add failing report-value assertions**

In `tests/report_schema.rs`, directly build an
`ActionInfo::FetchContent` with a resolved URL/path and assert those remain
typed presentation facts. Actual Fetch dry-run projection remains deferred to
the vertical wiring in Task 6.

- [ ] **Step 2: Run the projection tests and observe the command-only assumptions**

Run: `cargo test --test list_command --test dry_run --test report_schema --test output_table`

Expected: FAIL because `ActionInfo` is still a command-only struct.

- [ ] **Step 3: Make presentation facts a structural sum**

Change `ActionInfo` in `src/report.rs` to:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionInfo {
    Command {
        check: Option<CommandInfo>,
        exec: CommandInfo,
    },
    FetchContent {
        source: String,
        target: PathBuf,
        on_conflict: FetchContentConflict,
    },
}
```

Provide `from_resolved_command` for existing planned/manual Command Actions and
`from_fetch_content(&PlannedFetchContentAction)` for the resolved Fetch value
introduced in Task 3. Manual packages call only the command constructor. Keep
the exhaustive source-Action match in `src/app/list_jobs.rs` from Task 2.
Update every existing command call site in `src/dry_run.rs` and
`src/app/apply.rs` to call `from_resolved_command`; this is part of the same
compiling rename, not deferred to Task 6.

- [ ] **Step 4: Render the new report variant**

In `src/output/table.rs`, keep command detail unchanged and render Fetch Content as:

```text
VIA: fetch
DETAIL: https://example.com/config.toml → /resolved/configs/app.toml
```

Do not expose staging paths, transport internals, or inferred target state.

- [ ] **Step 5: Re-run all currently reachable projection tests**

Run: `cargo test --test list_command --test dry_run --test dry_run_command --test report_schema --test output_table --test output_tsv`

Expected: PASS; list includes the unresolved Fetch row, existing Command
dry-run rows are unchanged, and the Fetch presentation variant renders
independently. No Fetch Content value can enter dry-run yet because Task 2's
explicit planning boundary remains in force.

- [ ] **Step 6: Commit listing and dry-run support**

```bash
git add src/report.rs src/dry_run.rs src/app/apply.rs src/output/table.rs tests/report_schema.rs tests/output_table.rs
git commit -m "feat: model fetch content report facts"
```

## Task 5: Implement private HTTPS transfer and safe local materialization

**Files:**

- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `src/lib.rs`
- Modify: `src/action_runner.rs`
- Create: `src/fetch_content.rs`

- [ ] **Step 1: Add failing private unit tests for the complete target matrix**

Inside `src/fetch_content.rs`, define a `FakeTransport` that records call count and either writes configured bytes or returns a configured error. Add tests for:

- missing target creates missing parents and returns `Created`;
- eligible target always calls transport once, even for equal bytes;
- `error` rejects regular files and symlinks before transport;
- `replace` replaces a regular file and a symlink without changing the symlink destination;
- replacing a hard-linked regular target changes only that directory entry and
  leaves the other hard-link entry on the old bytes;
- `replace` replaces a symlink that points to a directory;
- both policies reject a real directory before transport;
- both policies reject a Unix-domain socket under `#[cfg(unix)]` before transport;
- transport failure leaves an existing replace target unchanged;
- unusable parent/staging setup fails before transport;
- a target created by the fake transport after preflight causes a commit failure;
- a directory symlink changed by the fake transport into a real directory is
  rejected at commit and the real directory is not removed;
- transfer and failed commit clean staging files on a best-effort basis.

Use `tempfile::tempdir()` for test roots. On Unix, use `std::os::unix::net::UnixListener` for the special-entry case. Reuse or mirror the existing cross-platform symlink helpers in `src/link.rs` rather than assuming Unix APIs.
Construct test requests with Task 3's crate-private
`PlannedFetchContentAction::new`; do not route unit tests through the still
gated `ExecutionPlanner` and do not make fields or the transport seam public.

- [ ] **Step 2: Declare the module and run its empty/missing implementation test target**

Add `mod fetch_content;` to `src/lib.rs`, then run:

```bash
cargo test --lib fetch_content::tests
```

Expected: FAIL until the runner, transport seam, and error types exist.

- [ ] **Step 3: Add deliberately narrow production dependencies**

Add:

```toml
tempfile = "3.27"
ureq = { version = "3.3", default-features = false, features = ["rustls"] }
```

Do not enable `gzip`, cookies, JSON, charset conversion, SOCKS, system-proxy configuration, or native TLS features. Regenerate `Cargo.lock` through Cargo, not by hand.

- [ ] **Step 4: Implement typed outcomes, stages, and the internal transport seam**

Use this shape (names may vary only if all downstream matches stay equally explicit):

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FetchContentOutcome {
    Created,
    Replaced,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FetchContentStage {
    Preflight,
    Prepare,
    Transfer,
    Commit,
}

pub(crate) trait FetchTransport {
    fn fetch(&self, source: &Url, output: &mut dyn Write)
        -> Result<(), FetchTransportError>;
}

pub(crate) struct FetchContentRunner<'a> {
    transport: &'a dyn FetchTransport,
}

pub(crate) struct UreqHttpsTransport {
    agent: ureq::Agent,
}
```

`FetchContentError` is a public opaque struct because public
`job_runner::ActionOutcome` will carry it; its fields/internal error enum and
its `stage()` accessor remain private or `pub(crate)`. It must implement
`Display` and `Error`, preserve a source where applicable, and let apply code
obtain typed internal stage detail without parsing text. Keep
`FetchContentOutcome` public for the same interface reason. Publicly re-export
these two types from the existing public `action_runner` module while keeping
`fetch_content`, `FetchTransport`, `FetchContentRunner`,
`UreqHttpsTransport`, and all transport policy types crate-private. This keeps
the public `JobOutcome` nameable and avoids `private_interfaces` under Clippy
without exposing a transport plugin surface.

Separate conflict/unsupported-entry details from I/O, transport, staging, and
commit failures inside the opaque error.
`UreqHttpsTransport::new()` owns the reusable agent; `FetchContentRunner` only
borrows a transport for one dispatch. Do not create a fresh connection pool for
every Action.

- [ ] **Step 5: Implement preflight, staging, and commit in that order**

The runner algorithm is:

```rust
pub(crate) fn run(
    &self,
    action: &PlannedFetchContentAction,
) -> Result<FetchContentOutcome, FetchContentError> {
    let existing = classify_with_symlink_metadata(action.target(), action.on_conflict())?;
    let outcome = existing.outcome();

    let parent = action.target().parent().ok_or_else(/* prepare error */)?;
    fs::create_dir_all(parent).map_err(/* prepare error */)?;
    let mut staged = NamedTempFile::new_in(parent).map_err(/* prepare error */)?;

    self.transport
        .fetch(action.source(), staged.as_file_mut())
        .map_err(/* transfer error */)?;
    staged.as_file_mut().flush().map_err(/* transfer error */)?;

    if existing.must_remove() {
        let current = reclassify_for_commit(action.target())?;
        match current {
            CurrentTarget::Missing => {}
            CurrentTarget::RegularFile | CurrentTarget::Symlink => {
                remove_exact_current_entry(action.target(), current)
                    .map_err(/* commit error */)?;
            }
            CurrentTarget::Directory | CurrentTarget::Special => {
                return Err(/* commit race changed to an ineligible kind */);
            }
        }
    }
    staged
        .persist_noclobber(action.target())
        .map_err(/* commit error */)?;

    Ok(outcome)
}
```

Preflight classification permits only `Missing`, `RegularFile`, and `Symlink`;
`replace` authorizes removal only for the latter two. Immediately before any
removal, call `symlink_metadata` again and base the removal API on the current
file type. If a previously eligible target is now a real directory or special
entry, fail commit without removing it. This closes the Windows race where a
directory symlink would otherwise lead to `remove_dir` deleting a newly
substituted real empty directory. On Windows, distinguish current file and
directory symlinks exactly as `src/link.rs` does. Never use ordinary `metadata`
or recursive removal.

- [ ] **Step 6: Implement the production `ureq` adapter**

Build one reusable blocking agent:

```rust
const MAX_REDIRECTS: u32 = 5;

fn https_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .https_only(true)
        .max_redirects(MAX_REDIRECTS)
        .max_redirects_will_error(true)
        .http_status_as_error(true)
        .build()
        .into()
}
```

The adapter performs exactly one `GET`, checks the returned final status with a
small `validate_final_status` helper, and copies
`response.body_mut().as_reader()` to staging with `std::io::copy` only for
`2xx`. It has no retry loop. Map `ureq::Error::RequireHttpsOnly(_)` to a clear
HTTPS-downgrade failure, `TooManyRedirects` to redirect exhaustion, and
`StatusCode` to an HTTP-status failure.

Add unit assertions that the production agent has HTTPS-only enabled,
`max_redirects() == 5`, redirect exhaustion errors enabled, and status errors
enabled. Add direct error-mapping tests for `RequireHttpsOnly` (HTTPS to HTTP
downgrade), `TooManyRedirects`, `StatusCode(400)`, `StatusCode(500)`, and a
generic transport error. Test `validate_final_status` with representative
`1xx`, `2xx`, `3xx`, `4xx`, and `5xx` values so an unhandled final redirect can
never be accepted. These tests verify policy without external internet access.

- [ ] **Step 7: Run the fetch module tests and dependency checks**

Run:

```bash
cargo test --lib fetch_content::tests
cargo tree -e features -i ureq
```

Expected: all fetch tests PASS; the feature tree includes `rustls` but not
`gzip`, `brotli`, `charset`, `cookies`, or `json`. Treat this feature-tree check
as the deterministic test of the declared raw/no-transparent-content-decoding
policy: if a decoding feature appears, the task fails even when unit tests
pass.

- [ ] **Step 8: Commit the isolated fetch engine**

```bash
git add Cargo.toml Cargo.lock src/lib.rs src/action_runner.rs src/fetch_content.rs
git commit -m "feat: materialize https content at local targets"
```

## Task 6: Wire Fetch Content vertically through plan, execution, and reports

**Files:**

- Modify: `src/plan.rs`
- Modify: `src/action_runner.rs`
- Modify: `src/job_executor.rs`
- Modify: `src/job_runner.rs`
- Modify: `src/report.rs`
- Modify: `src/dry_run.rs`
- Modify: `src/app/apply.rs`
- Modify: `src/output/table.rs`
- Modify: `tests/action_runner.rs`
- Modify: `tests/job_runner.rs`
- Modify: `tests/error_contract.rs`
- Modify: `tests/job.rs`
- Modify: `tests/dry_run.rs`
- Modify: `tests/check.rs`
- Modify: `tests/report_schema.rs`
- Modify: `tests/output_table.rs`
- Create: `tests/fixtures/dry-run/valid-fetch-content-template.toml`
- Create planning-error fixtures under: `tests/fixtures/dry-run/`

- [ ] **Step 1: Add failing dispatch, ordering, and isolation tests**

Add tests proving:

- selected Fetch source/target expressions resolve into
  `PlannedActionKind::FetchContent`;
- invalid source/target locator pairs fail atomically only when selected;
- relative, absolute, and explicit `${dot:real_config_dir}` targets have the
  Task 3 behavior through the real planner;
- dry-run shows the resolved URL/path, remains `PLANNED`, and performs no
  network access or target inspection;
- `check providers` ignores Fetch runtime resolution;
- a planned Command Action still follows `check -> exec -> post-check`;
- a preflight-failing Fetch Content Action appears between Command Actions in canonical Action ID order;
- that Fetch failure does not prevent the later Command Action;
- the later Link phase still runs after the Fetch failure;
- exact `action:ID` selection runs only the selected Fetch Content Action;
- manual package installation accepts/runs only a Command Action.

Use a Fetch target that is a real directory so the production runner fails before network access. This makes job-level isolation deterministic and offline.

- [ ] **Step 2: Run command-runner and job-runner tests**

Run:

```bash
cargo test --test job --test dry_run --test check --test action_runner --test job_runner --test error_contract --test report_schema --test output_table
```

Expected: FAIL because Task 2 still returns the explicit temporary planning
error and `JobExecutor::run_action`/report projection are command-only.

- [ ] **Step 3: Wire the already-tested locator resolver into `PlannedAction`**

Replace the temporary command-only shape and delete
`PlanningError::FetchContentNotYetWired`:

```rust
#[derive(Debug)]
pub struct PlannedAction {
    id: SelectorIdentifier,
    kind: PlannedActionKind,
}

#[derive(Debug)]
pub enum PlannedActionKind {
    Command(ResolvedCommandAction),
    FetchContent(PlannedFetchContentAction),
}
```

Match `SourceAction` in `plan_actions`: Command uses the existing resolver;
Fetch Content calls the Task 3 locator resolver with
`self.dot_paths.config_dir()`. Add getters and update dry-run to construct the
Task 4 `ActionInfo` variant. This is the point where selected-only integration,
exact selection, and atomic planning become production behavior.

- [ ] **Step 4: Rename the command-only runner types for clarity**

In `src/action_runner.rs`, rename:

```text
ActionRunner   -> CommandActionRunner
ActionOutcome  -> CommandActionOutcome
ActionRunError -> CommandActionRunError
```

Its input becomes `&ResolvedCommandAction`. Preserve the current behavior and error/source contract exactly. Update manual package execution and its tests to use these command-specific names.

- [ ] **Step 5: Give the production transport an explicit owner and dispatch**

Use an explicit sum in `src/job_runner.rs`:

```rust
#[derive(Debug)]
pub enum ActionOutcome {
    Command(Result<CommandActionOutcome, CommandActionRunError>),
    FetchContent(Result<FetchContentOutcome, FetchContentError>),
}

impl ActionOutcome {
    fn is_succeeded(&self) -> bool {
        match self {
            Self::Command(result) => result.is_ok(),
            Self::FetchContent(result) => result.is_ok(),
        }
    }
}
```

Keep `JobOutcome::ManualPackage(Result<CommandActionOutcome, CommandActionRunError>)`; change only `JobOutcome::Action` to hold the new sum. In `JobExecutor::run_action`, match `PlannedActionKind` and dispatch to `CommandActionRunner` or `FetchContentRunner`.

The current executor/runner derives and constructors are incompatible with an
owned reusable HTTP agent. Change them explicitly:

```rust
pub(crate) struct JobExecutor<'a> {
    provider_runner: ProviderRunner<'a>,
    command_action_runner: CommandActionRunner<'a>,
    fetch_transport: UreqHttpsTransport,
}

impl<'a> JobExecutor<'a> {
    pub(crate) fn new(environment: &'a ExecutionEnvironment) -> Self {
        Self {
            provider_runner: ProviderRunner::new(environment),
            command_action_runner: CommandActionRunner::new(environment),
            fetch_transport: UreqHttpsTransport::new(),
        }
    }
}
```

Remove `Copy`, `Clone`, and `Debug` derives from `JobExecutor` and `JobRunner`;
none is required by their public behavior, and removing `Debug` avoids forcing
the private `ureq::Agent` wrapper to expose/debug-print transport state. Change
both `JobExecutor::new` and `JobRunner::new` from `const fn` to ordinary `fn`.
For a Fetch dispatch, create a short-lived
`FetchContentRunner::new(&self.fetch_transport)` so every Action reuses the
same agent/connection pool without making transport public.

Do not introduce a new phase, dependency edge, or early return. The current per-job loop must continue after either variant fails.

- [ ] **Step 6: Make apply/report matches exhaustive in the same compiling change**

Before the PASS checkpoint, update `src/app/apply.rs`, `src/report.rs`, and
`src/output/table.rs` for the new outcome. Add `EvidenceStage::Fetch`; map
Created/Replaced to their matching item statuses and every error to Failed.
Reuse Task 4's `ActionInfo::FetchContent` for both dry-run and apply. A more
detailed stage/evidence contract is locked down in Task 7, but no wildcard,
panic, or fake Command outcome is allowed here.

- [ ] **Step 7: Re-run the complete vertical slice**

Run:

```bash
cargo test --lib fetch_content::tests
cargo test --test job --test dry_run --test dry_run_command --test check --test action_runner --test job_runner --test error_contract --test report_schema --test output_table --test link
```

Expected: PASS; selected Fetch values now cross the plan/report/execution
boundary, Command semantics are unchanged, and preflight Fetch failures are
isolated without network access.

- [ ] **Step 8: Commit the vertical wiring**

```bash
git add src tests
git commit -m "feat: execute fetch content actions"
```

## Task 7: Lock down Fetch Content evidence and end-to-end failure reporting

**Files:**

- Modify: `src/report.rs`
- Modify and add private unit tests in: `src/app/apply.rs`
- Modify: `src/output/table.rs`
- Modify: `tests/report_schema.rs`
- Modify: `tests/output_table.rs`
- Modify: `tests/apply_command.rs`

- [ ] **Step 1: Add failing report-projection tests**

Extend the existing `#[cfg(test)] mod tests` inside `src/app/apply.rs`; do not
declare a second module with the same name. Plan the Fetch fixture
from Task 6 to obtain a real `PlannedAction`, then pair it with synthetic
crate-private outcomes/errors and call the item-projection helper directly.
This preserves the private transport/outcome boundary while testing:

- `FetchContentOutcome::Created -> ItemStatus::Created`;
- `FetchContentOutcome::Replaced -> ItemStatus::Replaced`;
- every `FetchContentStage` maps to structured evidence with no fabricated exit code;
- conflict and directory/special-entry failures report `FAILED` with a preflight message;
- transfer/HTTP/redirect failures report `FAILED` at `EvidenceStage::Fetch`;
- commit failures report `FAILED` with commit-stage detail;
- a mixed apply report is globally failed if one fetch fails while later successful items still appear.

Keep only externally observable shape/table tests in `tests/report_schema.rs`,
`tests/output_table.rs`, and `tests/apply_command.rs`. The integration test uses
a real-directory preflight failure (no network) to verify global failure plus
later-item continuation; it must not attempt to construct crate-private types.

- [ ] **Step 2: Run apply/report/output tests and confirm the missing matches**

Run:

```bash
cargo test --lib app::apply::tests
cargo test --test apply_command --test report_schema --test output_table
```

Expected: the focused unit cases FAIL because Task 6 has only the minimal
exhaustive mapping and does not yet preserve every Fetch stage/message detail.
The integration tests compile without access to private outcomes.

- [ ] **Step 3: Add one Fetch evidence category and preserve fine-grained stage text**

Keep the runner's `Preflight`, `Prepare`, `Transfer`, and `Commit` detail in
`Evidence.message`; do not inflate the global report enum beyond the
`EvidenceStage::Fetch` variant added in Task 6.

Split the current `action_result` projection into:

```rust
fn command_action_result(
    result: &Result<CommandActionOutcome, CommandActionRunError>,
    executed_status: ItemStatus,
) -> (ItemStatus, Vec<Evidence>);

fn selected_action_result(
    result: &ActionOutcome,
) -> (ItemStatus, Vec<Evidence>);
```

For Fetch Content, map only `Created`, `Replaced`, and `Failed`; never return `Satisfied` or `Executed`.

- [ ] **Step 4: Update table rendering and diagnostics**

Teach `EvidenceStage` rendering about `Fetch` and ensure the report detail still comes from `ActionInfo::FetchContent`, not from an error string. Preserve error sources for diagnostic hint processing where applicable.

- [ ] **Step 5: Run the full report and command-output slice**

Run:

```bash
cargo test --test apply_command --test dry_run_command --test report_schema --test output_table --test output_tsv --test diagnostic
```

Also run: `cargo test --lib app::apply::tests`

Expected: PASS with unchanged Command Action output, typed Fetch Content
statuses, and private outcome/error projection coverage.

- [ ] **Step 6: Commit apply reporting**

```bash
git add src/report.rs src/app/apply.rs src/output/table.rs tests
git commit -m "feat: report fetch content outcomes"
```

## Task 8: Finish current documentation and migration guidance

**Files:**

- Modify: `docs/SCHEMA.txt`
- Modify: `docs/DESIGN.txt`
- Modify: `docs/CONFIGURATION.md`
- Modify: `README.md`
- Do not modify: `docs/archive/v0.1.0/**`

- [ ] **Step 1: Write a failing documentation consistency check**

Run:

```bash
rg -n "ExecActionType|type = ['\"]exec['\"]" README.md docs --glob '!docs/archive/**'
```

Expected before updates: active documentation matches remain.

- [ ] **Step 2: Complete the normative schema text**

Ensure `docs/SCHEMA.txt` defines the untagged Action sum, command-only manual install, Fetch Content fields/defaults, and the removal of `ExecAction.type`. Keep protocol/runtime behavior out of schema grammar sections except where needed for runtime validation notes.

- [ ] **Step 3: Update runtime design and user reference**

In `docs/DESIGN.txt`, document selected-only locator resolution, entry-relative targets, preflight matrix, staging/commit limits, existing Action phase/order/isolation, and the no-download-manager boundary.

In `docs/CONFIGURATION.md`, replace every active `type = "exec"` example, remove `ExecActionType`, add Command/Fetch Content Action tables and examples, and explicitly say richer transfers belong in Command Actions/scripts.

- [ ] **Step 4: Update the README without expanding the product claim**

Mention Fetch Content as a small built-in Action for `HTTPS -> local path`, not as downloads/artifact management. Include one short example and link to the configuration reference. State that it always fetches on eligible apply and supports only `error`/`replace` conflict handling.

- [ ] **Step 5: Verify active docs and archive isolation**

Run:

```bash
rg -n "ExecActionType|type = ['\"]exec['\"]" README.md docs --glob '!docs/archive/**'
git diff --name-only 23761ce -- docs/archive/v0.1.0
```

Expected: first command has no matches; second command has no output.

- [ ] **Step 6: Commit documentation**

```bash
git add README.md docs/SCHEMA.txt docs/DESIGN.txt docs/CONFIGURATION.md
git commit -m "docs: describe fetch content actions"
```

## Task 9: Run full verification and audit the feature boundary

**Files:**

- Modify only files required by failures found in this task.

- [ ] **Step 1: Format and make formatting failures visible**

Run:

```bash
cargo fmt --all
cargo fmt --all -- --check
```

Expected: PASS after the first command. If formatting changes tracked files, inspect and commit them with the relevant task fix rather than creating an opaque bulk commit.

- [ ] **Step 2: Run all host tests**

Run:

```bash
cargo test --locked --all-targets --all-features
```

Expected: PASS, including all existing tests and the new schema/planning/fetch/report cases.

- [ ] **Step 3: Run Clippy with warnings denied**

Run:

```bash
cargo clippy --locked --all-targets --all-features -- -D warnings
```

Expected: PASS with no warnings.

- [ ] **Step 4: Audit removed and forbidden concepts**

Run:

```bash
rg -n "ExecActionType|type = ['\"]exec['\"]" src tests README.md docs --glob '!docs/archive/**'
rg -n "sha256|checksum|etag|last-modified|cache_key|resume|extract|download_id" src/schema.rs src/fetch_content.rs docs/SCHEMA.txt
```

Expected: no legacy type matches; no new schema/runtime fields for forbidden download-manager concepts. Documentation prose may mention non-goals elsewhere.

- [ ] **Step 5: Audit the dependency feature boundary and worktree**

Run:

```bash
cargo tree -e features -i ureq
git status --short
git diff --check
```

Expected: `ureq` has Rustls but no gzip/decompression feature; no unintended files; no whitespace errors.

- [ ] **Step 6: Inspect the repository's CI matrix rather than claiming unrun platforms**

Confirm the existing GitHub Actions workflow still exercises the supported Linux/macOS/Windows matrix and that no platform-specific fetch test bypasses the behavior it is meant to cover. Do not claim those remote jobs passed until CI actually reports them.

- [ ] **Step 7: Commit any verification-driven fixes**

```bash
git add <only-files-fixed-during-verification>
git commit -m "test: complete fetch content coverage"
```

Skip this commit if verification required no changes.

## Final acceptance checklist

- [ ] Existing Command Actions without `type` behave unchanged.
- [ ] Every active `type = "exec"` is rejected; archived documentation is untouched.
- [ ] Fetch Content is selected as `action:ID`, inherits/replaces like any Action, and runs in Action ID order.
- [ ] Only selected Fetch Content locator values resolve; list remains structural and dry-run remains side-effect-free.
- [ ] Only public absolute HTTPS sources and native local targets are planned.
- [ ] Relative targets use the configuration entry directory; `${dot:real_config_dir}` remains explicit.
- [ ] `error` and `replace` implement the approved target-kind matrix without following symlinks.
- [ ] Every eligible apply performs one fresh fetch; there is no satisfied/cache/checksum path.
- [ ] Transfer completes to staging before commit begins; failures are typed and locally isolated.
- [ ] Apply reports only `CREATED`, `REPLACED`, or `FAILED` for Fetch Content.
- [ ] Full host tests, rustfmt, and Clippy pass; remote platform results are left to CI.

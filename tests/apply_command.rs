mod support;

use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::sync::atomic::{AtomicU64, Ordering};

use support::fixture;

static NEXT_WORKSPACE: AtomicU64 = AtomicU64::new(0);

struct TempWorkspace {
    directory: PathBuf,
}

impl TempWorkspace {
    fn new() -> Self {
        let sequence = NEXT_WORKSPACE.fetch_add(1, Ordering::Relaxed);
        let directory =
            env::temp_dir().join(format!("dot-apply-command-{}-{sequence}", process::id()));
        fs::create_dir(&directory).expect("temporary workspace should be created");
        Self { directory }
    }

    fn write_manifest(&self, contents: &str) -> PathBuf {
        let path = self.directory.join("dot.toml");
        fs::write(&path, render_manifest(contents)).expect("test manifest should be written");
        path
    }

    fn write_source(&self, name: &str) -> PathBuf {
        let path = self.directory.join(name);
        fs::write(&path, name).expect("link source should be written");
        path
    }

    fn events(&self) -> PathBuf {
        self.directory.join("events")
    }

    fn recorded_events(&self) -> Vec<String> {
        fs::read_to_string(self.events())
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn path(&self, name: &str) -> PathBuf {
        self.directory.join(name)
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn render_manifest(contents: &str) -> String {
    contents
        .replace("__OS__", env::consts::OS)
        .replace("__PROGRAM__", &helper_program_toml())
}

fn helper_program_toml() -> String {
    format!(
        "{:?}",
        env::current_exe()
            .expect("test executable should have a path")
            .to_string_lossy()
    )
}

fn helper_exec(mode: &str) -> String {
    format!(
        r#"{{ program = __PROGRAM__, args = ["--exact", "helper_process", "--nocapture"], env = {{ variables = {{ DOT_APPLY_HELPER = "{mode}", DOT_APPLY_EVENTS = "${{dot:config_dir}}/events", DOT_APPLY_LINK = "${{dot:config_dir}}/linked.txt" }} }} }}"#
    )
}

fn exact_manifest(workspace: &TempWorkspace) -> PathBuf {
    workspace.write_source("selected-source.txt");
    workspace.write_source("unselected-source.txt");
    workspace.write_manifest(&fixture::read(
        "selection/valid-exact-command-template.toml",
    ))
}

fn run_exact_apply(manifest: &Path, selectors: &[&str]) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_dot"));
    command
        .arg("--config")
        .arg(manifest)
        .arg("apply")
        .env_remove("DOT_INTENTIONALLY_MISSING");
    for selector in selectors {
        command.args(["--job", selector]);
    }
    command.output().expect("dot apply should start")
}

fn report_job_rows(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .filter_map(|line| {
            let cells = line
                .trim_matches('│')
                .split('┆')
                .map(str::trim)
                .collect::<Vec<_>>();
            let [kind, id, ..] = cells.as_slice() else {
                return None;
            };
            matches!(*kind, "provider" | "package" | "action" | "link")
                .then(|| format!("{kind}:{id}"))
        })
        .collect()
}

fn assert_exact_workspace_untouched(workspace: &TempWorkspace) {
    assert!(
        !workspace.events().exists(),
        "an atomic planning failure must not create an event log"
    );
    assert!(
        !workspace.path("selected-linked.txt").exists(),
        "an atomic planning failure must not create the selected link"
    );
    assert!(
        !workspace.path("unselected-linked.txt").exists(),
        "an atomic planning failure must not create the unselected link"
    );
}

#[test]
fn exact_apply_executes_only_selected_jobs_in_stable_plan_order() {
    for selectors in [
        ["package:selected", "link:selected-link"],
        ["link:selected-link", "package:selected"],
    ] {
        let workspace = TempWorkspace::new();
        let manifest = exact_manifest(&workspace);
        let output = run_exact_apply(&manifest, &selectors);
        let stdout = String::from_utf8(output.stdout).expect("apply report should be UTF-8");

        assert!(
            output.status.success(),
            "stdout:\n{stdout}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            report_job_rows(&stdout),
            [
                "provider:selected-provider",
                "package:selected",
                "link:selected-link",
            ],
            "{stdout}"
        );
        assert_eq!(
            workspace.recorded_events(),
            ["selected-provider-probe", "selected-provider-install"]
        );
        assert_eq!(
            fs::canonicalize(workspace.path("selected-linked.txt"))
                .expect("selected link should resolve"),
            fs::canonicalize(workspace.path("selected-source.txt"))
                .expect("selected source should resolve")
        );
        assert!(
            !workspace.path("unselected-linked.txt").exists(),
            "unselected link must remain absent"
        );
        assert!(
            stdout.contains("SUCCESS · 3 items · 1 provider · 1 package · 0 actions · 1 link"),
            "{stdout}"
        );
    }
}

#[test]
fn exact_apply_rejects_an_unknown_selector_atomically() {
    let workspace = TempWorkspace::new();
    let manifest = exact_manifest(&workspace);
    let output = run_exact_apply(
        &manifest,
        &["package:selected", "link:selected-link", "action:unknown"],
    );

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty(), "{:?}", output.stdout);
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("unknown job `action:unknown`"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_exact_workspace_untouched(&workspace);
}

#[test]
fn exact_apply_rejects_a_selected_runtime_failure_atomically() {
    let workspace = TempWorkspace::new();
    let manifest = exact_manifest(&workspace);
    let output = run_exact_apply(
        &manifest,
        &[
            "package:selected",
            "action:missing-runtime",
            "link:selected-link",
        ],
    );

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty(), "{:?}", output.stdout);
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("environment variable `DOT_INTENTIONALLY_MISSING` is not defined"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_exact_workspace_untouched(&workspace);
}

#[test]
fn apply_runs_the_complete_plan_in_phase_order_and_prints_a_summary() {
    let workspace = TempWorkspace::new();
    let source = workspace.write_source("source.txt");
    let contents = fixture::read("apply/valid-complete-plan-template.toml")
        .replace("__PROBE__", &helper_exec("probe-ready"))
        .replace("__INSTALL__", &helper_exec("install-ready"))
        .replace("__MANUAL__", &helper_exec("manual-ok"))
        .replace("__ACTION__", &helper_exec("action-ok"));
    let manifest = workspace.write_manifest(&contents);

    let output = Command::new(env!("CARGO_BIN_EXE_dot"))
        .args(["apply", "--config"])
        .arg(&manifest)
        .output()
        .expect("dot apply should start");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "stdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        workspace.recorded_events(),
        [
            "probe-ready",
            "install-ready",
            "install-ready",
            "manual-ok",
            "action-ok"
        ]
    );
    assert_eq!(
        fs::canonicalize(workspace.path("linked.txt")).expect("link should resolve"),
        fs::canonicalize(source).expect("source should resolve")
    );
    assert!(stdout.contains("dot apply · target=current"), "{stdout}");
    assert!(stdout.contains("│ provider ┆ ready"), "{stdout}");
    assert!(stdout.contains("│ package  ┆ cli-tools"), "{stdout}");
    assert!(stdout.contains("names: [bat, fd, fzf]"), "{stdout}");
    assert!(stdout.contains("│ package  ┆ tool"), "{stdout}");
    assert!(stdout.contains("│ package  ┆ manual-tool"), "{stdout}");
    assert!(stdout.contains("│ action   ┆ configure"), "{stdout}");
    assert!(stdout.contains("│ link     ┆ config"), "{stdout}");
    assert!(stdout.contains("READY"), "{stdout}");
    assert!(stdout.contains("INSTALLED"), "{stdout}");
    assert!(stdout.contains("EXECUTED"), "{stdout}");
    assert!(stdout.contains("CREATED"), "{stdout}");
    assert!(stdout.contains("SUCCESS · 6 items"), "{stdout}");
}

#[test]
fn apply_continues_unrelated_work_and_fails_when_any_runtime_item_fails() {
    let workspace = TempWorkspace::new();
    let source = workspace.write_source("source.txt");
    let contents = fixture::read("apply/invalid-runtime-items-template.toml")
        .replace("__MISSING_PROBE__", &helper_exec("probe-missing"))
        .replace("__UNEXPECTED_INSTALL__", &helper_exec("install-unexpected"))
        .replace("__READY_PROBE__", &helper_exec("probe-ready"))
        .replace("__READY_INSTALL__", &helper_exec("install-ready"))
        .replace("__MANUAL_FAIL__", &helper_exec("manual-fail"))
        .replace("__MANUAL_OK__", &helper_exec("manual-ok"))
        .replace("__ACTION_FAIL__", &helper_exec("action-fail"))
        .replace("__ACTION_OK__", &helper_exec("action-ok"));
    let manifest = workspace.write_manifest(&contents);

    let output = Command::new(env!("CARGO_BIN_EXE_dot"))
        .args(["apply", "--config"])
        .arg(&manifest)
        .output()
        .expect("dot apply should start");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert_eq!(output.status.code(), Some(1), "stdout:\n{stdout}");
    assert_eq!(
        workspace.recorded_events(),
        [
            "probe-missing",
            "probe-ready",
            "install-ready",
            "manual-fail",
            "manual-ok",
            "action-fail",
            "action-ok",
        ]
    );
    assert_eq!(
        fs::canonicalize(workspace.path("linked.txt")).expect("working link should resolve"),
        fs::canonicalize(source).expect("source should resolve")
    );
    assert!(stdout.contains("│ provider ┆ a-missing"), "{stdout}");
    assert!(stdout.contains("│ package  ┆ blocked-tool"), "{stdout}");
    assert!(stdout.contains("│ package  ┆ working-tool"), "{stdout}");
    assert!(stdout.contains("│ package  ┆ manual-fail"), "{stdout}");
    assert!(stdout.contains("│ package  ┆ manual-ok"), "{stdout}");
    assert!(stdout.contains("│ action   ┆ action-fail"), "{stdout}");
    assert!(stdout.contains("│ action   ┆ action-ok"), "{stdout}");
    assert!(stdout.contains("│ link     ┆ broken"), "{stdout}");
    assert!(stdout.contains("│ link     ┆ working"), "{stdout}");
    assert!(stdout.contains("NOT_READY"), "{stdout}");
    assert!(stdout.contains("BLOCKED"), "{stdout}");
    assert!(stdout.contains("provider unavailable"), "{stdout}");
    assert!(stdout.contains("INSTALLED"), "{stdout}");
    assert!(stdout.contains("EXECUTED"), "{stdout}");
    assert!(stdout.contains("CREATED"), "{stdout}");
    assert!(stdout.contains("FAILED · 10 items"), "{stdout}");
}

#[test]
fn fetch_preflight_failure_is_reported_while_later_actions_and_links_continue() {
    let workspace = TempWorkspace::new();
    let source = workspace.write_source("source.txt");
    fs::create_dir(workspace.path("fetch-target"))
        .expect("directory target should force an offline preflight failure");
    let contents = fixture::read("jobs/valid-fetch-failure-continuation-template.toml")
        .replace("__BEFORE__", &helper_exec("action-ok"))
        .replace("__AFTER__", &helper_exec("action-ok"));
    let manifest = workspace.write_manifest(&contents);

    let output = Command::new(env!("CARGO_BIN_EXE_dot"))
        .args(["apply", "--config"])
        .arg(&manifest)
        .output()
        .expect("dot apply should start");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert_eq!(output.status.code(), Some(1), "stdout:\n{stdout}");
    assert_eq!(workspace.recorded_events(), ["action-ok", "action-ok"]);
    assert_eq!(
        report_job_rows(&stdout),
        [
            "action:a-before",
            "action:b-fetch",
            "action:c-after",
            "link:config",
        ],
        "{stdout}"
    );
    assert_eq!(stdout.matches("EXECUTED").count(), 2, "{stdout}");
    assert!(stdout.contains("FAILED"), "{stdout}");
    assert!(stdout.contains("directory"), "{stdout}");
    assert!(stdout.contains("preflight"), "{stdout}");
    assert!(
        stdout.contains("https://192.0.2.1/unreachable →"),
        "{stdout}"
    );
    assert!(!stdout.contains("exit 0"), "{stdout}");
    assert_eq!(
        fs::canonicalize(workspace.path("linked.txt")).expect("link should resolve"),
        fs::canonicalize(source).expect("source should resolve")
    );
    assert!(stdout.contains("FAILED · 4 items"), "{stdout}");
}

#[test]
fn apply_projects_a_link_phase_error_as_blocked_items_and_one_diagnostic() {
    let workspace = TempWorkspace::new();
    workspace.write_source("first.txt");
    workspace.write_source("second.txt");
    let contents = fixture::read("apply/invalid-duplicate-link-target-template.toml")
        .replace("__ACTION__", &helper_exec("action-ok"));
    let manifest = workspace.write_manifest(&contents);

    let output = Command::new(env!("CARGO_BIN_EXE_dot"))
        .args(["apply", "--config"])
        .arg(&manifest)
        .output()
        .expect("dot apply should start");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert_eq!(output.status.code(), Some(1), "stdout:\n{stdout}");
    assert_eq!(workspace.recorded_events(), ["action-ok"]);
    assert!(stdout.contains("┆ first "), "{stdout}");
    assert!(stdout.contains("┆ second "), "{stdout}");
    assert_eq!(stdout.matches("BLOCKED").count(), 2, "{stdout}");
    assert_eq!(stdout.matches("ERROR: links [").count(), 1, "{stdout}");
    assert!(stdout.contains("resolve to the same target"), "{stdout}");
    assert!(stdout.contains("FAILED · 3 items"), "{stdout}");
    assert!(
        !workspace.path("linked.txt").exists(),
        "duplicate-target preflight must not create the normalized target"
    );
}

#[cfg(feature = "dev-platform-override")]
#[test]
fn apply_warns_that_the_platform_override_is_ignored() {
    let workspace = TempWorkspace::new();
    let manifest = workspace.write_manifest(
        r#"
[targets.current]
platform = { os = "__OS__" }
"#,
    );

    let output = Command::new(env!("CARGO_BIN_EXE_dot"))
        .args([
            "apply",
            "--platform",
            r#"{ os = "never-os", arch = "never-arch" }"#,
            "--config",
        ])
        .arg(&manifest)
        .output()
        .expect("dot apply should start");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "{stderr}");
    assert!(
        stderr.contains("--platform is ignored by apply"),
        "{stderr}"
    );
    assert!(stderr.contains("detected host PlatformInfo"), "{stderr}");
}

#[test]
fn helper_process() {
    if let Ok(mode) = env::var("DOT_EXACT_HELPER") {
        let events = PathBuf::from(
            env::var_os("DOT_EXACT_EVENTS").expect("exact-selection event path should be present"),
        );
        let expected_provider = match mode.as_str() {
            "selected-provider-probe" | "selected-provider-install" => Some("selected-provider"),
            "unrelated-provider-probe" | "unrelated-provider-install" => Some("unrelated-provider"),
            "manual-package" | "runnable-action" | "missing-runtime" => None,
            unknown => panic!("unknown exact-selection helper mode: {unknown}"),
        };
        assert_eq!(
            env::var("DOT_EXACT_PROVIDER").ok().as_deref(),
            expected_provider,
            "only provider phases should receive provider activation"
        );
        record(&events, &mode);
        return;
    }

    let Ok(mode) = env::var("DOT_APPLY_HELPER") else {
        return;
    };
    let events =
        PathBuf::from(env::var_os("DOT_APPLY_EVENTS").expect("apply event path should be present"));

    if mode.starts_with("probe-") || mode.starts_with("install-") {
        assert_eq!(
            env::var("DOT_APPLY_PROVIDER_ACTIVE").as_deref(),
            Ok("yes"),
            "provider child process should receive activate environment"
        );
    } else {
        assert!(
            env::var_os("DOT_APPLY_PROVIDER_ACTIVE").is_none(),
            "manual and global actions must not receive provider environment"
        );
    }

    match mode.as_str() {
        "probe-ready" | "install-ready" | "manual-ok" => record(&events, &mode),
        "probe-missing" => {
            record(&events, &mode);
            process::exit(1);
        }
        "manual-fail" => {
            record(&events, &mode);
            process::exit(31);
        }
        "action-ok" => {
            let link = PathBuf::from(
                env::var_os("DOT_APPLY_LINK").expect("apply link path should be present"),
            );
            assert!(!link.exists(), "links must run after global actions");
            record(&events, &mode);
        }
        "action-fail" => {
            record(&events, &mode);
            process::exit(32);
        }
        "install-unexpected" => panic!("unavailable provider install must not run"),
        unknown => panic!("unknown apply helper mode: {unknown}"),
    }
}

fn record(path: &Path, event: &str) {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("event log should open");
    writeln!(file, "{event}").expect("event should be recorded");
}

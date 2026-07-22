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
        .args(["--config"])
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
        .args(["--config"])
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
fn helper_process() {
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

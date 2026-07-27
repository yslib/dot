mod support;

use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::sync::atomic::{AtomicU64, Ordering};

use support::fixture;

static NEXT_MANIFEST: AtomicU64 = AtomicU64::new(0);
static NEXT_EXACT_WORKSPACE: AtomicU64 = AtomicU64::new(0);

struct TempManifest(PathBuf);

impl TempManifest {
    fn write(contents: &str) -> Self {
        let sequence = NEXT_MANIFEST.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "dot-dry-run-command-{}-{sequence}.toml",
            process::id()
        ));
        fs::write(&path, contents).expect("test manifest should be written");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempManifest {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

struct ExactWorkspace {
    directory: PathBuf,
    manifest: PathBuf,
}

impl ExactWorkspace {
    fn new() -> Self {
        let sequence = NEXT_EXACT_WORKSPACE.fetch_add(1, Ordering::Relaxed);
        let directory = env::temp_dir().join(format!(
            "dot-dry-run-command-exact-{}-{sequence}",
            process::id()
        ));
        fs::create_dir(&directory).expect("exact-selection workspace should be created");
        fs::write(directory.join("selected-source.txt"), "selected")
            .expect("selected link source should be written");
        fs::write(directory.join("unselected-source.txt"), "unselected")
            .expect("unselected link source should be written");
        let manifest = directory.join("dot.toml");
        let contents = fixture::read("selection/valid-exact-command-template.toml")
            .replace("__OS__", env::consts::OS)
            .replace("__PROGRAM__", &helper_program_toml());
        fs::write(&manifest, contents).expect("exact-selection manifest should be written");
        Self {
            directory,
            manifest,
        }
    }

    fn events(&self) -> PathBuf {
        self.directory.join("events")
    }

    fn selected_link(&self) -> PathBuf {
        self.directory.join("selected-linked.txt")
    }

    fn unselected_link(&self) -> PathBuf {
        self.directory.join("unselected-linked.txt")
    }

    fn recorded_events(&self) -> Vec<String> {
        fs::read_to_string(self.events())
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

impl Drop for ExactWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn helper_program_toml() -> String {
    format!(
        "{:?}",
        env::current_exe()
            .expect("test executable should have a path")
            .to_string_lossy()
    )
}

fn run_exact_command(
    workspace: &ExactWorkspace,
    leaf: &str,
    selectors: &[&str],
) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_dot"));
    command
        .arg("--config")
        .arg(&workspace.manifest)
        .arg(leaf)
        .env_remove("DOT_INTENTIONALLY_MISSING");
    for selector in selectors {
        command.args(["--job", selector]);
    }
    command.output().expect("dot command should start")
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

fn assert_no_exact_side_effects(workspace: &ExactWorkspace) {
    assert!(
        !workspace.events().exists(),
        "dry-run or an atomic planning failure must not create an event log"
    );
    assert!(
        !workspace.selected_link().exists(),
        "dry-run or an atomic planning failure must not create the selected link"
    );
    assert!(
        !workspace.unselected_link().exists(),
        "dry-run or an atomic planning failure must not create the unselected link"
    );
}

#[test]
fn exact_dry_run_matches_apply_without_side_effects_in_stable_plan_order() {
    let expected = [
        "provider:selected-provider",
        "package:selected",
        "link:selected-link",
    ];
    let selectors = ["package:selected", "link:selected-link"];
    let workspace = ExactWorkspace::new();
    let dry_run = run_exact_command(&workspace, "dry-run", &selectors);
    let dry_run_stdout = String::from_utf8(dry_run.stdout).expect("dry-run report should be UTF-8");

    assert!(
        dry_run.status.success(),
        "stdout:\n{dry_run_stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&dry_run.stderr)
    );
    assert_eq!(
        report_job_rows(&dry_run_stdout),
        expected,
        "{dry_run_stdout}"
    );
    assert!(
        dry_run_stdout.contains("PLANNED · 3 items · 1 provider · 1 package · 0 actions · 1 link"),
        "{dry_run_stdout}"
    );
    assert_no_exact_side_effects(&workspace);

    let apply = run_exact_command(&workspace, "apply", &selectors);
    let apply_stdout = String::from_utf8(apply.stdout).expect("apply report should be UTF-8");
    assert!(
        apply.status.success(),
        "stdout:\n{apply_stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&apply.stderr)
    );
    assert_eq!(
        report_job_rows(&dry_run_stdout),
        report_job_rows(&apply_stdout),
        "dry-run:\n{dry_run_stdout}\napply:\n{apply_stdout}"
    );
    assert_eq!(
        workspace.recorded_events(),
        ["selected-provider-probe", "selected-provider-install"]
    );
    assert_eq!(
        fs::canonicalize(workspace.selected_link()).expect("selected link should resolve"),
        fs::canonicalize(workspace.directory.join("selected-source.txt"))
            .expect("selected source should resolve")
    );

    let reverse_workspace = ExactWorkspace::new();
    let reverse = run_exact_command(
        &reverse_workspace,
        "dry-run",
        &["link:selected-link", "package:selected"],
    );
    let reverse_stdout =
        String::from_utf8(reverse.stdout).expect("reverse dry-run report should be UTF-8");
    assert!(
        reverse.status.success(),
        "stdout:\n{reverse_stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&reverse.stderr)
    );
    assert_eq!(
        report_job_rows(&reverse_stdout),
        expected,
        "{reverse_stdout}"
    );
    assert_no_exact_side_effects(&reverse_workspace);
}

#[test]
fn exact_dry_run_rejects_an_unknown_selector_atomically() {
    let workspace = ExactWorkspace::new();
    let output = run_exact_command(
        &workspace,
        "dry-run",
        &["package:selected", "link:selected-link", "action:unknown"],
    );

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty(), "{:?}", output.stdout);
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("unknown action job `unknown`"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_no_exact_side_effects(&workspace);
}

#[test]
fn exact_dry_run_rejects_a_selected_runtime_failure_atomically() {
    let workspace = ExactWorkspace::new();
    let output = run_exact_command(
        &workspace,
        "dry-run",
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
    assert_no_exact_side_effects(&workspace);
}

#[test]
fn dry_run_prints_the_resolved_plan_without_executing_or_inspecting() {
    let contents = fixture::read("dry-run/valid-command-plan-template.toml")
        .replace("__OS__", env::consts::OS);
    let manifest = TempManifest::write(&contents);

    let output = Command::new(env!("CARGO_BIN_EXE_dot"))
        .args(["dry-run", "--config"])
        .arg(manifest.path())
        .output()
        .expect("dot should start");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("dot dry-run · target=current"), "{stdout}");
    assert!(stdout.contains("│ provider ┆ system"), "{stdout}");
    assert!(stdout.contains("│ package  ┆ alpha"), "{stdout}");
    assert!(stdout.contains("│ package  ┆ manual"), "{stdout}");
    assert!(stdout.contains("│ action   ┆ configure"), "{stdout}");
    assert!(stdout.contains("│ link     ┆ missing"), "{stdout}");
    assert!(stdout.contains("PLANNED · 5 items"), "{stdout}");
    assert!(!stdout.contains("Dispatch {"), "{stdout}");
}

#[cfg(feature = "dev-platform-override")]
#[test]
fn dry_run_selects_against_the_injected_platform() {
    let contents = fixture::read("dry-run/valid-injected-platform.toml");
    let manifest = TempManifest::write(&contents);

    let output = Command::new(env!("CARGO_BIN_EXE_dot"))
        .args([
            "dry-run",
            "--platform",
            r#"{ os = "windows", arch = "x86_64" }"#,
            "--config",
        ])
        .arg(manifest.path())
        .output()
        .expect("dot should start");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "stderr:\n{stderr}",);
    assert!(
        stdout.contains("dot dry-run · target=simulated"),
        "{stdout}"
    );
    assert!(stdout.contains("platform=windows/x86_64"), "{stdout}");
    assert!(stderr.contains("warning"), "{stderr}");
    assert!(stderr.contains("XDG paths"), "{stderr}");
    assert!(stderr.contains("host"), "{stderr}");
}

#[test]
fn helper_process() {
    let Ok(mode) = env::var("DOT_EXACT_HELPER") else {
        return;
    };
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
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(events)
        .expect("event log should open");
    writeln!(file, "{mode}").expect("event should be recorded");
}

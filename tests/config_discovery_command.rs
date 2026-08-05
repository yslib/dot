//! End-to-end configuration discovery behavior.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{self, Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_WORKSPACE: AtomicU64 = AtomicU64::new(0);

const SIDE_EFFECT_FREE_MANIFEST: &str = r#"[targets.current]
platform = { os = ["linux", "macos", "windows"] }
"#;

struct TempWorkspace {
    root: PathBuf,
    cwd: PathBuf,
    #[cfg(unix)]
    home: PathBuf,
}

impl TempWorkspace {
    fn new() -> Self {
        let sequence = NEXT_WORKSPACE.fetch_add(1, Ordering::Relaxed);
        let root = env::temp_dir().join(format!(
            "dot-config-discovery-command-{}-{sequence}",
            process::id()
        ));
        let cwd = root.join("cwd");
        let home = root.join("home");
        fs::create_dir_all(&cwd).expect("temporary working directory should be created");
        fs::create_dir(&home).expect("temporary home directory should be created");
        let root =
            fs::canonicalize(root).expect("temporary workspace should have an absolute path");
        let cwd = root.join("cwd");
        #[cfg(unix)]
        let home = root.join("home");
        Self {
            root,
            cwd,
            #[cfg(unix)]
            home,
        }
    }

    fn write_local(&self, contents: &str) {
        fs::write(self.cwd.join(".dot.toml"), contents)
            .expect("local test manifest should be written");
    }

    #[cfg(unix)]
    fn write_legacy_local(&self, contents: &str) {
        fs::write(self.cwd.join("dot.toml"), contents)
            .expect("legacy local test manifest should be written");
    }

    #[cfg(unix)]
    fn write_user(&self, contents: &str) {
        let directory = self.home.join(".config").join("dot");
        fs::create_dir_all(&directory).expect("user configuration directory should be created");
        fs::write(directory.join(".dot.toml"), contents)
            .expect("user test manifest should be written");
    }

    fn command(&self, args: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_dot"));
        command.args(args).current_dir(&self.cwd);
        #[cfg(unix)]
        command.env("HOME", &self.home);
        command.output().expect("dot should start")
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn assert_success(args: &[&str], output: &Output) {
    assert!(
        output.status.success(),
        "command arguments: {args:?}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn every_operation_uses_the_local_default_manifest() {
    let workspace = TempWorkspace::new();
    workspace.write_local(SIDE_EFFECT_FREE_MANIFEST);

    for args in [
        &["apply"][..],
        &["dry-run"][..],
        &["check", "providers"][..],
        &["list", "targets"][..],
        &["list", "profiles"][..],
        &["list", "jobs"][..],
    ] {
        let output = workspace.command(args);
        assert_success(args, &output);
    }
}

#[cfg(unix)]
#[test]
fn missing_local_manifest_uses_the_user_manifest() {
    let workspace = TempWorkspace::new();
    workspace.write_user(SIDE_EFFECT_FREE_MANIFEST);

    let args = ["list", "targets"];
    let output = workspace.command(&args);
    assert_success(&args, &output);

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("current\tcompatible"), "{stdout}");
}

#[cfg(unix)]
#[test]
fn invalid_local_manifest_does_not_fall_back_to_valid_user_manifest() {
    let workspace = TempWorkspace::new();
    workspace.write_local("[targets");
    workspace.write_user(SIDE_EFFECT_FREE_MANIFEST);

    let output = workspace.command(&["list", "targets"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let local = workspace.cwd.join(".dot.toml");

    assert!(!output.status.success(), "stderr:\n{stderr}");
    assert!(local.is_absolute(), "{}", local.display());
    assert!(
        stderr.contains(&format!("`{}`", local.display())),
        "{stderr}"
    );
    assert!(stderr.contains("failed to parse configuration"), "{stderr}");
}

#[cfg(unix)]
#[test]
fn legacy_dot_toml_is_not_discovered() {
    let workspace = TempWorkspace::new();
    workspace.write_legacy_local(SIDE_EFFECT_FREE_MANIFEST);

    let output = workspace.command(&["list", "targets"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let local_candidate = workspace.cwd.join(".dot.toml").display().to_string();
    let user_candidate = workspace
        .home
        .join(".config")
        .join("dot")
        .join(".dot.toml")
        .display()
        .to_string();

    assert!(!output.status.success(), "stderr:\n{stderr}");
    assert!(stderr.contains("configuration not found"), "{stderr}");
    let local_offset = stderr
        .find(&local_candidate)
        .unwrap_or_else(|| panic!("missing local candidate `{local_candidate}`:\n{stderr}"));
    let user_offset = stderr
        .find(&user_candidate)
        .unwrap_or_else(|| panic!("missing user candidate `{user_candidate}`:\n{stderr}"));
    assert!(
        local_offset < user_offset,
        "local candidate must precede user candidate:\n{stderr}"
    );
}

#[cfg(all(feature = "dev-platform-override", unix))]
#[test]
fn platform_override_does_not_change_the_user_fallback_path() {
    let workspace = TempWorkspace::new();
    workspace.write_user(SIDE_EFFECT_FREE_MANIFEST);

    let args = [
        "--platform",
        r#"{ os = "windows", arch = "x86_64" }"#,
        "list",
        "targets",
        "--all",
    ];
    let output = workspace.command(&args);
    assert_success(&args, &output);

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("current\tcompatible"), "{stdout}");
}

//! End-to-end local configuration path behavior.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_WORKSPACE: AtomicU64 = AtomicU64::new(0);

const PATH_MANIFEST: &str = r#"[targets.current]
platform = { os = ["linux", "macos", "windows"] }

[targets.current.actions.entry]
source = "https://example.com/entry"
target = "${dot:config_dir}/entry.txt"

[targets.current.actions.real]
source = "https://example.com/real"
target = "${dot:real_config_dir}/real.txt"
"#;

struct TempWorkspace {
    root: PathBuf,
}

impl TempWorkspace {
    fn new() -> Self {
        let sequence = NEXT_WORKSPACE.fetch_add(1, Ordering::Relaxed);
        let temp_root = if cfg!(unix) {
            PathBuf::from("/tmp")
        } else {
            env::temp_dir()
        };
        let root = temp_root.join(format!(
            "dot-config-path-command-{}-{sequence}",
            process::id()
        ));
        fs::create_dir(&root).expect("temporary workspace should be created");
        let root = fs::canonicalize(root).expect("temporary workspace should canonicalize");
        Self { root }
    }

    fn directory(&self, name: &str) -> PathBuf {
        let directory = self.root.join(name);
        fs::create_dir(&directory).expect("test directory should be created");
        directory
    }

    fn run(&self, source: impl AsRef<Path>) -> Output {
        Command::new(env!("CARGO_BIN_EXE_dot"))
            .arg("--config")
            .arg(source.as_ref())
            .arg("dry-run")
            .current_dir(&self.root)
            .output()
            .expect("dot should start")
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn relative_config_paths_produce_absolute_protocol_directories() {
    let workspace = TempWorkspace::new();
    let config_dir = workspace.directory("config");
    fs::write(config_dir.join(".dot.toml"), PATH_MANIFEST)
        .expect("test manifest should be written");

    let output = workspace.run(Path::new("config").join(".dot.toml"));
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "stdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains(config_dir.join("entry.txt").to_string_lossy().as_ref()),
        "{stdout}"
    );
    assert!(
        stdout.contains(config_dir.join("real.txt").to_string_lossy().as_ref()),
        "{stdout}"
    );
}

#[cfg(unix)]
#[test]
fn symlinked_config_keeps_entry_and_real_directories_distinct() {
    let workspace = TempWorkspace::new();
    let entry_dir = workspace.directory("entry");
    let real_dir = workspace.directory("real");
    let real_manifest = real_dir.join(".dot.toml");
    fs::write(&real_manifest, PATH_MANIFEST).expect("real manifest should be written");
    std::os::unix::fs::symlink(&real_manifest, entry_dir.join(".dot.toml"))
        .expect("manifest symlink should be created");

    let output = workspace.run(Path::new("entry").join(".dot.toml"));
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "stdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains(entry_dir.join("entry.txt").to_string_lossy().as_ref()),
        "{stdout}"
    );
    assert!(
        stdout.contains(real_dir.join("real.txt").to_string_lossy().as_ref()),
        "{stdout}"
    );
}

#[test]
fn missing_config_error_names_the_requested_absolute_path() {
    let workspace = TempWorkspace::new();
    let relative = Path::new("missing").join(".dot.toml");
    let expected = workspace.root.join(&relative);

    let output = workspace.run(&relative);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(
        stderr.contains("failed to canonicalize configuration"),
        "{stderr}"
    );
    assert!(
        stderr.contains(expected.to_string_lossy().as_ref()),
        "{stderr}"
    );
}

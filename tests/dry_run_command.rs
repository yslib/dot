mod support;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::sync::atomic::{AtomicU64, Ordering};

use support::fixture;

static NEXT_MANIFEST: AtomicU64 = AtomicU64::new(0);

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

#[test]
fn dry_run_prints_the_resolved_plan_without_executing_or_inspecting() {
    let contents = fixture::read("dry-run/valid-command-plan-template.toml")
        .replace("__OS__", env::consts::OS);
    let manifest = TempManifest::write(&contents);

    let output = Command::new(env!("CARGO_BIN_EXE_dot"))
        .args(["--dry-run", "--config"])
        .arg(manifest.path())
        .output()
        .expect("dot should start");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("target: current"), "{stdout}");
    assert!(stdout.contains("providers:\n  system"), "{stdout}");
    assert!(stdout.contains("packages: [\"alpha\"]"), "{stdout}");
    assert!(stdout.contains("manual packages:\n  manual"), "{stdout}");
    assert!(stdout.contains("actions:\n  configure"), "{stdout}");
    assert!(stdout.contains("links:\n  missing:"), "{stdout}");
    assert!(!stdout.contains("Dispatch {"), "{stdout}");
}

#[cfg(feature = "dev-platform-override")]
#[test]
fn dry_run_selects_against_the_injected_platform() {
    let contents = fixture::read("dry-run/valid-injected-platform.toml");
    let manifest = TempManifest::write(&contents);

    let output = Command::new(env!("CARGO_BIN_EXE_dot"))
        .args([
            "--dry-run",
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
    assert!(stdout.contains("target: simulated"), "{stdout}");
    assert!(stdout.contains("platform: windows/x86_64"), "{stdout}");
    assert!(stderr.contains("warning"), "{stderr}");
    assert!(stderr.contains("XDG paths"), "{stderr}");
    assert!(stderr.contains("host"), "{stderr}");
}

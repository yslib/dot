use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_MANIFEST: AtomicU64 = AtomicU64::new(0);

struct TempManifest(PathBuf);

impl TempManifest {
    fn write(contents: &str) -> Self {
        let sequence = NEXT_MANIFEST.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "dot-check-command-{}-{sequence}.toml",
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

fn helper_program_toml() -> String {
    format!(
        "{:?}",
        env::current_exe()
            .expect("test executable should have a path")
            .to_string_lossy()
    )
}

fn provider_manifest() -> TempManifest {
    let contents = r#"
        [targets.current]
        platform = { os = "__OS__" }

        [targets.current.providers.a-ready]
        activate = { variables = { PROVIDER_VALUE = "${dot:config_dir}/ready" } }
        probe = { program = __PROGRAM__, args = ["--exact", "helper_process", "--nocapture"], env = { variables = { DOT_CHECK_COMMAND_HELPER = "ready" } } }
        install = { program = "unused-install" }

        [targets.current.providers.b-not-ready]
        activate = { variables = { PROVIDER_VALUE = "${dot:config_dir}/not-ready" } }
        probe = { program = __PROGRAM__, args = ["--exact", "helper_process", "--nocapture"], env = { variables = { DOT_CHECK_COMMAND_HELPER = "not-ready" } } }
        install = { program = "unused-install" }
    "#
    .replace("__OS__", env::consts::OS)
    .replace("__PROGRAM__", &helper_program_toml());
    TempManifest::write(&contents)
}

#[test]
fn check_providers_runs_the_selected_manifest_and_sets_the_exit_code() {
    let manifest = provider_manifest();

    let output = Command::new(env!("CARGO_BIN_EXE_dot"))
        .args(["check", "providers", "--config"])
        .arg(manifest.path())
        .output()
        .expect("dot should start");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(1), "stdout:\n{stdout}");
    assert!(stdout.contains("READY a-ready (exit 0)"), "{stdout}");
    assert!(
        stdout.contains("NOT_READY b-not-ready (exit 23)"),
        "{stdout}"
    );
    assert!(stdout.contains("/ready"), "{stdout}");
    assert!(stdout.contains("/not-ready"), "{stdout}");
}

#[test]
fn check_providers_reports_an_empty_manifest_as_ready() {
    let contents = r#"
        [targets.current]
        platform = { os = "__OS__" }
    "#
    .replace("__OS__", env::consts::OS);
    let manifest = TempManifest::write(&contents);

    let output = Command::new(env!("CARGO_BIN_EXE_dot"))
        .args(["check", "providers", "--config"])
        .arg(manifest.path())
        .output()
        .expect("dot should start");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "No providers.\n");
}

#[test]
fn helper_process() {
    let Ok(mode) = env::var("DOT_CHECK_COMMAND_HELPER") else {
        return;
    };
    let value = env::var("PROVIDER_VALUE").expect("provider activate should set a value");
    println!("value={value}");

    if mode == "not-ready" {
        std::io::stderr()
            .write_all(b"provider is not ready")
            .expect("helper should write stderr");
        process::exit(23);
    }
}

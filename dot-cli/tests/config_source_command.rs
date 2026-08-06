//! End-to-end Git configuration source behavior.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use url::Url;

static NEXT_WORKSPACE: AtomicU64 = AtomicU64::new(0);

const MANIFEST: &str = r#"[targets.from-git]
platform = { os = ["linux", "macos", "windows"] }
"#;

struct TempRepositories {
    root: PathBuf,
    cwd: PathBuf,
}

impl TempRepositories {
    fn new() -> Self {
        let sequence = NEXT_WORKSPACE.fetch_add(1, Ordering::Relaxed);
        let root = env::temp_dir().join(format!(
            "dot-config-source-command-{}-{sequence}",
            process::id()
        ));
        let cwd = root.join("cwd");
        fs::create_dir_all(&cwd).expect("temporary working directory should be created");
        let root = fs::canonicalize(root).expect("temporary root should canonicalize");
        let cwd = root.join("cwd");
        Self { root, cwd }
    }

    fn repository(&self, name: &str) -> String {
        let source = self.root.join(name);
        fs::create_dir(&source).expect("source repository directory should be created");
        git(&source, &["init"]);
        git(&source, &["config", "user.name", "dot tests"]);
        git(
            &source,
            &["config", "user.email", "dot-tests@example.invalid"],
        );
        fs::write(source.join(".dot.toml"), MANIFEST)
            .expect("repository manifest should be written");
        git(&source, &["add", ".dot.toml"]);
        git(&source, &["commit", "-m", "add configuration"]);

        let source = fs::canonicalize(source).expect("source repository should canonicalize");
        Url::from_file_path(source)
            .expect("canonical source path should convert to a file URL")
            .into()
    }

    fn dot(&self, repository: &str, worktree: &Path) -> Output {
        self.dot_command(repository, worktree)
            .output()
            .expect("dot should start")
    }

    fn dot_with_global_origin(
        &self,
        repository: &str,
        worktree: &Path,
        global_origin: &str,
    ) -> Output {
        let global_config = self.root.join("misleading-global.gitconfig");
        fs::write(
            &global_config,
            format!("[remote \"origin\"]\n\turl = {global_origin}\n"),
        )
        .expect("misleading global Git configuration should be written");
        self.dot_command(repository, worktree)
            .env("GIT_CONFIG_GLOBAL", global_config)
            .output()
            .expect("dot should start")
    }

    fn dot_command(&self, repository: &str, worktree: &Path) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_dot"));
        command
            .args([
                "--git",
                repository,
                "--git-worktree",
                worktree
                    .to_str()
                    .expect("test worktree path should be Unicode"),
                "list",
                "targets",
                "--all",
            ])
            .current_dir(&self.cwd)
            .env("GIT_DIR", self.root.join("misleading-git-dir"))
            .env("GIT_WORK_TREE", self.root.join("misleading-work-tree"));
        command
    }
}

impl Drop for TempRepositories {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn git(current_dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(current_dir)
        .output()
        .expect("installed Git should start");
    assert!(
        output.status.success(),
        "git {args:?} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_lists_git_target(output: &Output) {
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("from-git\tcompatible"), "{stdout}");
}

#[test]
fn clones_then_reuses_a_git_worktree_with_git_location_environment_isolated() {
    let repositories = TempRepositories::new();
    let repository = repositories.repository("source");
    let requested_worktree = Path::new("persistent-checkout");
    let absolute_worktree = repositories.cwd.join(requested_worktree);

    let first = repositories.dot(&repository, requested_worktree);
    assert_lists_git_target(&first);
    assert!(absolute_worktree.join(".git").exists());

    let second = repositories.dot_with_global_origin(
        &repository,
        requested_worktree,
        "file:///misleading-global",
    );
    assert_lists_git_target(&second);
}

#[test]
fn rejects_an_existing_worktree_whose_origin_differs_from_git() {
    let repositories = TempRepositories::new();
    let actual = repositories.repository("actual");
    let expected = repositories.repository("expected");
    let worktree = Path::new("persistent-checkout");
    assert_lists_git_target(&repositories.dot(&actual, worktree));

    let output = repositories.dot(&expected, worktree);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let absolute_worktree = repositories.cwd.join(worktree);

    assert!(!output.status.success(), "{stderr}");
    assert!(
        stderr.contains(absolute_worktree.to_string_lossy().as_ref()),
        "{stderr}"
    );
    assert!(stderr.contains(&format!("actual: `{actual}`")), "{stderr}");
    assert!(
        stderr.contains(&format!("expected: `{expected}`")),
        "{stderr}"
    );
}

use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dot"))
        .args(args)
        .output()
        .expect("dot should start")
}

#[test]
fn requires_an_explicit_root_subcommand() {
    let output = run(&[]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(output.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("Usage: dot"), "{stderr}");
}

#[test]
fn rejects_the_old_implicit_apply_and_global_dry_run_syntax() {
    for args in [&["--config", "config/dev.toml"][..], &["--dry-run"][..]] {
        let output = run(args);

        assert_eq!(output.status.code(), Some(2));
    }
}

#[test]
fn duplicate_job_errors_use_the_leaf_clap_format() {
    for leaf in ["apply", "dry-run"] {
        let output = run(&[leaf, "--job", "package:ripgrep", "--job", "package:ripgrep"]);
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert_eq!(output.status.code(), Some(2), "{stderr}");
        assert!(
            stderr.starts_with(
                "error: job selector `package:ripgrep` was supplied more than once\n\n"
            ),
            "{stderr:?}"
        );
        assert!(stderr.contains(&format!("Usage: dot {leaf} [OPTIONS]\n")));
    }
}

#[test]
fn rejects_invalid_and_misplaced_selectors() {
    for args in [
        &["apply", "--target", "desktop/laptop"][..],
        &["apply", "--profile", ""][..],
        &["apply", "--job", "provider:brew"][..],
        &["list", "targets", "--profile", "desktop"][..],
        &["list", "profiles", "--job", "link:config"][..],
        &["list", "jobs", "--all"][..],
    ] {
        let output = run(args);

        assert_eq!(output.status.code(), Some(2), "args: {args:?}");
    }
}

#[test]
fn requires_complete_nested_commands() {
    for args in [&["check"][..], &["list"][..]] {
        let output = run(args);

        assert_eq!(output.status.code(), Some(2));
    }
}

#[cfg(not(feature = "dev-platform-override"))]
#[test]
fn production_cli_does_not_expose_platform_override() {
    let output = run(&[
        "dry-run",
        "--platform",
        r#"{ os = "windows", arch = "x86_64" }"#,
    ]);

    assert_eq!(output.status.code(), Some(2));
}

#[cfg(feature = "dev-platform-override")]
#[test]
fn development_help_states_the_platform_override_boundary() {
    let output = run(&["--help"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success(), "{stdout}");
    assert!(stdout.contains("compatibility"), "{stdout}");
    assert!(stdout.contains("host"), "{stdout}");
}

#[test]
fn exposes_standard_help_version_and_config_discovery_documentation() {
    let help = run(&["--help"]);
    let version = run(&["--version"]);
    let stdout = String::from_utf8_lossy(&help.stdout);
    let normalized = stdout.split_whitespace().collect::<Vec<_>>().join(" ");

    assert!(help.status.success(), "{stdout}");
    assert!(version.status.success());
    assert!(normalized.contains("--config <SOURCE>"), "{stdout}");
    assert!(stdout.contains("./.dot.toml"), "{stdout}");
    assert!(normalized.contains("user fallback"), "{stdout}");
}

#[test]
fn rejects_unsupported_config_source_protocols_during_cli_parsing() {
    let output = run(&["--config", "http://example.com/dot.toml", "list", "targets"]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(output.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("protocol `http`"), "{stderr}");
}

#[test]
fn requires_a_complete_non_conflicting_git_source_group() {
    for args in [
        &["--git", "file:///source.git", "list", "targets"][..],
        &["--git-worktree", "checkout", "list", "targets"][..],
        &[
            "--config",
            "config.toml",
            "--git",
            "file:///source.git",
            "--git-worktree",
            "checkout",
            "list",
            "targets",
        ][..],
    ] {
        let output = run(args);
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert_eq!(output.status.code(), Some(2), "args: {args:?}\n{stderr}");
    }
}

#[test]
fn help_documents_git_source_values_without_reusing_target() {
    let output = run(&["apply", "--help"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let normalized = stdout.split_whitespace().collect::<Vec<_>>().join(" ");

    assert!(output.status.success(), "{stdout}");
    assert!(normalized.contains("--git <REPOSITORY>"), "{stdout}");
    assert!(normalized.contains("--git-worktree <PATH>"), "{stdout}");
    assert!(normalized.contains("--target <TARGET>"), "{stdout}");
}

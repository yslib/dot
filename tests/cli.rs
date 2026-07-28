use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;

use clap::error::ErrorKind;
use dot::app::{Dispatch, ExecutionRequest, Operation, ProfileSelection, ScopeSelection};
use dot::cli;
use dot::config::ConfigRequest;
use dot::job::{JobSelection, JobSelector};
#[cfg(feature = "dev-platform-override")]
use dot::platform::PlatformInfo;
use dot::schema::SelectorIdentifier;

fn identifier(value: &str) -> SelectorIdentifier {
    SelectorIdentifier::new(value).expect("test selector should be valid")
}

fn root_scope(target: Option<&str>) -> ScopeSelection {
    ScopeSelection {
        target: target.map(identifier),
        profile: ProfileSelection::Root,
    }
}

#[test]
fn requires_an_explicit_root_subcommand() {
    let error = cli::try_parse_from(["dot"]).expect_err("dot alone should show help");

    assert_eq!(
        error.kind(),
        ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
    );
}

#[test]
fn rejects_the_old_implicit_apply_and_global_dry_run_syntax() {
    let implicit = cli::try_parse_from(["dot", "--config", "config/dev.toml"])
        .expect_err("implicit apply should be rejected");
    let dry_run =
        cli::try_parse_from(["dot", "--dry-run"]).expect_err("global dry-run should be rejected");

    assert_eq!(implicit.kind(), ErrorKind::MissingSubcommand);
    assert_eq!(dry_run.kind(), ErrorKind::UnknownArgument);
}

#[test]
fn parses_explicit_apply_with_root_scope_and_all_jobs() {
    let dispatch = cli::try_parse_from(["dot", "apply"]).expect("apply should parse");

    assert_eq!(
        dispatch,
        Dispatch {
            config: ConfigRequest::Discover,
            operation: Operation::Apply(ExecutionRequest {
                scope: root_scope(None),
                jobs: JobSelection::All,
            }),
            platform_override: None,
        }
    );
}

#[test]
fn parses_dry_run_selection_and_repeatable_jobs() {
    let dispatch = cli::try_parse_from([
        "dot",
        "dry-run",
        "--config",
        "config/dev.toml",
        "--target",
        "arch-personal",
        "--profile",
        "laptop",
        "--job",
        "link:config",
        "--job",
        "package:ripgrep",
    ])
    .expect("dry-run arguments should parse");

    assert_eq!(
        dispatch.config,
        ConfigRequest::Explicit(PathBuf::from("config/dev.toml"))
    );
    assert_eq!(
        dispatch.operation,
        Operation::DryRun(ExecutionRequest {
            scope: ScopeSelection {
                target: Some(identifier("arch-personal")),
                profile: ProfileSelection::Named(identifier("laptop")),
            },
            jobs: JobSelection::Only(BTreeSet::from([
                "link:config".parse::<JobSelector>().unwrap(),
                "package:ripgrep".parse::<JobSelector>().unwrap(),
            ])),
        })
    );
}

#[test]
fn normalizes_absent_and_explicit_root_profiles() {
    for args in [
        vec!["dot", "apply"],
        vec!["dot", "apply", "--profile", "@root"],
    ] {
        let dispatch = cli::try_parse_from(args).expect("root profile should parse");
        let Operation::Apply(request) = dispatch.operation else {
            panic!("expected apply request");
        };
        assert_eq!(request.scope.profile, ProfileSelection::Root);
    }
}

#[test]
fn rejects_duplicate_job_selectors_during_dispatch_conversion() {
    let error = cli::try_parse_from([
        "dot",
        "apply",
        "--job",
        "package:ripgrep",
        "--job",
        "package:ripgrep",
    ])
    .expect_err("duplicate jobs should be rejected");

    assert_eq!(error.kind(), ErrorKind::ArgumentConflict);
    assert!(error.to_string().contains("package:ripgrep"));
}

#[test]
fn duplicate_job_errors_use_the_leaf_clap_stderr_format() {
    for leaf in ["apply", "dry-run"] {
        let output = Command::new(env!("CARGO_BIN_EXE_dot"))
            .args([leaf, "--job", "package:ripgrep", "--job", "package:ripgrep"])
            .output()
            .expect("dot should start");
        let stderr = String::from_utf8(output.stderr).expect("clap stderr should be UTF-8");

        assert_eq!(output.status.code(), Some(2), "{stderr}");
        assert!(output.stdout.is_empty());
        assert!(
            stderr.starts_with(
                "error: job selector `package:ripgrep` was supplied more than once\n\n"
            ),
            "{stderr:?}"
        );
        assert!(
            stderr.contains(&format!("Usage: dot {leaf} [OPTIONS]\n")),
            "{stderr:?}"
        );
        assert!(
            stderr.ends_with("For more information, try '--help'.\n"),
            "{stderr:?}"
        );
        assert!(stderr.ends_with('\n'), "{stderr:?}");
    }
}

#[test]
fn profile_selections_parse_and_display_with_one_canonical_spelling() {
    for (input, canonical) in [("@root", "@root"), ("laptop", "laptop")] {
        let selection = ProfileSelection::from_cli(Some(input)).expect("profile should parse");

        assert_eq!(selection.to_string(), canonical);
        assert_eq!(
            ProfileSelection::from_cli(Some(&selection.to_string())).unwrap(),
            selection
        );
    }

    assert_eq!(
        ProfileSelection::from_cli(None).unwrap().to_string(),
        "@root"
    );
}

#[test]
fn rejects_bare_provider_and_unknown_job_selectors() {
    for selector in ["ripgrep", "provider:brew", "service:ssh"] {
        let error = cli::try_parse_from(["dot", "apply", "--job", selector])
            .expect_err("invalid job selector should be rejected");

        assert_eq!(error.kind(), ErrorKind::ValueValidation);
    }
}

#[test]
fn parses_check_and_list_operations() {
    let check = cli::try_parse_from([
        "dot",
        "check",
        "providers",
        "-c",
        "config/dev.toml",
        "-t",
        "arch-personal",
        "-p",
        "laptop",
    ])
    .expect("check providers should parse");
    assert_eq!(
        check.config,
        ConfigRequest::Explicit(PathBuf::from("config/dev.toml"))
    );
    assert_eq!(
        check.operation,
        Operation::CheckProviders(ScopeSelection {
            target: Some(identifier("arch-personal")),
            profile: ProfileSelection::Named(identifier("laptop")),
        })
    );

    let targets =
        cli::try_parse_from(["dot", "list", "targets", "--all"]).expect("targets should parse");
    assert_eq!(targets.operation, Operation::ListTargets { all: true });

    let profiles = cli::try_parse_from(["dot", "list", "profiles", "-t", "never"])
        .expect("profiles should parse");
    assert_eq!(
        profiles.operation,
        Operation::ListProfiles {
            target: Some(identifier("never")),
        }
    );

    let jobs = cli::try_parse_from([
        "dot",
        "list",
        "jobs",
        "--target",
        "never",
        "--profile",
        "@root",
    ])
    .expect("jobs should parse");
    assert_eq!(
        jobs.operation,
        Operation::ListJobs(root_scope(Some("never")))
    );
}

#[test]
fn structurally_rejects_options_on_commands_where_they_do_not_apply() {
    for args in [
        vec!["dot", "check", "providers", "--job", "package:ripgrep"],
        vec!["dot", "list", "targets", "--profile", "desktop"],
        vec!["dot", "list", "targets", "--target", "current"],
        vec!["dot", "list", "profiles", "--profile", "desktop"],
        vec!["dot", "list", "profiles", "--job", "link:config"],
        vec!["dot", "list", "jobs", "--all"],
    ] {
        let error = cli::try_parse_from(args).expect_err("misplaced option should be rejected");
        assert_eq!(error.kind(), ErrorKind::UnknownArgument);
    }
}

#[test]
fn requires_complete_nested_commands() {
    for args in [vec!["dot", "check"], vec!["dot", "list"]] {
        let error = cli::try_parse_from(args).expect_err("nested command should be required");
        assert_eq!(
            error.kind(),
            ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
        );
    }
}

#[test]
fn rejects_invalid_target_and_profile_identifiers() {
    for args in [
        vec!["dot", "apply", "--target", "desktop/laptop"],
        vec!["dot", "apply", "--profile", "desktop/laptop"],
        vec!["dot", "apply", "--profile", ""],
    ] {
        let error = cli::try_parse_from(args).expect_err("invalid selector should be rejected");
        assert_eq!(error.kind(), ErrorKind::ValueValidation);
    }
}

#[cfg(feature = "dev-platform-override")]
#[test]
fn parses_a_global_toml_platform_override() {
    let dispatch = cli::try_parse_from([
        "dot",
        "--platform",
        r#"{ os = "linux", arch = "x86_64", distro = "ubuntu", distro_family = ["debian", "linux"], environment = ["wsl", "container"] }"#,
        "dry-run",
    ])
    .expect("a complete TOML platform should parse");

    assert_eq!(
        dispatch.platform_override,
        Some(PlatformInfo {
            os: "linux".into(),
            arch: "x86_64".into(),
            distro: Some("ubuntu".into()),
            distro_families: ["debian", "linux"].into_iter().map(str::to_owned).collect(),
            environments: ["container", "wsl"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        })
    );
}

#[cfg(feature = "dev-platform-override")]
#[test]
fn defaults_an_injected_platform_to_native() {
    let dispatch = cli::try_parse_from([
        "dot",
        "list",
        "targets",
        "--platform",
        r#"{ os = "windows", arch = "x86_64" }"#,
    ])
    .expect("optional platform facts should have defaults");

    let platform = dispatch
        .platform_override
        .expect("the platform should be injected");
    assert_eq!(platform.os, "windows");
    assert_eq!(platform.arch, "x86_64");
    assert_eq!(platform.distro, None);
    assert!(platform.distro_families.is_empty());
    assert_eq!(
        platform.environments,
        ["native"].into_iter().map(str::to_owned).collect()
    );
}

#[cfg(feature = "dev-platform-override")]
#[test]
fn rejects_an_invalid_platform_override() {
    for platform in [
        r#"{ os = "windows" }"#,
        r#"{ os = "windows", arch = "x86_64", unknown = "value" }"#,
        r#"{ os = "", arch = "x86_64" }"#,
    ] {
        let error = cli::try_parse_from(["dot", "dry-run", "--platform", platform])
            .expect_err("an invalid platform should be rejected");

        assert_eq!(error.kind(), ErrorKind::ValueValidation);
    }
}

#[cfg(feature = "dev-platform-override")]
#[test]
fn development_help_states_the_platform_override_boundary() {
    let help = cli::try_parse_from(["dot", "--help"])
        .expect_err("help should exit early")
        .to_string();

    assert!(help.contains("compatibility"), "{help}");
    assert!(help.contains("host"), "{help}");
}

#[cfg(not(feature = "dev-platform-override"))]
#[test]
fn production_cli_does_not_expose_platform_override() {
    let error = cli::try_parse_from([
        "dot",
        "dry-run",
        "--platform",
        r#"{ os = "windows", arch = "x86_64" }"#,
    ])
    .expect_err("the production CLI must not expose the development option");

    assert_eq!(error.kind(), ErrorKind::UnknownArgument);
}

#[test]
fn exposes_standard_help_and_version_flags() {
    let help = cli::try_parse_from(["dot", "--help"]).expect_err("help exits early");
    let version = cli::try_parse_from(["dot", "--version"]).expect_err("version exits early");

    assert_eq!(help.kind(), ErrorKind::DisplayHelp);
    assert_eq!(version.kind(), ErrorKind::DisplayVersion);
}

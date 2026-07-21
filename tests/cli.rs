use std::path::PathBuf;

use clap::error::ErrorKind;
use dot::app::{Dispatch, Operation, Selection};
use dot::cli;
#[cfg(feature = "dev-platform-override")]
use dot::platform::PlatformInfo;

#[test]
fn defaults_to_apply_with_the_current_directory_dotfile() {
    let dispatch = cli::try_parse_from(["dot"]).expect("default invocation should parse");

    assert_eq!(
        dispatch,
        Dispatch {
            selection: Selection {
                config: PathBuf::from("dot.toml"),
                target: None,
                profile: None,
            },
            operation: Operation::Apply { dry_run: false },
            platform_override: None,
        }
    );
}

#[cfg(feature = "dev-platform-override")]
#[test]
fn parses_a_toml_platform_override() {
    let dispatch = cli::try_parse_from([
        "dot",
        "--dry-run",
        "--platform",
        r#"{ os = "linux", arch = "x86_64", distro = "ubuntu", distro_family = ["debian", "linux"], environment = ["wsl", "container"] }"#,
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
        "check",
        "providers",
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
        let error = cli::try_parse_from(["dot", "--dry-run", "--platform", platform])
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

    assert!(help.contains("target selection only"), "{help}");
    assert!(help.contains("XDG"), "{help}");
    assert!(help.contains("host"), "{help}");
}

#[cfg(not(feature = "dev-platform-override"))]
#[test]
fn production_cli_does_not_expose_platform_override() {
    let error = cli::try_parse_from([
        "dot",
        "--dry-run",
        "--platform",
        r#"{ os = "windows", arch = "x86_64" }"#,
    ])
    .expect_err("the production CLI must not expose the development option");

    assert_eq!(error.kind(), ErrorKind::UnknownArgument);
}

#[test]
fn parses_explicit_apply_selection_and_dry_run() {
    let dispatch = cli::try_parse_from([
        "dot",
        "--config",
        "config/dev.toml",
        "--target",
        "arch-personal",
        "--profile",
        "laptop",
        "--dry-run",
    ])
    .expect("apply arguments should parse");

    assert_eq!(dispatch.selection.config, PathBuf::from("config/dev.toml"));
    assert_eq!(dispatch.selection.target.as_deref(), Some("arch-personal"));
    assert_eq!(dispatch.selection.profile.as_deref(), Some("laptop"));
    assert_eq!(dispatch.operation, Operation::Apply { dry_run: true });
}

#[test]
fn parses_check_providers_with_global_options_after_the_subcommands() {
    let dispatch = cli::try_parse_from([
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
    .expect("check providers should accept global options");

    assert_eq!(dispatch.selection.config, PathBuf::from("config/dev.toml"));
    assert_eq!(dispatch.selection.target.as_deref(), Some("arch-personal"));
    assert_eq!(dispatch.selection.profile.as_deref(), Some("laptop"));
    assert_eq!(dispatch.operation, Operation::CheckProviders);
}

#[test]
fn requires_the_complete_check_providers_command() {
    let error =
        cli::try_parse_from(["dot", "check"]).expect_err("check without providers should fail");

    assert_eq!(
        error.kind(),
        ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
    );
}

#[test]
fn rejects_dry_run_with_check_providers() {
    let error = cli::try_parse_from(["dot", "--dry-run", "check", "providers"])
        .expect_err("dry-run must not modify check providers");

    assert_eq!(error.kind(), ErrorKind::ArgumentConflict);
    assert!(cli::try_parse_from(["dot", "check", "providers", "--dry-run"]).is_err());
}

#[test]
fn rejects_profile_paths_and_empty_profile_names() {
    for profile in ["", "/desktop", "desktop/laptop", "desktop/"] {
        let error = cli::try_parse_from(["dot", "--profile", profile])
            .expect_err("profile must be one node name");

        assert_eq!(error.kind(), ErrorKind::ValueValidation);
    }
}

#[test]
fn exposes_standard_help_and_version_flags() {
    let help = cli::try_parse_from(["dot", "--help"]).expect_err("help exits early");
    let version = cli::try_parse_from(["dot", "--version"]).expect_err("version exits early");

    assert_eq!(help.kind(), ErrorKind::DisplayHelp);
    assert_eq!(version.kind(), ErrorKind::DisplayVersion);
}

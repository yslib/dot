use std::path::PathBuf;

use clap::error::ErrorKind;
use dot::cli::{Dispatch, Operation, Selection};

#[test]
fn defaults_to_apply_with_the_current_directory_dotfile() {
    let dispatch = Dispatch::try_parse_from(["dot"]).expect("default invocation should parse");

    assert_eq!(
        dispatch,
        Dispatch {
            selection: Selection {
                config: PathBuf::from("dot.toml"),
                target: None,
                profile: None,
            },
            operation: Operation::Apply { dry_run: false },
        }
    );
}

#[test]
fn parses_explicit_apply_selection_and_dry_run() {
    let dispatch = Dispatch::try_parse_from([
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
    let dispatch = Dispatch::try_parse_from([
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
    let error = Dispatch::try_parse_from(["dot", "check"])
        .expect_err("check without providers should fail");

    assert_eq!(
        error.kind(),
        ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
    );
}

#[test]
fn rejects_dry_run_with_check_providers() {
    let error = Dispatch::try_parse_from(["dot", "--dry-run", "check", "providers"])
        .expect_err("dry-run must not modify check providers");

    assert_eq!(error.kind(), ErrorKind::ArgumentConflict);
    assert!(Dispatch::try_parse_from(["dot", "check", "providers", "--dry-run"]).is_err());
}

#[test]
fn rejects_profile_paths_and_empty_profile_names() {
    for profile in ["", "/desktop", "desktop/laptop", "desktop/"] {
        let error = Dispatch::try_parse_from(["dot", "--profile", profile])
            .expect_err("profile must be one node name");

        assert_eq!(error.kind(), ErrorKind::ValueValidation);
    }
}

#[test]
fn exposes_standard_help_and_version_flags() {
    let help = Dispatch::try_parse_from(["dot", "--help"]).expect_err("help exits early");
    let version = Dispatch::try_parse_from(["dot", "--version"]).expect_err("version exits early");

    assert_eq!(help.kind(), ErrorKind::DisplayHelp);
    assert_eq!(version.kind(), ErrorKind::DisplayVersion);
}

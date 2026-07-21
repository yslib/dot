use std::collections::BTreeSet;

use dot::manifest::{EffectiveManifest, ManifestError};
use dot::platform::PlatformInfo;
use dot::schema::{Config, Package};

const PROFILE_CONFIG: &str = r#"
    [targets.machine]
    platform = { os = "linux" }

    [targets.machine.providers.system]
    probe = { program = "system-probe" }
    install = { program = "system-install" }

    [targets.machine.packages]
    base = { provider = "system" }
    replace-me = { provider = "system" }

    [targets.machine.links.shared]
    source = "root-source"
    target = "/root-target"

    [targets.machine.actions.configure]
    check = { program = "root-check" }
    exec = { program = "root-exec" }

    [targets.machine.profiles.desktop.providers.desktop]
    probe = { program = "desktop-probe" }
    install = { program = "desktop-install" }

    [targets.machine.profiles.desktop.packages]
    desktop = { provider = "desktop" }
    replace-me = { provider = "desktop" }

    [targets.machine.profiles.desktop.links.shared]
    source = "desktop-source"
    target = "/desktop-target"

    [targets.machine.profiles.desktop.actions.configure]
    exec = { program = "desktop-exec" }

    [targets.machine.profiles.desktop.profiles.laptop.packages]
    laptop = { provider = "desktop" }

    [targets.machine.profiles.server.packages]
    server-only = { provider = "system" }
"#;

fn parse_config(input: &str) -> Config {
    toml::from_str(input).expect("test config should deserialize")
}

fn platform(os: &str) -> PlatformInfo {
    PlatformInfo {
        os: os.into(),
        arch: "x86_64".into(),
        distro: None,
        distro_families: BTreeSet::new(),
        environments: BTreeSet::from(["native".into()]),
    }
}

#[test]
fn selects_the_only_target_when_no_target_is_requested() {
    let config = parse_config(
        r#"
        [targets.only]
        platform = { os = "linux" }

        [targets.only.packages]
        git = { provider = "system" }
        "#,
    );

    let manifest = EffectiveManifest::select(&config, &platform("linux"), None, None)
        .expect("the only compatible target should be selected");

    assert_eq!(manifest.target(), "only");
    assert_eq!(manifest.profile(), None);
    assert!(manifest.packages().contains_key("git"));
}

#[test]
fn requires_a_target_when_the_config_contains_multiple_targets() {
    let config = parse_config(
        r#"
        [targets.first]
        platform = { os = "linux" }

        [targets.second]
        platform = { os = "linux" }
        "#,
    );

    let error = EffectiveManifest::select(&config, &platform("linux"), None, None)
        .expect_err("ambiguous target selection should fail");

    assert_eq!(
        error,
        ManifestError::TargetRequired {
            available: vec!["first".into(), "second".into()]
        }
    );
}

#[test]
fn reports_an_unknown_explicit_target() {
    let config = parse_config(
        r#"
        [targets.known]
        platform = { os = "linux" }
        "#,
    );

    let error = EffectiveManifest::select(&config, &platform("linux"), Some("missing"), None)
        .expect_err("unknown target should fail");

    assert_eq!(
        error,
        ManifestError::UnknownTarget {
            requested: "missing".into(),
            available: vec!["known".into()]
        }
    );
}

#[test]
fn rejects_a_target_that_does_not_match_the_current_platform() {
    let config = parse_config(
        r#"
        [targets.macos]
        platform = { os = "macos", arch = "aarch64" }
        "#,
    );
    let actual = platform("linux");

    let error = EffectiveManifest::select(&config, &actual, Some("macos"), None)
        .expect_err("incompatible target should fail");

    match error {
        ManifestError::IncompatiblePlatform {
            target,
            expected,
            actual: error_actual,
        } => {
            assert_eq!(target, "macos");
            assert_eq!(*expected, config.targets["macos"].platform);
            assert_eq!(*error_actual, actual);
        }
        other => panic!("expected an incompatible platform error, got {other:?}"),
    }
}

#[test]
fn selects_a_nested_profile_by_name_and_merges_its_ancestor_chain() {
    let config = parse_config(PROFILE_CONFIG);

    let manifest =
        EffectiveManifest::select(&config, &platform("linux"), Some("machine"), Some("laptop"))
            .expect("nested profile should be selected by its unique name");

    assert_eq!(manifest.target(), "machine");
    assert_eq!(manifest.profile(), Some("laptop"));
    assert!(manifest.providers().contains_key("system"));
    assert!(manifest.providers().contains_key("desktop"));
    assert!(manifest.packages().contains_key("base"));
    assert!(manifest.packages().contains_key("desktop"));
    assert!(manifest.packages().contains_key("laptop"));
    assert!(!manifest.packages().contains_key("server-only"));

    let Package::Provider(replaced) = &manifest.packages()["replace-me"] else {
        panic!("replacement should remain a provider package");
    };
    assert_eq!(replaced.provider, "desktop");
    assert_eq!(manifest.links()["shared"].source, "desktop-source");
    assert_eq!(manifest.actions()["configure"].exec.program, "desktop-exec");
    assert!(
        manifest.actions()["configure"].check.is_none(),
        "a child action replaces the complete root action"
    );
}

#[test]
fn selecting_no_profile_uses_only_the_target_root() {
    let config = parse_config(PROFILE_CONFIG);

    let manifest = EffectiveManifest::select(&config, &platform("linux"), Some("machine"), None)
        .expect("target root should be a complete selection");

    assert_eq!(manifest.profile(), None);
    assert!(manifest.packages().contains_key("base"));
    assert!(!manifest.packages().contains_key("desktop"));
    assert!(!manifest.packages().contains_key("laptop"));
    assert_eq!(manifest.links()["shared"].source, "root-source");
    assert!(manifest.actions()["configure"].check.is_some());
}

#[test]
fn rejects_duplicate_profile_names_anywhere_in_a_target_tree() {
    let config = parse_config(
        r#"
        [targets.machine]
        platform = { os = "linux" }

        [targets.machine.profiles.desktop.profiles.shared]
        [targets.machine.profiles.server.profiles.shared]
        "#,
    );

    let error =
        EffectiveManifest::select(&config, &platform("linux"), Some("machine"), Some("shared"))
            .expect_err("duplicate profile names should fail before selection");

    assert_eq!(
        error,
        ManifestError::DuplicateProfile {
            target: "machine".into(),
            profile: "shared".into(),
            first_path: "desktop/shared".into(),
            second_path: "server/shared".into(),
        }
    );
}

#[test]
fn reports_an_unknown_profile_with_available_node_names() {
    let config = parse_config(PROFILE_CONFIG);

    let error = EffectiveManifest::select(
        &config,
        &platform("linux"),
        Some("machine"),
        Some("missing"),
    )
    .expect_err("unknown profile should fail");

    assert_eq!(
        error,
        ManifestError::UnknownProfile {
            target: "machine".into(),
            requested: "missing".into(),
            available: vec!["desktop".into(), "laptop".into(), "server".into()],
        }
    );
}

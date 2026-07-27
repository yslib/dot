mod support;

use std::collections::BTreeSet;

use dot::manifest::{
    EffectiveManifest, ManifestError, ManifestJobRef, profile_entries, target_entries,
};
use dot::platform::PlatformInfo;
use dot::schema::{Config, Package, ProviderPackage, SelectorIdentifier};
use support::fixture;

fn parse_fixture(name: &str) -> Config {
    let input = fixture::read(name);
    toml::from_str(&input).expect("test config should deserialize")
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

fn selector_id(value: &str) -> SelectorIdentifier {
    SelectorIdentifier::new(value).expect("test selector identifier should be valid")
}

#[test]
fn execution_infers_the_only_compatible_target() {
    let config = parse_fixture("manifest/valid-compatible-target-inference.toml");

    let manifest = EffectiveManifest::select_for_execution(&config, &platform("linux"), None, None)
        .expect("the only compatible target should be selected");

    assert_eq!(manifest.target(), "linux-machine");
}

#[test]
fn execution_reports_when_no_targets_are_compatible() {
    let config = parse_fixture("manifest/valid-compatible-target-inference.toml");

    let error = EffectiveManifest::select_for_execution(&config, &platform("macos"), None, None)
        .expect_err("selection should fail when no targets are compatible");

    assert_eq!(
        error,
        ManifestError::NoCompatibleTargets {
            available: vec!["linux-machine".into(), "windows-machine".into()],
        }
    );
}

#[test]
fn execution_reports_no_compatible_targets_for_an_empty_target_map() {
    let config = Config {
        targets: Default::default(),
    };

    let error = EffectiveManifest::select_for_execution(&config, &platform("linux"), None, None)
        .expect_err("an empty target map has no compatible targets");

    assert_eq!(
        error,
        ManifestError::NoCompatibleTargets { available: vec![] }
    );
}

#[test]
fn no_compatible_targets_display_omits_available_suffix_when_empty() {
    let error = ManifestError::NoCompatibleTargets { available: vec![] };

    assert_eq!(
        error.to_string(),
        "no configured targets are compatible with this platform"
    );
}

#[test]
fn no_compatible_targets_display_lists_nonempty_available_targets() {
    let error = ManifestError::NoCompatibleTargets {
        available: vec!["linux-machine".into(), "windows-machine".into()],
    };

    assert_eq!(
        error.to_string(),
        "no configured targets are compatible with this platform; available targets: \
         linux-machine, windows-machine"
    );
}

#[test]
fn execution_reports_only_compatible_targets_when_inference_is_ambiguous() {
    let config = parse_fixture("manifest/invalid-ambiguous-targets.toml");

    let error = EffectiveManifest::select_for_execution(&config, &platform("linux"), None, None)
        .expect_err("selection should fail when multiple targets are compatible");

    assert_eq!(
        error,
        ManifestError::TargetRequired {
            available: vec!["first".into(), "second".into()],
        }
    );
}

#[test]
fn execution_rejects_an_explicit_incompatible_target() {
    let config = parse_fixture("manifest/valid-compatible-target-inference.toml");
    let windows = selector_id("windows-machine");

    let error =
        EffectiveManifest::select_for_execution(&config, &platform("linux"), Some(&windows), None)
            .expect_err("execution should reject an incompatible target");

    assert!(matches!(
        error,
        ManifestError::IncompatiblePlatform { target, .. } if target == "windows-machine"
    ));
}

#[test]
fn inspection_permits_an_explicit_incompatible_target() {
    let config = parse_fixture("manifest/valid-compatible-target-inference.toml");
    let windows = selector_id("windows-machine");

    let manifest =
        EffectiveManifest::select_for_inspection(&config, &platform("linux"), Some(&windows), None)
            .expect("inspection should permit an explicitly requested incompatible target");

    assert_eq!(manifest.target(), "windows-machine");
}

#[test]
fn inspection_infers_the_only_compatible_target_when_target_is_omitted() {
    let config = parse_fixture("manifest/valid-compatible-target-inference.toml");

    let manifest =
        EffectiveManifest::select_for_inspection(&config, &platform("linux"), None, None)
            .expect("inspection should infer a compatible target");

    assert_eq!(manifest.target(), "linux-machine");
}

#[test]
fn target_entries_borrow_targets_and_label_platform_compatibility() {
    let config = parse_fixture("manifest/valid-compatible-target-inference.toml");

    let entries = target_entries(&config, &platform("linux")).collect::<Vec<_>>();

    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].id.as_str(), "linux-machine");
    assert!(entries[0].compatible);
    assert!(std::ptr::eq(
        entries[0].target,
        &config.targets["linux-machine"]
    ));
    assert_eq!(entries[1].id.as_str(), "windows-machine");
    assert!(!entries[1].compatible);
    assert!(std::ptr::eq(
        entries[1].target,
        &config.targets["windows-machine"]
    ));
}

#[test]
fn profile_entries_are_borrowed_recursive_preorder_without_the_root() {
    let config = parse_fixture("manifest/valid-profile-tree.toml");
    let (target_id, target) = config
        .targets
        .get_key_value("machine")
        .expect("fixture should contain the machine target");

    let entries = profile_entries(target_id, target)
        .expect("fixture profile names should be unique")
        .collect::<Vec<_>>();
    let actual = entries
        .iter()
        .map(|entry| {
            (
                entry.id.as_str(),
                entry
                    .path
                    .iter()
                    .map(|segment| segment.as_str())
                    .collect::<Vec<_>>()
                    .join("/"),
                entry.depth(),
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        actual,
        vec![
            ("desktop", "desktop".into(), 1),
            ("laptop", "desktop/laptop".into(), 2),
            ("server", "server".into(), 1),
        ]
    );
    assert!(std::ptr::eq(
        entries[0].id,
        config.targets["machine"]
            .profiles
            .get_key_value("desktop")
            .expect("fixture should contain desktop")
            .0,
    ));
}

#[test]
fn profile_entries_report_duplicate_names_in_deterministic_preorder() {
    let config = parse_fixture("manifest/invalid-duplicate-profile-name.toml");
    let (target_id, target) = config
        .targets
        .get_key_value("machine")
        .expect("fixture should contain the machine target");

    let error =
        profile_entries(target_id, target).expect_err("duplicate profile names should fail");

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
fn unresolved_jobs_borrow_records_in_execution_category_order() {
    let config = parse_fixture("manifest/valid-compatible-target-inference.toml");
    let manifest = EffectiveManifest::select_for_execution(&config, &platform("linux"), None, None)
        .expect("the Linux target should be inferred");

    let jobs = manifest.unresolved_jobs().collect::<Vec<_>>();
    let actual = jobs
        .iter()
        .map(|job| match job {
            ManifestJobRef::Package(id, Package::Provider(_)) => {
                format!("provider-package:{id}")
            }
            ManifestJobRef::Package(id, Package::Manual(_)) => format!("manual-package:{id}"),
            ManifestJobRef::Action(id, _) => format!("action:{id}"),
            ManifestJobRef::Link(id, _) => format!("link:{id}"),
        })
        .collect::<Vec<_>>();

    assert_eq!(
        actual,
        vec![
            "provider-package:alpha-provider",
            "provider-package:zulu-provider",
            "manual-package:alpha-manual",
            "manual-package:zulu-manual",
            "action:alpha-action",
            "action:zulu-action",
            "link:alpha-link",
            "link:zulu-link",
        ]
    );

    let ManifestJobRef::Package(first_id, first_package) = jobs[0] else {
        panic!("the first job should be a provider package");
    };
    let (expected_id, expected_package) = manifest
        .packages()
        .get_key_value("alpha-provider")
        .expect("fixture should contain alpha-provider");
    assert!(std::ptr::eq(first_id, expected_id));
    assert!(std::ptr::eq(first_package, expected_package));
}

#[test]
fn selects_the_only_target_when_no_target_is_requested() {
    let config = parse_fixture("manifest/valid-single-target.toml");

    let manifest = EffectiveManifest::select_for_execution(&config, &platform("linux"), None, None)
        .expect("the only compatible target should be selected");

    assert_eq!(manifest.target(), "only");
    assert_eq!(manifest.profile(), None);
    assert!(manifest.packages().contains_key("git"));
}

#[test]
fn requires_a_target_when_the_config_contains_multiple_targets() {
    let config = parse_fixture("manifest/invalid-ambiguous-targets.toml");

    let error = EffectiveManifest::select_for_execution(&config, &platform("linux"), None, None)
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
    let config = parse_fixture("manifest/invalid-unknown-target.toml");
    let missing = selector_id("missing");

    let error =
        EffectiveManifest::select_for_execution(&config, &platform("linux"), Some(&missing), None)
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
    let config = parse_fixture("manifest/invalid-incompatible-platform.toml");
    let actual = platform("linux");
    let macos = selector_id("macos");

    let error = EffectiveManifest::select_for_execution(&config, &actual, Some(&macos), None)
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
    let config = parse_fixture("manifest/valid-profile-tree.toml");
    let machine = selector_id("machine");
    let laptop = selector_id("laptop");

    let manifest = EffectiveManifest::select_for_execution(
        &config,
        &platform("linux"),
        Some(&machine),
        Some(&laptop),
    )
    .expect("nested profile should be selected by its unique name");

    assert_eq!(manifest.target(), "machine");
    assert_eq!(manifest.profile(), Some("laptop"));
    assert!(manifest.providers().contains_key("system"));
    assert!(manifest.providers().contains_key("desktop"));
    assert!(manifest.packages().contains_key("base"));
    assert!(manifest.packages().contains_key("desktop"));
    assert!(manifest.packages().contains_key("laptop"));
    assert!(!manifest.packages().contains_key("server-only"));

    let Package::Provider(ProviderPackage::Single(replaced)) = &manifest.packages()["replace-me"]
    else {
        panic!("replacement should remain a provider package");
    };
    assert_eq!(replaced.provider.as_str(), "desktop");
    assert_eq!(
        manifest.links()["shared"].source.source_spelling(),
        "desktop-source"
    );
    assert_eq!(
        manifest.actions()["configure"]
            .exec
            .program
            .source_spelling(),
        "desktop-exec"
    );
    assert!(
        manifest.actions()["configure"].check.is_none(),
        "a child action replaces the complete root action"
    );
}

#[test]
fn selecting_no_profile_uses_only_the_target_root() {
    let config = parse_fixture("manifest/valid-profile-tree.toml");
    let machine = selector_id("machine");

    let manifest =
        EffectiveManifest::select_for_execution(&config, &platform("linux"), Some(&machine), None)
            .expect("target root should be a complete selection");

    assert_eq!(manifest.profile(), None);
    assert!(manifest.packages().contains_key("base"));
    assert!(!manifest.packages().contains_key("desktop"));
    assert!(!manifest.packages().contains_key("laptop"));
    assert_eq!(
        manifest.links()["shared"].source.source_spelling(),
        "root-source"
    );
    assert!(manifest.actions()["configure"].check.is_some());
}

#[test]
fn rejects_duplicate_profile_names_anywhere_in_a_target_tree() {
    let config = parse_fixture("manifest/invalid-duplicate-profile-name.toml");
    let machine = selector_id("machine");
    let shared = selector_id("shared");

    let error = EffectiveManifest::select_for_execution(
        &config,
        &platform("linux"),
        Some(&machine),
        Some(&shared),
    )
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
    let config = parse_fixture("manifest/valid-profile-tree.toml");
    let machine = selector_id("machine");
    let missing = selector_id("missing");

    let error = EffectiveManifest::select_for_execution(
        &config,
        &platform("linux"),
        Some(&machine),
        Some(&missing),
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

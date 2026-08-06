mod support;

use std::env;
use std::error::Error;
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use dot::config::ConfigParseError;
use dot::manifest::ManifestError;
use dot::native::{ConfigLoadError, ConfigLocation, NativeRuntime, load_config};
use dot::platform::PlatformInfo;
use dot::schema::Config;
use dot::validation::{ConfigValidationError, ConfigValidationErrorKind};
use support::fixture;

fn relative_fixture(name: &str) -> (PathBuf, PathBuf) {
    let absolute = fixture::path(name);
    let invocation_cwd = env::current_dir().expect("test should have a current directory");
    let relative = absolute
        .strip_prefix(&invocation_cwd)
        .expect("fixture should be below the invocation directory")
        .to_owned();
    (relative, absolute)
}

#[test]
fn parses_a_valid_fixture_from_memory_with_static_validation() {
    let source = fs::read_to_string(fixture::path("dot.toml")).expect("fixture should be readable");

    let config = Config::parse(&source).expect("fixture should parse from memory");

    assert_eq!(config.targets.len(), 6);
    assert!(config.targets.contains_key("arch-personal"));
}

#[test]
fn preserves_schema_keyed_table_declaration_order() {
    let source = r#"
[targets.zulu]
platform = { os = "linux" }

[targets.zulu.providers.zulu]
probe = { program = "probe-zulu" }
install = { program = "install-zulu", args = ["${package:names}"] }

[targets.zulu.providers.alpha]
probe = { program = "probe-alpha" }
install = { program = "install-alpha", args = ["${package:names}"] }

[targets.zulu.packages.zulu]
install = { exec = { program = "package-zulu" } }

[targets.zulu.packages.alpha]
install = { exec = { program = "package-alpha" } }

[targets.zulu.actions.zulu]
exec = { program = "action-zulu" }

[targets.zulu.actions.alpha]
exec = { program = "action-alpha" }

[targets.zulu.links.zulu]
source = "source-zulu"
target = "target-zulu"

[targets.zulu.links.alpha]
source = "source-alpha"
target = "target-alpha"

[targets.zulu.profiles.zulu]

[targets.zulu.profiles.alpha]

[targets.alpha]
platform = { os = "linux" }
"#;

    let config = Config::parse(source).expect("configuration should parse");
    let zulu = config
        .targets
        .get("zulu")
        .expect("zulu target should exist");

    assert_eq!(
        config
            .targets
            .keys()
            .map(|id| id.as_str())
            .collect::<Vec<_>>(),
        ["zulu", "alpha"]
    );
    assert_eq!(
        zulu.providers
            .keys()
            .map(|id| id.as_str())
            .collect::<Vec<_>>(),
        ["zulu", "alpha"]
    );
    assert_eq!(
        zulu.packages
            .keys()
            .map(|id| id.as_str())
            .collect::<Vec<_>>(),
        ["zulu", "alpha"]
    );
    assert_eq!(
        zulu.actions
            .keys()
            .map(|id| id.as_str())
            .collect::<Vec<_>>(),
        ["zulu", "alpha"]
    );
    assert_eq!(
        zulu.links.keys().map(|id| id.as_str()).collect::<Vec<_>>(),
        ["zulu", "alpha"]
    );
    assert_eq!(
        zulu.profiles
            .keys()
            .map(|id| id.as_str())
            .collect::<Vec<_>>(),
        ["zulu", "alpha"]
    );
}

#[test]
fn distinguishes_deserialization_and_validation_errors_when_parsing_from_memory() {
    let deserialization_source = fs::read_to_string(fixture::path("config/invalid-syntax.toml"))
        .expect("fixture should be readable");
    let validation_source = fs::read_to_string(fixture::path(
        "manifest/invalid-duplicate-profile-name.toml",
    ))
    .expect("fixture should be readable");

    let deserialization_error = Config::parse(&deserialization_source)
        .expect_err("invalid TOML should fail deserialization");
    let validation_error =
        Config::parse(&validation_source).expect_err("invalid manifest should fail validation");

    assert!(matches!(
        deserialization_error,
        ConfigParseError::Deserialize { .. }
    ));
    assert!(
        deserialization_error
            .source()
            .and_then(|source| source.downcast_ref::<toml::de::Error>())
            .is_some()
    );
    assert!(matches!(
        validation_error,
        ConfigParseError::Validation { .. }
    ));
    assert!(
        validation_error
            .source()
            .and_then(|source| source.downcast_ref::<ConfigValidationError>())
            .is_some()
    );
}

#[test]
fn memory_parsing_matches_path_loading_for_the_same_fixture() {
    let fixture_path = fixture::path("dot.toml");
    let source = fs::read_to_string(&fixture_path).expect("fixture should be readable");

    let parsed = Config::parse(&source).expect("fixture should parse from memory");
    let loaded =
        load_config(ConfigLocation::Path(fixture_path)).expect("fixture should load from path");

    assert_eq!(&parsed, loaded.config());
}

#[test]
fn loads_a_relative_manifest_with_its_path_context() {
    let invocation_cwd = env::current_dir().expect("test should have a current directory");
    let (relative_path, expected_path) = relative_fixture("dot.toml");

    let loaded = load_config(ConfigLocation::Path(relative_path)).expect("fixture should load");

    assert_eq!(loaded.config().targets.len(), 6);
    assert_eq!(loaded.config_dir(), expected_path.parent().unwrap());
    assert!(loaded.config_dir().is_absolute());
    let expected_real_path =
        fs::canonicalize(&expected_path).expect("fixture path should canonicalize");
    assert_eq!(
        loaded.real_config_dir(),
        expected_real_path
            .parent()
            .expect("canonical fixture should have a parent")
    );
    assert_eq!(loaded.cwd(), invocation_cwd);
}

#[test]
fn detects_native_runtime_independently_from_config_loading() {
    let runtime = NativeRuntime::detect();

    assert_eq!(runtime.platform(), &PlatformInfo::detect());
}

#[test]
fn reports_the_entry_path_when_a_manifest_cannot_be_canonicalized() {
    let (relative_path, expected_path) = relative_fixture("config/does-not-exist.toml");

    let error =
        load_config(ConfigLocation::Path(relative_path)).expect_err("missing fixture should fail");

    match &error {
        ConfigLoadError::Canonicalize { path, source } => {
            assert_eq!(path, &expected_path);
            assert_eq!(source.kind(), ErrorKind::NotFound);
        }
        other => panic!("expected a canonicalize error, got {other:?}"),
    }
    assert!(
        error
            .to_string()
            .contains(expected_path.to_string_lossy().as_ref())
    );
    assert!(error.source().is_some());
}

#[test]
fn reports_the_manifest_path_when_toml_is_invalid() {
    let (relative_path, expected_path) = relative_fixture("config/invalid-syntax.toml");

    let error =
        load_config(ConfigLocation::Path(relative_path)).expect_err("invalid fixture should fail");

    match &error {
        ConfigLoadError::Parse { path, .. } => assert_eq!(path, &expected_path),
        other => panic!("expected a parse error, got {other:?}"),
    }
    assert!(
        error
            .to_string()
            .contains(expected_path.to_string_lossy().as_ref())
    );
    assert!(error.source().is_some());
}

#[test]
fn reports_the_absolute_path_when_the_complete_manifest_is_invalid() {
    let (relative_path, expected_path) =
        relative_fixture("manifest/invalid-duplicate-profile-name.toml");

    let error = load_config(ConfigLocation::Path(relative_path))
        .expect_err("duplicate profiles must fail validation");

    match &error {
        ConfigLoadError::Validation { path, source } => {
            assert_eq!(path, &expected_path);
            assert!(matches!(
                &source.kind,
                ConfigValidationErrorKind::Manifest(ManifestError::DuplicateProfile {
                    target,
                    profile,
                    first_path,
                    second_path,
                }) if target == "machine"
                    && profile == "shared"
                    && first_path == "desktop/shared"
                    && second_path == "server/shared"
            ));
        }
        other => panic!("expected a validation error, got {other:?}"),
    }
    assert!(
        error
            .to_string()
            .contains(expected_path.to_string_lossy().as_ref())
    );
    assert!(error.source().is_some());
}

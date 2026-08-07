mod support;

use std::fs;

use dot_core::ConfigFile;
use dot_core::config::ConfigParseError;
use dot_core::native::NativeRuntime;
use dot_core::platform::PlatformInfo;
use dot_core::schema::Config;
use support::fixture;

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
    assert!(matches!(
        validation_error,
        ConfigParseError::Validation { .. }
    ));
}

#[test]
fn parsed_configuration_constructs_an_explicit_protocol_context() {
    let path = fixture::path("dot.toml");
    let source = fixture::read("dot.toml");
    let parsed = Config::parse(&source).expect("fixture should parse from memory");
    let real_path = fs::canonicalize(&path).expect("fixture should canonicalize");
    let cwd = std::env::current_dir().expect("test should have a current directory");

    let config = ConfigFile::new(
        parsed,
        path.parent()
            .expect("fixture should have a parent")
            .to_owned(),
        real_path
            .parent()
            .expect("canonical fixture should have a parent")
            .to_owned(),
        cwd.clone(),
    )
    .expect("fixture context should be absolute");

    assert_eq!(config.config().targets.len(), 6);
    assert_eq!(config.config_dir(), path.parent().unwrap());
    assert_eq!(config.real_config_dir(), real_path.parent().unwrap());
    assert_eq!(config.cwd(), cwd);
}

#[test]
fn detects_native_runtime_independently_from_config_acquisition() {
    let runtime = NativeRuntime::detect();

    assert_eq!(runtime.platform(), &PlatformInfo::detect());
}

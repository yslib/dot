mod support;

use std::env;
use std::error::Error;
use std::io::ErrorKind;
use std::path::PathBuf;

use dot::config::{ConfigLoadError, LoadedConfig};
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
fn loads_a_relative_manifest_with_its_runtime_context() {
    let invocation_cwd = env::current_dir().expect("test should have a current directory");
    let (relative_path, expected_path) = relative_fixture("dot.toml");

    let loaded = LoadedConfig::load(&relative_path).expect("fixture should load");

    assert_eq!(loaded.config().targets.len(), 6);
    assert_eq!(loaded.path(), expected_path);
    assert!(loaded.path().is_absolute());
    assert_eq!(loaded.directory(), expected_path.parent().unwrap());
    assert_eq!(loaded.invocation_cwd(), invocation_cwd);
    assert_eq!(
        loaded.environment().get("PATH"),
        env::var_os("PATH").as_deref()
    );
}

#[test]
fn reports_the_absolute_path_when_a_manifest_cannot_be_read() {
    let (relative_path, expected_path) = relative_fixture("config/does-not-exist.toml");

    let error = LoadedConfig::load(&relative_path).expect_err("missing fixture should fail");

    match &error {
        ConfigLoadError::Read { path, source } => {
            assert_eq!(path, &expected_path);
            assert_eq!(source.kind(), ErrorKind::NotFound);
        }
        other => panic!("expected a read error, got {other:?}"),
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

    let error = LoadedConfig::load(&relative_path).expect_err("invalid fixture should fail");

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

use std::env;
use std::path::PathBuf;

use dot_core::schema::Config;
use dot_core::{ConfigFile, ConfigFileError};

fn config() -> Config {
    Config::parse(
        r#"
[targets.machine]
platform = { os = "linux" }
"#,
    )
    .expect("test configuration should parse")
}

fn absolute_paths() -> (PathBuf, PathBuf, PathBuf) {
    let cwd = env::current_dir().expect("test should have a current directory");
    (
        cwd.join("entry"),
        cwd.join("canonical"),
        cwd.join("invocation"),
    )
}

#[test]
fn stores_the_config_and_supplied_absolute_directories() {
    let config = config();
    let (config_dir, real_config_dir, cwd) = absolute_paths();

    let config_file = ConfigFile::new(
        config.clone(),
        config_dir.clone(),
        real_config_dir.clone(),
        cwd.clone(),
    )
    .expect("absolute protocol paths should be accepted");

    assert_eq!(config_file.config(), &config);
    assert_eq!(config_file.config_dir(), config_dir);
    assert_eq!(config_file.real_config_dir(), real_config_dir);
    assert_eq!(config_file.cwd(), cwd);
}

#[test]
fn rejects_a_relative_config_directory() {
    let (_, real_config_dir, cwd) = absolute_paths();
    let path = PathBuf::from("relative-config-dir");

    let error = ConfigFile::new(config(), path.clone(), real_config_dir, cwd)
        .expect_err("a relative config directory should be rejected");

    assert_eq!(error, ConfigFileError::RelativeConfigDir { path });
}

#[test]
fn rejects_a_relative_real_config_directory() {
    let (config_dir, _, cwd) = absolute_paths();
    let path = PathBuf::from("relative-real-config-dir");

    let error = ConfigFile::new(config(), config_dir, path.clone(), cwd)
        .expect_err("a relative real config directory should be rejected");

    assert_eq!(error, ConfigFileError::RelativeRealConfigDir { path });
}

#[test]
fn rejects_a_relative_working_directory() {
    let (config_dir, real_config_dir, _) = absolute_paths();
    let path = PathBuf::from("relative-cwd");

    let error = ConfigFile::new(config(), config_dir, real_config_dir, path.clone())
        .expect_err("a relative working directory should be rejected");

    assert_eq!(error, ConfigFileError::RelativeCwd { path });
}

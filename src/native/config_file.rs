//! Configuration discovery, loading, and path metadata.

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use directories::BaseDirs;

use crate::config::ConfigParseError;
use crate::interpolation::DotPaths;
use crate::schema::Config;
use crate::validation::ConfigValidationError;

const DEFAULT_CONFIG_FILENAME: &str = ".dot.toml";

fn path_entry_exists(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(source),
    }
}

fn detect_user_config_root() -> Option<UserConfigRoot> {
    let base_directories = BaseDirs::new()?;

    #[cfg(windows)]
    {
        Some(UserConfigRoot::Roaming(
            base_directories.config_dir().to_path_buf(),
        ))
    }

    #[cfg(not(windows))]
    {
        Some(UserConfigRoot::Home(
            base_directories.home_dir().to_path_buf(),
        ))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigLocation {
    Path(PathBuf),
    Discover,
}

impl ConfigLocation {
    fn resolve(self) -> Result<PathBuf, ConfigDiscoveryError> {
        self.resolve_with(env::current_dir, detect_user_config_root, path_entry_exists)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum UserConfigRoot {
    // Keep both variants available so path construction stays unit-testable on either host.
    #[cfg_attr(windows, allow(dead_code))]
    Home(PathBuf),
    #[cfg_attr(not(windows), allow(dead_code))]
    Roaming(PathBuf),
}

impl UserConfigRoot {
    fn manifest_path(self) -> PathBuf {
        match self {
            Self::Home(root) => root
                .join(".config")
                .join("dot")
                .join(DEFAULT_CONFIG_FILENAME),
            Self::Roaming(root) => root.join("dot").join(DEFAULT_CONFIG_FILENAME),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigDiscoveryError {
    #[error("failed to determine the invocation directory: {source}")]
    CurrentDirectory {
        #[source]
        source: io::Error,
    },
    #[error("failed to determine the user configuration directory")]
    UserDirectoryUnavailable,
    #[error(
        "failed to inspect configuration candidate `{}`: {source}",
        .path.display()
    )]
    Inspect {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "configuration not found; checked `{}` then `{}`; use --config PATH to select another file",
        .local.display(),
        .user.display()
    )]
    NotFound { local: PathBuf, user: PathBuf },
}

impl ConfigLocation {
    fn resolve_with<CurrentDir, UserRoot, Present>(
        self,
        current_dir: CurrentDir,
        user_root: UserRoot,
        mut present: Present,
    ) -> Result<PathBuf, ConfigDiscoveryError>
    where
        CurrentDir: FnOnce() -> io::Result<PathBuf>,
        UserRoot: FnOnce() -> Option<UserConfigRoot>,
        Present: FnMut(&Path) -> io::Result<bool>,
    {
        match self {
            Self::Path(path) => Ok(path),
            Self::Discover => {
                let invocation_cwd = current_dir()
                    .map_err(|source| ConfigDiscoveryError::CurrentDirectory { source })?;
                let local = invocation_cwd.join(DEFAULT_CONFIG_FILENAME);

                if present(&local).map_err(|source| ConfigDiscoveryError::Inspect {
                    path: local.clone(),
                    source,
                })? {
                    Ok(local)
                } else {
                    let user = user_root()
                        .ok_or(ConfigDiscoveryError::UserDirectoryUnavailable)?
                        .manifest_path();

                    if present(&user).map_err(|source| ConfigDiscoveryError::Inspect {
                        path: user.clone(),
                        source,
                    })? {
                        Ok(user)
                    } else {
                        Err(ConfigDiscoveryError::NotFound { local, user })
                    }
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigFile {
    config: Config,
    path: PathBuf,
    directory: PathBuf,
    real_path: PathBuf,
    real_directory: PathBuf,
    invocation_cwd: PathBuf,
}

impl ConfigFile {
    pub fn load(location: ConfigLocation) -> Result<Self, ConfigFileError> {
        let path = location.resolve()?;
        let invocation_cwd =
            env::current_dir().map_err(|source| ConfigFileError::CurrentDirectory { source })?;
        let path = absolute_path(&path, &invocation_cwd);
        let real_path =
            fs::canonicalize(&path).map_err(|source| ConfigFileError::Canonicalize {
                path: path.clone(),
                source,
            })?;
        let source = fs::read_to_string(&real_path).map_err(|source| ConfigFileError::Read {
            path: path.clone(),
            source,
        })?;
        let config = Config::parse(&source).map_err(|error| match error {
            ConfigParseError::Deserialize { source } => ConfigFileError::Parse {
                path: path.clone(),
                source,
            },
            ConfigParseError::Validation { source } => ConfigFileError::Validation {
                path: path.clone(),
                source,
            },
        })?;
        let directory = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| invocation_cwd.clone());
        let real_directory = real_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| invocation_cwd.clone());

        Ok(Self {
            config,
            path,
            directory,
            real_path,
            real_directory,
            invocation_cwd,
        })
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn real_path(&self) -> &Path {
        &self.real_path
    }

    pub fn real_directory(&self) -> &Path {
        &self.real_directory
    }

    pub fn invocation_cwd(&self) -> &Path {
        &self.invocation_cwd
    }
}

impl<'a> From<&'a ConfigFile> for DotPaths<'a> {
    fn from(config: &'a ConfigFile) -> Self {
        Self::new(
            config.path(),
            config.directory(),
            config.real_path(),
            config.real_directory(),
            config.invocation_cwd(),
        )
    }
}

fn absolute_path(path: &Path, invocation_cwd: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        invocation_cwd.join(path)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigFileError {
    #[error(transparent)]
    Discovery(#[from] ConfigDiscoveryError),

    #[error("failed to determine the invocation directory: {source}")]
    CurrentDirectory {
        #[source]
        source: io::Error,
    },
    #[error(
        "failed to canonicalize configuration `{}`: {source}",
        .path.display()
    )]
    Canonicalize {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to read configuration `{}`: {source}", .path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to parse configuration `{}`: {source}", .path.display())]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error(
        "failed to validate configuration `{}`: {source}",
        .path.display()
    )]
    Validation {
        path: PathBuf,
        #[source]
        source: ConfigValidationError,
    },
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;

    fn unique_temp_path(label: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after the Unix epoch")
            .as_nanos();
        env::temp_dir().join(format!("dot-{label}-{}-{nonce}", std::process::id()))
    }

    #[test]
    fn explicit_request_bypasses_discovery() {
        let requested = PathBuf::from("relative/config.toml");

        let resolved = ConfigLocation::Path(requested.clone())
            .resolve_with(
                || -> io::Result<PathBuf> { panic!("current directory must not be queried") },
                || panic!("user root must not be queried"),
                |_| -> io::Result<bool> { panic!("candidate must not be inspected") },
            )
            .expect("explicit request should resolve");

        assert_eq!(resolved, requested);
    }

    #[test]
    fn explicit_resolve_preserves_the_requested_path() {
        let requested = PathBuf::from("relative/config.toml");

        let resolved = ConfigLocation::Path(requested.clone())
            .resolve()
            .expect("explicit request should resolve");

        assert_eq!(resolved, requested);
    }

    #[test]
    fn local_candidate_wins_without_querying_user_root() {
        let invocation_cwd = PathBuf::from("/work");
        let expected = invocation_cwd.join(".dot.toml");

        let resolved = ConfigLocation::Discover
            .resolve_with(
                || Ok(invocation_cwd),
                || panic!("user root must not be queried"),
                |candidate| {
                    assert_eq!(candidate, expected);
                    Ok(true)
                },
            )
            .expect("local candidate should resolve");

        assert_eq!(resolved, expected);
    }

    #[test]
    fn home_root_constructs_platform_neutral_manifest_path() {
        let root = PathBuf::from("/home/alice");

        let manifest = UserConfigRoot::Home(root.clone()).manifest_path();

        assert_eq!(manifest, root.join(".config").join("dot").join(".dot.toml"));
    }

    #[test]
    fn roaming_root_constructs_platform_neutral_manifest_path() {
        let root = PathBuf::from(r"C:\Users\alice\AppData\Roaming");

        let manifest = UserConfigRoot::Roaming(root.clone()).manifest_path();

        assert_eq!(manifest, root.join("dot").join(".dot.toml"));
    }

    #[test]
    fn missing_local_candidate_selects_user_candidate() {
        let invocation_cwd = PathBuf::from("/work");
        let local = invocation_cwd.join(".dot.toml");
        let home = PathBuf::from("/home/alice");
        let user = home.join(".config").join("dot").join(".dot.toml");
        let mut inspected = Vec::new();

        let resolved = ConfigLocation::Discover
            .resolve_with(
                || Ok(invocation_cwd),
                || Some(UserConfigRoot::Home(home)),
                |candidate| {
                    inspected.push(candidate.to_path_buf());
                    Ok(candidate == user)
                },
            )
            .expect("user candidate should resolve");

        assert_eq!(resolved, user);
        assert_eq!(inspected, vec![local, user]);
    }

    #[test]
    fn local_inspection_error_stops_before_user_detection() {
        let invocation_cwd = PathBuf::from("/work");
        let local = invocation_cwd.join(".dot.toml");

        let error = ConfigLocation::Discover
            .resolve_with(
                || Ok(invocation_cwd),
                || panic!("user root must not be queried"),
                |_| {
                    Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "inspection blocked",
                    ))
                },
            )
            .expect_err("inspection should fail");

        match error {
            ConfigDiscoveryError::Inspect { path, source } => {
                assert_eq!(path, local);
                assert_eq!(source.kind(), io::ErrorKind::PermissionDenied);
            }
            other => panic!("expected inspect error, got {other:?}"),
        }
    }

    #[test]
    fn unavailable_user_root_is_reported_only_after_local_is_missing() {
        let invocation_cwd = PathBuf::from("/work");
        let local = invocation_cwd.join(".dot.toml");
        let mut inspected = Vec::new();

        let error = ConfigLocation::Discover
            .resolve_with(
                || Ok(invocation_cwd),
                || None,
                |candidate| {
                    inspected.push(candidate.to_path_buf());
                    Ok(false)
                },
            )
            .expect_err("missing user root should fail");

        assert!(matches!(
            error,
            ConfigDiscoveryError::UserDirectoryUnavailable
        ));
        assert_eq!(inspected, vec![local]);
    }

    #[test]
    fn current_directory_error_is_typed() {
        let error = ConfigLocation::Discover
            .resolve_with(
                || {
                    Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "cwd unavailable",
                    ))
                },
                || panic!("user root must not be queried"),
                |_| -> io::Result<bool> { panic!("candidate must not be inspected") },
            )
            .expect_err("current-directory lookup should fail");

        match error {
            ConfigDiscoveryError::CurrentDirectory { source } => {
                assert_eq!(source.kind(), io::ErrorKind::PermissionDenied);
            }
            other => panic!("expected current-directory error, got {other:?}"),
        }
    }

    #[test]
    fn missing_candidates_report_both_paths_in_probe_order() {
        let invocation_cwd = PathBuf::from("/work");
        let local = invocation_cwd.join(".dot.toml");
        let home = PathBuf::from("/home/alice");
        let user = home.join(".config").join("dot").join(".dot.toml");

        let error = ConfigLocation::Discover
            .resolve_with(
                || Ok(invocation_cwd),
                || Some(UserConfigRoot::Home(home)),
                |_| Ok(false),
            )
            .expect_err("missing candidates should fail");

        match &error {
            ConfigDiscoveryError::NotFound {
                local: actual_local,
                user: actual_user,
            } => {
                assert_eq!(actual_local, &local);
                assert_eq!(actual_user, &user);
            }
            other => panic!("expected not-found error, got {other:?}"),
        }
        assert_eq!(
            error.to_string(),
            format!(
                "configuration not found; checked `{}` then `{}`; use --config PATH to select another file",
                local.display(),
                user.display()
            )
        );
        assert!(error.to_string().contains("--config PATH"));
    }

    #[test]
    fn discovery_errors_have_exact_messages() {
        let current_directory = ConfigDiscoveryError::CurrentDirectory {
            source: io::Error::new(io::ErrorKind::PermissionDenied, "cwd blocked"),
        };
        assert_eq!(
            current_directory.to_string(),
            "failed to determine the invocation directory: cwd blocked"
        );

        let unavailable = ConfigDiscoveryError::UserDirectoryUnavailable;
        assert_eq!(
            unavailable.to_string(),
            "failed to determine the user configuration directory"
        );

        let candidate = PathBuf::from("/work/.dot.toml");
        let inspect = ConfigDiscoveryError::Inspect {
            path: candidate.clone(),
            source: io::Error::new(io::ErrorKind::PermissionDenied, "entry blocked"),
        };
        assert_eq!(
            inspect.to_string(),
            format!(
                "failed to inspect configuration candidate `{}`: entry blocked",
                candidate.display()
            )
        );
    }

    #[test]
    fn discovery_error_sources_expose_only_io_errors() {
        let current_directory = ConfigDiscoveryError::CurrentDirectory {
            source: io::Error::new(io::ErrorKind::PermissionDenied, "cwd blocked"),
        };
        assert_eq!(
            Error::source(&current_directory)
                .expect("current-directory error should have a source")
                .to_string(),
            "cwd blocked"
        );

        let inspect = ConfigDiscoveryError::Inspect {
            path: PathBuf::from("/work/.dot.toml"),
            source: io::Error::new(io::ErrorKind::PermissionDenied, "entry blocked"),
        };
        assert_eq!(
            Error::source(&inspect)
                .expect("inspect error should have a source")
                .to_string(),
            "entry blocked"
        );

        assert!(Error::source(&ConfigDiscoveryError::UserDirectoryUnavailable).is_none());
        assert!(
            Error::source(&ConfigDiscoveryError::NotFound {
                local: PathBuf::from("/work/.dot.toml"),
                user: PathBuf::from("/home/alice/.config/dot/.dot.toml"),
            })
            .is_none()
        );
    }

    #[test]
    fn missing_path_entry_is_absent() {
        let missing = unique_temp_path("missing-config");

        assert!(!path_entry_exists(&missing).expect("missing entry should be inspectable"));
    }

    #[cfg(unix)]
    #[test]
    fn dangling_symlink_is_present_but_cannot_be_canonicalized() {
        let directory = unique_temp_path("dangling-config");
        fs::create_dir(&directory).expect("temporary directory should be created");
        let candidate = directory.join(".dot.toml");
        std::os::unix::fs::symlink(directory.join("missing-target"), &candidate)
            .expect("dangling symlink should be created");

        let present = path_entry_exists(&candidate).expect("symlink entry should be inspectable");
        let load_error = ConfigFile::load(ConfigLocation::Path(candidate.clone()))
            .expect_err("dangling symlink should not load");

        fs::remove_file(&candidate).expect("temporary symlink should be removed");
        fs::remove_dir(&directory).expect("temporary directory should be removed");

        assert!(present);
        match load_error {
            ConfigFileError::Canonicalize { path, source } => {
                assert_eq!(path, candidate);
                assert_eq!(source.kind(), io::ErrorKind::NotFound);
            }
            other => panic!("expected canonicalize error, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn canonicalizable_directory_entry_fails_during_read() {
        let root = unique_temp_path("directory-config");
        fs::create_dir(&root).expect("temporary root should be created");
        let entity = root.join("config-directory");
        fs::create_dir(&entity).expect("directory entity should be created");
        let entry = root.join(".dot.toml");
        std::os::unix::fs::symlink(&entity, &entry)
            .expect("configuration symlink should be created");

        let result = ConfigFile::load(ConfigLocation::Path(entry.clone()));

        fs::remove_file(&entry).expect("temporary symlink should be removed");
        fs::remove_dir(&entity).expect("directory entity should be removed");
        fs::remove_dir(&root).expect("temporary root should be removed");

        let error = result.expect_err("directory configuration should fail during read");
        let immediate_source =
            Error::source(&error).expect("read error should have an immediate source");
        assert!(
            immediate_source.downcast_ref::<io::Error>().is_some(),
            "read error source should be an io::Error"
        );
        match error {
            ConfigFileError::Read { path, source } => {
                assert_eq!(path, entry);
                assert_ne!(path, entity);
                assert_eq!(source.kind(), io::ErrorKind::IsADirectory);
            }
            other => panic!("expected read error, got {other:?}"),
        }
    }

    #[test]
    fn static_document_loads_config_and_absolute_path_metadata() {
        let invocation_cwd = env::current_dir().expect("test should have a current directory");
        let relative_path = Path::new("tests/fixtures/dot.toml");
        let expected_path = invocation_cwd.join(relative_path);
        let expected_real_path =
            fs::canonicalize(relative_path).expect("fixture path should canonicalize");

        let loaded = ConfigFile::load(ConfigLocation::Path(relative_path.to_path_buf()))
            .expect("fixture should load");

        assert_eq!(loaded.config().targets.len(), 6);
        assert_eq!(loaded.path(), expected_path);
        assert!(loaded.path().is_absolute());
        assert_eq!(loaded.directory(), expected_path.parent().unwrap());
        assert_eq!(loaded.real_path(), expected_real_path);
        assert_eq!(
            loaded.real_directory(),
            expected_real_path
                .parent()
                .expect("canonical fixture should have a parent")
        );
        assert_eq!(loaded.invocation_cwd(), invocation_cwd);
    }

    #[cfg(unix)]
    #[test]
    fn static_document_distinguishes_a_symlink_entry_from_the_real_config() {
        let directory = unique_temp_path("config-symlink");
        fs::create_dir(&directory).expect("temporary directory should be created");
        let entry = directory.join(".dot.toml");
        let real =
            fs::canonicalize("tests/fixtures/dot.toml").expect("fixture should canonicalize");
        std::os::unix::fs::symlink(&real, &entry).expect("configuration symlink should be created");

        let result = ConfigFile::load(ConfigLocation::Path(entry.clone()));

        fs::remove_file(&entry).expect("temporary symlink should be removed");
        fs::remove_dir(&directory).expect("temporary directory should be removed");

        let loaded = result.expect("configuration symlink should load");
        assert_eq!(loaded.path(), entry);
        assert_eq!(loaded.directory(), directory);
        assert_eq!(loaded.real_path(), real);
        assert_eq!(loaded.real_directory(), real.parent().unwrap());
    }
}

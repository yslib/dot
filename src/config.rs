use std::env;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use directories::BaseDirs;

use crate::action::ExecutionEnvironment;
use crate::schema::Config;
use crate::validation::{ConfigValidationError, validate_config};

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
pub enum ConfigRequest {
    Explicit(PathBuf),
    Discover,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum UserConfigRoot {
    Home(PathBuf),
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

#[derive(Debug)]
pub enum ConfigDiscoveryError {
    CurrentDirectory { source: io::Error },
    UserDirectoryUnavailable,
    Inspect { path: PathBuf, source: io::Error },
    NotFound { local: PathBuf, user: PathBuf },
}

impl ConfigRequest {
    pub(crate) fn resolve(self) -> Result<PathBuf, ConfigDiscoveryError> {
        self.resolve_with(env::current_dir, detect_user_config_root, path_entry_exists)
    }

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
            Self::Explicit(path) => Ok(path),
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

impl fmt::Display for ConfigDiscoveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CurrentDirectory { source } => {
                write!(
                    formatter,
                    "failed to determine the invocation directory: {source}"
                )
            }
            Self::UserDirectoryUnavailable => {
                write!(
                    formatter,
                    "failed to determine the user configuration directory"
                )
            }
            Self::Inspect { path, source } => {
                write!(
                    formatter,
                    "failed to inspect configuration candidate `{}`: {source}",
                    path.display()
                )
            }
            Self::NotFound { local, user } => {
                write!(
                    formatter,
                    "configuration not found; checked `{}` then `{}`; use --config PATH to select another file",
                    local.display(),
                    user.display()
                )
            }
        }
    }
}

impl Error for ConfigDiscoveryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::CurrentDirectory { source } | Self::Inspect { source, .. } => Some(source),
            Self::UserDirectoryUnavailable | Self::NotFound { .. } => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LoadedConfigDocument {
    config: Config,
    path: PathBuf,
    directory: PathBuf,
    invocation_cwd: PathBuf,
}

impl LoadedConfigDocument {
    pub(crate) fn load(path: impl AsRef<Path>) -> Result<Self, ConfigLoadError> {
        let invocation_cwd =
            env::current_dir().map_err(|source| ConfigLoadError::CurrentDirectory { source })?;
        let path = absolute_path(path.as_ref(), &invocation_cwd);
        let source = fs::read_to_string(&path).map_err(|source| ConfigLoadError::Read {
            path: path.clone(),
            source,
        })?;
        let config = toml::from_str(&source).map_err(|source| ConfigLoadError::Parse {
            path: path.clone(),
            source,
        })?;
        validate_config(&config).map_err(|source| ConfigLoadError::Validation {
            path: path.clone(),
            source: Box::new(source),
        })?;
        let directory = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| invocation_cwd.clone());

        Ok(Self {
            config,
            path,
            directory,
            invocation_cwd,
        })
    }

    pub(crate) fn config(&self) -> &Config {
        &self.config
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn directory(&self) -> &Path {
        &self.directory
    }

    pub(crate) fn invocation_cwd(&self) -> &Path {
        &self.invocation_cwd
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedConfig {
    document: LoadedConfigDocument,
    environment: ExecutionEnvironment,
}

impl LoadedConfig {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigLoadError> {
        let document = LoadedConfigDocument::load(path)?;
        let environment = ExecutionEnvironment::capture();

        Ok(Self {
            document,
            environment,
        })
    }

    pub fn config(&self) -> &Config {
        self.document.config()
    }

    pub fn path(&self) -> &Path {
        self.document.path()
    }

    pub fn directory(&self) -> &Path {
        self.document.directory()
    }

    pub fn invocation_cwd(&self) -> &Path {
        self.document.invocation_cwd()
    }

    pub fn environment(&self) -> &ExecutionEnvironment {
        &self.environment
    }
}

fn absolute_path(path: &Path, invocation_cwd: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        invocation_cwd.join(path)
    }
}

#[derive(Debug)]
pub enum ConfigLoadError {
    CurrentDirectory {
        source: io::Error,
    },
    Read {
        path: PathBuf,
        source: io::Error,
    },
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    Validation {
        path: PathBuf,
        source: Box<ConfigValidationError>,
    },
}

impl fmt::Display for ConfigLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CurrentDirectory { source } => {
                write!(
                    formatter,
                    "failed to determine the invocation directory: {source}"
                )
            }
            Self::Read { path, source } => {
                write!(
                    formatter,
                    "failed to read configuration `{}`: {source}",
                    path.display()
                )
            }
            Self::Parse { path, source } => {
                write!(
                    formatter,
                    "failed to parse configuration `{}`: {source}",
                    path.display()
                )
            }
            Self::Validation { path, source } => {
                write!(
                    formatter,
                    "failed to validate configuration `{}`: {source}",
                    path.display()
                )
            }
        }
    }
}

impl Error for ConfigLoadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::CurrentDirectory { source } | Self::Read { source, .. } => Some(source),
            Self::Parse { source, .. } => Some(source),
            Self::Validation { source, .. } => Some(source.as_ref()),
        }
    }
}

#[cfg(test)]
mod tests {
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

        let resolved = ConfigRequest::Explicit(requested.clone())
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

        let resolved = ConfigRequest::Explicit(requested.clone())
            .resolve()
            .expect("explicit request should resolve");

        assert_eq!(resolved, requested);
    }

    #[test]
    fn local_candidate_wins_without_querying_user_root() {
        let invocation_cwd = PathBuf::from("/work");
        let expected = invocation_cwd.join(".dot.toml");

        let resolved = ConfigRequest::Discover
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

        let resolved = ConfigRequest::Discover
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

        let error = ConfigRequest::Discover
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

        let error = ConfigRequest::Discover
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
        let error = ConfigRequest::Discover
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

        let error = ConfigRequest::Discover
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
    fn dangling_symlink_is_present_but_subsequent_read_fails() {
        let directory = unique_temp_path("dangling-config");
        fs::create_dir(&directory).expect("temporary directory should be created");
        let candidate = directory.join(".dot.toml");
        std::os::unix::fs::symlink(directory.join("missing-target"), &candidate)
            .expect("dangling symlink should be created");

        let present = path_entry_exists(&candidate).expect("symlink entry should be inspectable");
        let load_error =
            LoadedConfigDocument::load(&candidate).expect_err("dangling symlink should not load");

        fs::remove_file(&candidate).expect("temporary symlink should be removed");
        fs::remove_dir(&directory).expect("temporary directory should be removed");

        assert!(present);
        match load_error {
            ConfigLoadError::Read { path, source } => {
                assert_eq!(path, candidate);
                assert_eq!(source.kind(), io::ErrorKind::NotFound);
            }
            other => panic!("expected read error, got {other:?}"),
        }
    }

    #[test]
    fn static_document_loads_config_and_absolute_path_metadata() {
        let invocation_cwd = env::current_dir().expect("test should have a current directory");
        let relative_path = Path::new("tests/fixtures/dot.toml");
        let expected_path = invocation_cwd.join(relative_path);

        let loaded = LoadedConfigDocument::load(relative_path).expect("fixture should load");

        assert_eq!(loaded.config().targets.len(), 6);
        assert_eq!(loaded.path(), expected_path);
        assert!(loaded.path().is_absolute());
        assert_eq!(loaded.directory(), expected_path.parent().unwrap());
        assert_eq!(loaded.invocation_cwd(), invocation_cwd);
    }
}

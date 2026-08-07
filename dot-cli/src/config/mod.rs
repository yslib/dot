//! Configuration source parsing, local discovery, and loading.

#![expect(
    clippy::result_large_err,
    reason = "direct typed error sources preserve Error::source downcasts"
)]

use std::env;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use directories::BaseDirs;
use url::Url;

use dot_core::config::ConfigParseError;
use dot_core::schema::Config;
use dot_core::validation::ConfigValidationError;
use dot_core::{ConfigFile, ConfigFileError};

mod git;
mod https;

pub(crate) use git::GitError;
pub(crate) use https::HttpsError;

const DEFAULT_CONFIG_FILENAME: &str = ".dot.toml";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ConfigSource {
    Path(PathBuf),
    Https(Url),
}

impl ConfigSource {
    pub(crate) fn from_os_string(value: OsString) -> Result<Self, ConfigSourceError> {
        match value.into_string() {
            Ok(value) => value.parse(),
            Err(value) => Ok(Self::Path(PathBuf::from(value))),
        }
    }
}

impl FromStr for ConfigSource {
    type Err = ConfigSourceError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if is_windows_drive_rooted(value) {
            return Ok(Self::Path(PathBuf::from(value)));
        }

        let Some(scheme) = explicit_scheme(value) else {
            return Ok(Self::Path(PathBuf::from(value)));
        };

        if scheme.eq_ignore_ascii_case("https") {
            Url::parse(value).map(Self::Https).map_err(|source| {
                ConfigSourceError::InvalidHttpsUrl {
                    value: value.to_owned(),
                    source,
                }
            })
        } else {
            Err(ConfigSourceError::UnsupportedScheme {
                scheme: scheme.to_ascii_lowercase(),
            })
        }
    }
}

fn is_windows_drive_rooted(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\')
}

fn explicit_scheme(value: &str) -> Option<&str> {
    let (scheme, _) = value.split_once("://")?;
    let mut characters = scheme.chars();
    let first = characters.next()?;
    (first.is_ascii_alphabetic()
        && characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
        }))
    .then_some(scheme)
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ConfigSourceError {
    #[error(
        "configuration source protocol `{scheme}` is not supported; use HTTPS or a filesystem path"
    )]
    UnsupportedScheme { scheme: String },
    #[error("invalid HTTPS configuration source `{value}`: {source}")]
    InvalidHttpsUrl {
        value: String,
        #[source]
        source: url::ParseError,
    },
}

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
pub(crate) enum ConfigRequest {
    Discover,
    Source(ConfigSource),
    Git {
        repository: String,
        worktree: PathBuf,
    },
}

impl ConfigRequest {
    fn resolve(self, invocation_cwd: &Path) -> Result<ConfigSource, ConfigDiscoveryError> {
        self.resolve_with(invocation_cwd, detect_user_config_root, path_entry_exists)
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
pub(crate) enum ConfigDiscoveryError {
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
        "configuration not found; checked `{}` then `{}`; use --config SOURCE to select another file",
        .local.display(),
        .user.display()
    )]
    NotFound { local: PathBuf, user: PathBuf },
}

impl ConfigRequest {
    fn resolve_with<UserRoot, Present>(
        self,
        invocation_cwd: &Path,
        user_root: UserRoot,
        mut present: Present,
    ) -> Result<ConfigSource, ConfigDiscoveryError>
    where
        UserRoot: FnOnce() -> Option<UserConfigRoot>,
        Present: FnMut(&Path) -> io::Result<bool>,
    {
        match self {
            Self::Source(source) => Ok(source),
            Self::Git { worktree, .. } => Ok(ConfigSource::Path(
                absolute_path(&worktree, invocation_cwd).join(DEFAULT_CONFIG_FILENAME),
            )),
            Self::Discover => {
                let local = invocation_cwd.join(DEFAULT_CONFIG_FILENAME);

                if present(&local).map_err(|source| ConfigDiscoveryError::Inspect {
                    path: local.clone(),
                    source,
                })? {
                    Ok(ConfigSource::Path(local))
                } else {
                    let user = user_root()
                        .ok_or(ConfigDiscoveryError::UserDirectoryUnavailable)?
                        .manifest_path();

                    if present(&user).map_err(|source| ConfigDiscoveryError::Inspect {
                        path: user.clone(),
                        source,
                    })? {
                        Ok(ConfigSource::Path(user))
                    } else {
                        Err(ConfigDiscoveryError::NotFound { local, user })
                    }
                }
            }
        }
    }
}

pub(crate) fn load_config(request: ConfigRequest) -> Result<ConfigFile, ConfigLoadError> {
    let invocation_cwd =
        env::current_dir().map_err(|source| ConfigLoadError::CurrentDirectory { source })?;
    if let ConfigRequest::Git {
        repository,
        worktree,
    } = request
    {
        let worktree = absolute_path(&worktree, &invocation_cwd);
        git::prepare(&repository, &worktree).map_err(|source| ConfigLoadError::AcquireGit {
            worktree: worktree.clone(),
            source,
        })?;
        return load_local(&worktree.join(DEFAULT_CONFIG_FILENAME), &invocation_cwd);
    }
    match request.resolve(&invocation_cwd)? {
        ConfigSource::Path(path) => load_local(&path, &invocation_cwd),
        ConfigSource::Https(url) => {
            let source = https::fetch(&url).map_err(|source| ConfigLoadError::AcquireHttps {
                url: url.clone(),
                source,
            })?;
            load_remote(&url, &source, &invocation_cwd)
        }
    }
}

fn load_local(path: &Path, invocation_cwd: &Path) -> Result<ConfigFile, ConfigLoadError> {
    let path = absolute_path(path, invocation_cwd);
    let real_path = fs::canonicalize(&path).map_err(|source| ConfigLoadError::Canonicalize {
        path: path.clone(),
        source,
    })?;
    let source = fs::read_to_string(&real_path).map_err(|source| ConfigLoadError::Read {
        path: path.clone(),
        source,
    })?;
    let config = Config::parse(&source).map_err(|error| match error {
        ConfigParseError::Deserialize { source } => ConfigLoadError::Parse {
            path: path.clone(),
            source,
        },
        ConfigParseError::Validation { source } => ConfigLoadError::Validation {
            path: path.clone(),
            source,
        },
    })?;
    let config_dir = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| invocation_cwd.to_path_buf());
    let real_config_dir = real_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| invocation_cwd.to_path_buf());

    ConfigFile::new(
        config,
        config_dir,
        real_config_dir,
        invocation_cwd.to_path_buf(),
    )
    .map_err(ConfigLoadError::from)
}

fn load_remote(
    url: &Url,
    source: &str,
    invocation_cwd: &Path,
) -> Result<ConfigFile, ConfigLoadError> {
    let config = Config::parse(source).map_err(|error| match error {
        ConfigParseError::Deserialize { source } => ConfigLoadError::RemoteParse {
            url: url.clone(),
            source,
        },
        ConfigParseError::Validation { source } => ConfigLoadError::RemoteValidation {
            url: url.clone(),
            source,
        },
    })?;

    ConfigFile::new(
        config,
        invocation_cwd.to_path_buf(),
        invocation_cwd.to_path_buf(),
        invocation_cwd.to_path_buf(),
    )
    .map_err(ConfigLoadError::from)
}

fn absolute_path(path: &Path, invocation_cwd: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        invocation_cwd.join(path)
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ConfigLoadError {
    #[error(transparent)]
    Discovery(#[from] ConfigDiscoveryError),

    #[error("failed to construct configuration context: {0}")]
    ConfigFile(#[from] ConfigFileError),

    #[error("failed to determine the invocation directory: {source}")]
    CurrentDirectory {
        #[source]
        source: io::Error,
    },
    #[error("failed to acquire configuration from `{url}`: {source}")]
    AcquireHttps {
        url: Url,
        #[source]
        source: HttpsError,
    },
    #[error(
        "failed to prepare Git configuration worktree `{}`: {source}",
        .worktree.display()
    )]
    AcquireGit {
        worktree: PathBuf,
        #[source]
        source: GitError,
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
    #[error("failed to parse configuration from `{url}`: {source}")]
    RemoteParse {
        url: Url,
        #[source]
        source: toml::de::Error,
    },
    #[error("failed to validate configuration from `{url}`: {source}")]
    RemoteValidation {
        url: Url,
        #[source]
        source: ConfigValidationError,
    },
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use dot_core::interpolation::InterpolationError;
    use dot_core::schema::{Identifier, SelectorIdentifier};
    use dot_core::validation::{ConfigValidationErrorKind, ConfigValidationJob};

    use super::*;

    #[test]
    fn classifies_config_sources_by_explicit_scheme() {
        enum Expected<'a> {
            Path(&'a str),
            Https(&'a str),
            Unsupported(&'a str),
            InvalidHttps,
        }

        let cases = [
            ("config/dot.toml", Expected::Path("config/dot.toml")),
            ("/etc/dot/dot.toml", Expected::Path("/etc/dot/dot.toml")),
            (
                r"C:\Users\alice\dot.toml",
                Expected::Path(r"C:\Users\alice\dot.toml"),
            ),
            (
                "C:/Users/alice/dot.toml",
                Expected::Path("C:/Users/alice/dot.toml"),
            ),
            (
                "C://Users/alice/.dot.toml",
                Expected::Path("C://Users/alice/.dot.toml"),
            ),
            (
                "HTTPS://example.com/dot.toml",
                Expected::Https("https://example.com/dot.toml"),
            ),
            ("http://example.com/dot.toml", Expected::Unsupported("http")),
            ("file:///etc/dot.toml", Expected::Unsupported("file")),
            ("https://[::1", Expected::InvalidHttps),
        ];

        for (input, expected) in cases {
            let actual = input.parse::<ConfigSource>();
            match expected {
                Expected::Path(path) => assert_eq!(
                    actual.expect("source should be a path"),
                    ConfigSource::Path(PathBuf::from(path)),
                    "input: {input}"
                ),
                Expected::Https(url) => assert_eq!(
                    actual.expect("source should be HTTPS"),
                    ConfigSource::Https(Url::parse(url).expect("expected URL should parse")),
                    "input: {input}"
                ),
                Expected::Unsupported(scheme) => assert!(
                    matches!(
                        actual,
                        Err(ConfigSourceError::UnsupportedScheme {
                            scheme: actual_scheme,
                        }) if actual_scheme == scheme
                    ),
                    "input: {input}"
                ),
                Expected::InvalidHttps => assert!(
                    matches!(actual, Err(ConfigSourceError::InvalidHttpsUrl { .. })),
                    "input: {input}"
                ),
            }
        }
    }

    fn fixture_path(relative: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("dot-cli should be inside the workspace")
            .join("tests/fixtures")
            .join(relative)
    }

    #[test]
    fn remote_parse_and_validation_errors_name_the_source_url() {
        let url =
            Url::parse("https://example.com/config/dot.toml").expect("test URL should be valid");
        let cwd = env::current_dir().expect("test should have a current directory");

        let parse = load_remote(&url, "[targets", &cwd)
            .expect_err("invalid TOML should fail remote loading");
        assert!(matches!(
            &parse,
            ConfigLoadError::RemoteParse { url: actual, .. } if actual == &url
        ));
        assert!(parse.to_string().contains(url.as_str()));

        let validation = load_remote(
            &url,
            r#"[targets.machine]
platform = { os = "linux" }

[targets.machine.profiles.desktop.profiles.shared]
[targets.machine.profiles.server.profiles.shared]
"#,
            &cwd,
        )
        .expect_err("invalid configuration should fail remote loading");
        assert!(matches!(
            &validation,
            ConfigLoadError::RemoteValidation { url: actual, .. } if actual == &url
        ));
        assert!(validation.to_string().contains(url.as_str()));
    }

    #[test]
    fn remote_loading_uses_the_absolute_invocation_directory_for_all_context() {
        let url =
            Url::parse("https://example.com/config/dot.toml").expect("test URL should be valid");
        let cwd = env::current_dir().expect("test should have a current directory");
        let loaded = load_remote(
            &url,
            r#"[targets.machine]
platform = { os = ["linux", "macos", "windows"] }
"#,
            &cwd,
        )
        .expect("valid remote configuration should load");

        assert!(cwd.is_absolute());
        assert_eq!(loaded.config_dir(), cwd);
        assert_eq!(loaded.real_config_dir(), cwd);
        assert_eq!(loaded.cwd(), cwd);
    }

    fn unique_temp_path(label: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after the Unix epoch")
            .as_nanos();
        env::temp_dir().join(format!("dot-{label}-{}-{nonce}", std::process::id()))
    }

    fn path_request(path: PathBuf) -> ConfigRequest {
        ConfigRequest::Source(ConfigSource::Path(path))
    }

    #[test]
    fn explicit_request_bypasses_discovery() {
        let requested = PathBuf::from("relative/config.toml");

        let resolved = path_request(requested.clone())
            .resolve_with(
                Path::new("/unused"),
                || panic!("user root must not be queried"),
                |_| -> io::Result<bool> { panic!("candidate must not be inspected") },
            )
            .expect("explicit request should resolve");

        assert_eq!(resolved, ConfigSource::Path(requested));
    }

    #[test]
    fn explicit_resolve_preserves_the_requested_path() {
        let requested = PathBuf::from("relative/config.toml");

        let resolved = path_request(requested.clone())
            .resolve(Path::new("/unused"))
            .expect("explicit request should resolve");

        assert_eq!(resolved, ConfigSource::Path(requested));
    }

    #[test]
    fn local_candidate_wins_without_querying_user_root() {
        let invocation_cwd = PathBuf::from("/work");
        let expected = invocation_cwd.join(".dot.toml");

        let resolved = ConfigRequest::Discover
            .resolve_with(
                &invocation_cwd,
                || panic!("user root must not be queried"),
                |candidate| {
                    assert_eq!(candidate, expected);
                    Ok(true)
                },
            )
            .expect("local candidate should resolve");

        assert_eq!(resolved, ConfigSource::Path(expected));
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
                &invocation_cwd,
                || Some(UserConfigRoot::Home(home)),
                |candidate| {
                    inspected.push(candidate.to_path_buf());
                    Ok(candidate == user)
                },
            )
            .expect("user candidate should resolve");

        assert_eq!(resolved, ConfigSource::Path(user.clone()));
        assert_eq!(inspected, vec![local, user]);
    }

    #[test]
    fn local_inspection_error_stops_before_user_detection() {
        let invocation_cwd = PathBuf::from("/work");
        let local = invocation_cwd.join(".dot.toml");

        let error = ConfigRequest::Discover
            .resolve_with(
                &invocation_cwd,
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
                &invocation_cwd,
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
    fn missing_candidates_report_both_paths_in_probe_order() {
        let invocation_cwd = PathBuf::from("/work");
        let local = invocation_cwd.join(".dot.toml");
        let home = PathBuf::from("/home/alice");
        let user = home.join(".config").join("dot").join(".dot.toml");

        let error = ConfigRequest::Discover
            .resolve_with(
                &invocation_cwd,
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
                "configuration not found; checked `{}` then `{}`; use --config SOURCE to select another file",
                local.display(),
                user.display()
            )
        );
        assert!(error.to_string().contains("--config SOURCE"));
    }

    #[test]
    fn discovery_errors_have_exact_messages() {
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
    fn discovery_inspect_error_exposes_its_io_source() {
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
        let load_error = load_config(path_request(candidate.clone()))
            .expect_err("dangling symlink should not load");

        fs::remove_file(&candidate).expect("temporary symlink should be removed");
        fs::remove_dir(&directory).expect("temporary directory should be removed");

        assert!(present);
        match load_error {
            ConfigLoadError::Canonicalize { path, source } => {
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

        let result = load_config(path_request(entry.clone()));

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
            ConfigLoadError::Read { path, source } => {
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
        let expected_path = fixture_path("dot.toml");
        let expected_real_path =
            fs::canonicalize(&expected_path).expect("fixture path should canonicalize");

        let loaded = load_config(path_request(expected_path.clone())).expect("fixture should load");

        assert_eq!(loaded.config().targets.len(), 6);
        assert_eq!(loaded.config_dir(), expected_path.parent().unwrap());
        assert!(loaded.config_dir().is_absolute());
        assert_eq!(
            loaded.real_config_dir(),
            expected_real_path
                .parent()
                .expect("canonical fixture should have a parent")
        );
        assert_eq!(loaded.cwd(), invocation_cwd);
    }

    #[cfg(unix)]
    #[test]
    fn static_document_distinguishes_a_symlink_entry_from_the_real_config() {
        let directory = unique_temp_path("config-symlink");
        fs::create_dir(&directory).expect("temporary directory should be created");
        let entry = directory.join(".dot.toml");
        let real = fs::canonicalize(fixture_path("dot.toml")).expect("fixture should canonicalize");
        std::os::unix::fs::symlink(&real, &entry).expect("configuration symlink should be created");

        let result = load_config(path_request(entry.clone()));

        fs::remove_file(&entry).expect("temporary symlink should be removed");
        fs::remove_dir(&directory).expect("temporary directory should be removed");

        let loaded = result.expect("configuration symlink should load");
        assert_eq!(loaded.config_dir(), directory);
        assert_eq!(loaded.real_config_dir(), real.parent().unwrap());
    }

    #[test]
    fn relative_paths_are_made_absolute_against_the_invocation_directory() {
        let invocation_cwd = Path::new("/work");

        assert_eq!(
            absolute_path(Path::new("config/dot.toml"), invocation_cwd),
            invocation_cwd.join("config/dot.toml")
        );
    }

    #[test]
    fn relative_manifest_loads_with_absolute_protocol_context() {
        let invocation_cwd = env::current_dir().expect("test should have a current directory");
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after the Unix epoch")
            .as_nanos();
        let relative_dir = PathBuf::from("target").join(format!(
            "dot-relative-config-{}-{nonce}",
            std::process::id()
        ));
        let absolute_dir = invocation_cwd.join(&relative_dir);
        fs::create_dir_all(&absolute_dir).expect("temporary directory should be created");
        let relative_path = relative_dir.join("dot.toml");
        let absolute_path = absolute_dir.join("dot.toml");
        fs::write(
            &absolute_path,
            r#"[targets.machine]
platform = { os = ["linux", "macos", "windows"] }
"#,
        )
        .expect("test manifest should be written");

        let result = load_config(path_request(relative_path));
        let real_dir =
            fs::canonicalize(&absolute_dir).expect("temporary directory should canonicalize");
        fs::remove_dir_all(&absolute_dir).expect("temporary directory should be removed");

        let loaded = result.expect("relative manifest should load");
        assert_eq!(loaded.config_dir(), absolute_dir);
        assert_eq!(loaded.real_config_dir(), real_dir);
        assert_eq!(loaded.cwd(), invocation_cwd);
    }

    #[test]
    fn missing_manifest_reports_the_requested_absolute_entry_path() {
        let missing = unique_temp_path("missing-manifest");

        let error =
            load_config(path_request(missing.clone())).expect_err("missing manifest should fail");

        match &error {
            ConfigLoadError::Canonicalize { path, source } => {
                assert_eq!(path, &missing);
                assert_eq!(source.kind(), io::ErrorKind::NotFound);
            }
            other => panic!("expected canonicalize error, got {other:?}"),
        }
        assert!(
            error
                .to_string()
                .contains(missing.to_string_lossy().as_ref())
        );
        assert!(Error::source(&error).is_some());
    }

    #[test]
    fn invalid_documents_and_manifests_report_the_requested_absolute_path() {
        let invalid_document = fixture_path("config/invalid-syntax.toml");
        let parse_error = load_config(path_request(invalid_document.clone()))
            .expect_err("invalid TOML should fail");
        assert!(matches!(
            parse_error,
            ConfigLoadError::Parse { ref path, .. } if path == &invalid_document
        ));

        let invalid_manifest = fixture_path("manifest/invalid-duplicate-profile-name.toml");
        let validation_error = load_config(path_request(invalid_manifest.clone()))
            .expect_err("invalid manifest should fail");
        assert!(matches!(
            validation_error,
            ConfigLoadError::Validation { ref path, .. } if path == &invalid_manifest
        ));
    }

    fn test_validation_error() -> ConfigValidationError {
        ConfigValidationError {
            target: SelectorIdentifier::new("target").expect("test target should be valid"),
            profile: None,
            job: Some(ConfigValidationJob::Provider(
                Identifier::new("provider").expect("test provider should be valid"),
            )),
            field: Some("field".to_owned()),
            kind: ConfigValidationErrorKind::Expression(InterpolationError::UnclosedResolver {
                offset: 0,
            }),
        }
    }

    #[test]
    fn load_errors_preserve_their_typed_immediate_sources() {
        let io_errors = [
            ConfigLoadError::CurrentDirectory {
                source: io::Error::other("test I/O failure"),
            },
            ConfigLoadError::Canonicalize {
                path: PathBuf::from("dot.toml"),
                source: io::Error::other("test I/O failure"),
            },
            ConfigLoadError::Read {
                path: PathBuf::from("dot.toml"),
                source: io::Error::other("test I/O failure"),
            },
        ];
        for error in &io_errors {
            assert!(
                Error::source(error)
                    .and_then(|source| source.downcast_ref::<io::Error>())
                    .is_some()
            );
        }

        let parse = ConfigLoadError::Parse {
            path: PathBuf::from("dot.toml"),
            source: toml::from_str::<toml::Value>("invalid = [")
                .expect_err("test TOML should be invalid"),
        };
        assert!(
            Error::source(&parse)
                .and_then(|source| source.downcast_ref::<toml::de::Error>())
                .is_some()
        );

        let validation = ConfigLoadError::Validation {
            path: PathBuf::from("dot.toml"),
            source: test_validation_error(),
        };
        let validation_source = Error::source(&validation)
            .and_then(|source| source.downcast_ref::<ConfigValidationError>())
            .expect("validation error should be the immediate source");
        assert!(
            Error::source(validation_source)
                .and_then(|source| source.downcast_ref::<ConfigValidationErrorKind>())
                .is_some()
        );

        let protocol = ConfigLoadError::ConfigFile(ConfigFileError::RelativeCwd {
            path: PathBuf::from("relative"),
        });
        assert!(
            Error::source(&protocol)
                .and_then(|source| source.downcast_ref::<ConfigFileError>())
                .is_some()
        );
    }
}

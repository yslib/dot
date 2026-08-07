//! Configuration source parsing, local discovery, and loading.

#![expect(
    clippy::result_large_err,
    reason = "typed loading errors preserve precise source and path context without boxing"
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

    #[test]
    fn remote_errors_name_the_source_url() {
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
    fn remote_loading_uses_the_invocation_directory_for_all_context() {
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

    #[test]
    fn user_roots_construct_each_platform_manifest_path() {
        let home = PathBuf::from("/home/alice");
        assert_eq!(
            UserConfigRoot::Home(home.clone()).manifest_path(),
            home.join(".config").join("dot").join(".dot.toml")
        );

        let roaming = PathBuf::from(r"C:\Users\alice\AppData\Roaming");
        assert_eq!(
            UserConfigRoot::Roaming(roaming.clone()).manifest_path(),
            roaming.join("dot").join(".dot.toml")
        );
    }
}

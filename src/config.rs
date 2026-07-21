use std::env;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::action::ExecutionEnvironment;
use crate::schema::Config;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedConfig {
    config: Config,
    path: PathBuf,
    directory: PathBuf,
    invocation_cwd: PathBuf,
    environment: ExecutionEnvironment,
}

impl LoadedConfig {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigLoadError> {
        let invocation_cwd =
            env::current_dir().map_err(|source| ConfigLoadError::CurrentDirectory { source })?;
        let environment = ExecutionEnvironment::capture();
        let path = absolute_path(path.as_ref(), &invocation_cwd);
        let source = fs::read_to_string(&path).map_err(|source| ConfigLoadError::Read {
            path: path.clone(),
            source,
        })?;
        let config = toml::from_str(&source).map_err(|source| ConfigLoadError::Parse {
            path: path.clone(),
            source,
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
            environment,
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

    pub fn invocation_cwd(&self) -> &Path {
        &self.invocation_cwd
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
        }
    }
}

impl Error for ConfigLoadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::CurrentDirectory { source } | Self::Read { source, .. } => Some(source),
            Self::Parse { source, .. } => Some(source),
        }
    }
}

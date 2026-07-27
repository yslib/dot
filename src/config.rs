use std::env;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::action::ExecutionEnvironment;
use crate::schema::Config;
use crate::validation::{ConfigValidationError, validate_config};

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

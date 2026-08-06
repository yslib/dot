//! Source-independent structured configuration and its path context.

use std::path::{Path, PathBuf};

use crate::schema::Config;

/// A parsed configuration together with the absolute directories used to resolve it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigFile {
    config: Config,
    config_dir: PathBuf,
    real_config_dir: PathBuf,
    cwd: PathBuf,
}

impl ConfigFile {
    /// Constructs a configuration protocol value from owned, absolute path context.
    ///
    /// This constructor performs no filesystem access or path transformation.
    pub fn new(
        config: Config,
        config_dir: PathBuf,
        real_config_dir: PathBuf,
        cwd: PathBuf,
    ) -> Result<Self, ConfigFileError> {
        if config_dir.is_relative() {
            return Err(ConfigFileError::RelativeConfigDir { path: config_dir });
        }
        if real_config_dir.is_relative() {
            return Err(ConfigFileError::RelativeRealConfigDir {
                path: real_config_dir,
            });
        }
        if cwd.is_relative() {
            return Err(ConfigFileError::RelativeCwd { path: cwd });
        }

        Ok(Self {
            config,
            config_dir,
            real_config_dir,
            cwd,
        })
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    pub fn real_config_dir(&self) -> &Path {
        &self.real_config_dir
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }
}

/// A path invariant violation while constructing [`ConfigFile`].
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ConfigFileError {
    #[error("configuration directory must be absolute: `{}`", .path.display())]
    RelativeConfigDir { path: PathBuf },
    #[error("real configuration directory must be absolute: `{}`", .path.display())]
    RelativeRealConfigDir { path: PathBuf },
    #[error("invocation directory must be absolute: `{}`", .path.display())]
    RelativeCwd { path: PathBuf },
}

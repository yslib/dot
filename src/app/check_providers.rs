use std::error::Error;
use std::fmt;
use std::io::{self, Write};

use super::Selection;
use crate::check::ProviderChecker;
use crate::config::{ConfigLoadError, LoadedConfig};
use crate::interpolation::{DotPaths, XdgPaths};
use crate::manifest::{EffectiveManifest, ManifestError};
use crate::platform::PlatformInfo;

pub(super) fn run(selection: &Selection, output: &mut impl Write) -> Result<bool, CommandError> {
    let loaded = LoadedConfig::load(&selection.config)?;
    let platform = PlatformInfo::detect();
    let manifest = EffectiveManifest::select(
        loaded.config(),
        &platform,
        selection.target.as_deref(),
        selection.profile.as_deref(),
    )?;
    let xdg_paths = XdgPaths::detect();
    let dot_paths = DotPaths::new(loaded.path(), loaded.directory(), loaded.invocation_cwd());
    let checker = ProviderChecker::new(loaded.environment(), dot_paths, &xdg_paths);
    let report = checker.check(manifest.providers());

    writeln!(output, "{report}")?;
    Ok(report.all_ready())
}

#[derive(Debug)]
pub(super) enum CommandError {
    Config(ConfigLoadError),
    Manifest(ManifestError),
    Output(io::Error),
}

impl fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(source) => source.fmt(formatter),
            Self::Manifest(source) => source.fmt(formatter),
            Self::Output(source) => {
                write!(formatter, "failed to write provider check output: {source}")
            }
        }
    }
}

impl Error for CommandError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Config(source) => Some(source),
            Self::Manifest(source) => Some(source),
            Self::Output(source) => Some(source),
        }
    }
}

impl From<ConfigLoadError> for CommandError {
    fn from(source: ConfigLoadError) -> Self {
        Self::Config(source)
    }
}

impl From<ManifestError> for CommandError {
    fn from(source: ManifestError) -> Self {
        Self::Manifest(source)
    }
}

impl From<io::Error> for CommandError {
    fn from(source: io::Error) -> Self {
        Self::Output(source)
    }
}

use std::error::Error;
use std::fmt;

use super::Selection;
use crate::config::{ConfigLoadError, LoadedConfig};
use crate::dry_run::build_report;
use crate::interpolation::{DotPaths, XdgPaths};
use crate::manifest::{EffectiveManifest, ManifestError};
use crate::plan::{ExecutionPlanner, PlanningError};
use crate::platform::PlatformInfo;
use crate::report::CommandReport;

pub(super) fn run(
    selection: &Selection,
    platform_override: Option<&PlatformInfo>,
) -> Result<CommandReport, CommandError> {
    let loaded = LoadedConfig::load(&selection.config)?;
    let platform = platform_override
        .cloned()
        .unwrap_or_else(PlatformInfo::detect);
    let manifest = EffectiveManifest::select(
        loaded.config(),
        &platform,
        selection.target.as_deref(),
        selection.profile.as_deref(),
    )?;
    let xdg_paths = XdgPaths::detect();
    let dot_paths = DotPaths::new(loaded.path(), loaded.directory(), loaded.invocation_cwd());
    let planner = ExecutionPlanner::new(loaded.environment(), dot_paths, &xdg_paths, &platform);
    let plan = planner.plan(&manifest)?;

    Ok(build_report(loaded.path(), &plan))
}

#[derive(Debug)]
pub(super) enum CommandError {
    Config(ConfigLoadError),
    Manifest(ManifestError),
    Planning(PlanningError),
}

impl fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(source) => source.fmt(formatter),
            Self::Manifest(source) => source.fmt(formatter),
            Self::Planning(source) => source.fmt(formatter),
        }
    }
}

impl Error for CommandError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Config(source) => Some(source),
            Self::Manifest(source) => Some(source),
            Self::Planning(source) => Some(source),
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

impl From<PlanningError> for CommandError {
    fn from(source: PlanningError) -> Self {
        Self::Planning(source)
    }
}

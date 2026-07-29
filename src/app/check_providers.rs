use super::ScopeSelection;
use crate::check::{ProviderChecker, build_report};
use crate::config::{ConfigLoadError, LoadedConfig};
use crate::interpolation::{DotPaths, XdgPaths};
use crate::manifest::{EffectiveManifest, ManifestError};
use crate::platform::PlatformInfo;
use crate::report::CommandReport;

pub(super) fn run(
    config: &std::path::Path,
    scope: &ScopeSelection,
    platform_override: Option<&PlatformInfo>,
) -> Result<CommandReport, CommandError> {
    let loaded = LoadedConfig::load(config)?;
    let platform = platform_override
        .cloned()
        .unwrap_or_else(PlatformInfo::detect);
    let manifest = EffectiveManifest::select_for_execution(
        loaded.config(),
        &platform,
        scope.target.as_ref(),
        scope.profile.named(),
    )?;
    let xdg_paths = XdgPaths::detect();
    let dot_paths = DotPaths::new(loaded.path(), loaded.directory(), loaded.invocation_cwd());
    let checker = ProviderChecker::new(loaded.environment(), dot_paths, &xdg_paths);
    let checks = checker.check(manifest.providers());

    Ok(build_report(
        loaded.path(),
        manifest.target(),
        manifest.profile(),
        &platform,
        manifest.providers(),
        &checks,
    ))
}

#[derive(Debug, thiserror::Error)]
pub(super) enum CommandError {
    #[error("{0}")]
    Config(#[from] ConfigLoadError),

    #[error("{0}")]
    Manifest(#[from] ManifestError),
}

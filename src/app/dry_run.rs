use super::{ExecutionCommandError, ExecutionRequest};
use crate::config::LoadedConfig;
use crate::dry_run::build_report;
use crate::interpolation::{DotPaths, XdgPaths};
use crate::manifest::EffectiveManifest;
use crate::plan::ExecutionPlanner;
use crate::platform::PlatformInfo;
use crate::report::CommandReport;

pub(super) fn run(
    config: &std::path::Path,
    request: &ExecutionRequest,
    platform_override: Option<&PlatformInfo>,
) -> Result<CommandReport, ExecutionCommandError> {
    let loaded = LoadedConfig::load(config)?;
    let platform = platform_override
        .cloned()
        .unwrap_or_else(PlatformInfo::detect);
    let manifest = EffectiveManifest::select_for_execution(
        loaded.config(),
        &platform,
        request.scope.target.as_ref(),
        request.scope.profile.named(),
    )?;
    let xdg_paths = XdgPaths::detect();
    let dot_paths = DotPaths::new(loaded.path(), loaded.directory(), loaded.invocation_cwd());
    let planner = ExecutionPlanner::new(loaded.environment(), dot_paths, &xdg_paths, &platform);
    let plan = planner.plan(&manifest, &request.jobs)?;

    Ok(build_report(loaded.path(), &plan))
}

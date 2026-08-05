//! Provider readiness inspection and report projection.

use crate::interpolation::{
    DotPaths, ExecutionEnvironment, InterpolationError, ResolveContext, XdgPaths,
    resolve_environment_patch, resolve_exec_action,
};
use crate::manifest::{EffectiveManifest, ManifestError};
use crate::platform::PlatformInfo;
use crate::report::{
    CommandInfo, CommandReport, Evidence, EvidenceStage, ItemStatus, ProviderItem, ReportCommand,
    ReportContext, ReportItem, ReportStatus, ReportSubject,
};
use crate::schema::{Entries, OneOrMany, Provider};
use crate::selection::ScopeSelection;

use super::config_file::ConfigFile;
use super::process::{
    CommandPreparationError, ExecutionError, ExecutionResult, IoMode, PreparedCommand,
    ProcessExecutor, apply_environment_patch,
};
use super::runtime::NativeRuntime;

pub fn check_providers(
    config: &ConfigFile,
    runtime: &NativeRuntime,
    compatibility_platform: &PlatformInfo,
    scope: &ScopeSelection,
) -> Result<CommandReport, ProviderCheckError> {
    let manifest = EffectiveManifest::select_for_execution(
        config.config(),
        compatibility_platform,
        scope.target.as_ref(),
        scope.profile.named(),
    )?;
    let checker = ProviderChecker::new(
        runtime.environment(),
        DotPaths::from(config),
        runtime.xdg_paths(),
    );
    let checks = checker.check(manifest.providers());

    Ok(build_report(
        config.path(),
        manifest.target(),
        manifest.profile(),
        compatibility_platform,
        manifest.providers(),
        &checks,
    ))
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderCheckError {
    #[error(transparent)]
    Manifest(#[from] ManifestError),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderReadiness {
    Ready,
    NotReady,
}

#[derive(Debug)]
pub struct ProviderCheckResult {
    provider: String,
    outcome: Result<ExecutionResult, ProviderProbeError>,
}

impl ProviderCheckResult {
    pub fn provider(&self) -> &str {
        &self.provider
    }

    pub fn readiness(&self) -> ProviderReadiness {
        match &self.outcome {
            Ok(output) if output.success() => ProviderReadiness::Ready,
            Ok(_) | Err(_) => ProviderReadiness::NotReady,
        }
    }

    pub fn output(&self) -> Option<&ExecutionResult> {
        self.outcome.as_ref().ok()
    }

    pub fn error(&self) -> Option<&ProviderProbeError> {
        self.outcome.as_ref().err()
    }
}

#[derive(Debug, Default)]
pub struct ProviderCheckReport {
    results: Vec<ProviderCheckResult>,
}

impl ProviderCheckReport {
    pub fn results(&self) -> &[ProviderCheckResult] {
        &self.results
    }

    pub fn all_ready(&self) -> bool {
        self.results
            .iter()
            .all(|result| result.readiness() == ProviderReadiness::Ready)
    }
}

pub fn build_report(
    config: &std::path::Path,
    target: &str,
    profile: Option<&str>,
    platform: &PlatformInfo,
    providers: &Entries<Provider>,
    checks: &ProviderCheckReport,
) -> CommandReport {
    let items = checks
        .results()
        .iter()
        .map(|result| {
            let provider = providers
                .get(result.provider())
                .expect("provider check results must originate from the effective providers");
            ReportItem {
                id: result.provider().to_owned(),
                status: match result.readiness() {
                    ProviderReadiness::Ready => ItemStatus::Ready,
                    ProviderReadiness::NotReady => ItemStatus::NotReady,
                },
                subject: ReportSubject::Provider(ProviderItem {
                    probe: CommandInfo::from_source(&provider.probe),
                    ensure: provider.ensure.as_ref().map(commands).unwrap_or_default(),
                    has_activation: provider.activate.is_some(),
                }),
                evidence: vec![check_evidence(result)],
            }
        })
        .collect();

    CommandReport {
        command: ReportCommand::CheckProviders,
        context: ReportContext {
            config: config.to_owned(),
            target: target.to_owned(),
            profile: profile.map(str::to_owned),
            platform: platform.clone(),
        },
        status: if checks.all_ready() {
            ReportStatus::Succeeded
        } else {
            ReportStatus::Failed
        },
        items,
        diagnostics: Vec::new(),
    }
}

fn commands(actions: &OneOrMany<crate::schema::SourceExecAction>) -> Vec<CommandInfo> {
    match actions {
        OneOrMany::One(action) => vec![CommandInfo::from_source(action)],
        OneOrMany::Many(actions) => actions.iter().map(CommandInfo::from_source).collect(),
    }
}

fn check_evidence(result: &ProviderCheckResult) -> Evidence {
    match &result.outcome {
        Ok(output) => Evidence {
            stage: EvidenceStage::Probe,
            exit_code: output.code(),
            message: None,
            stdout: captured_text(output.stdout()),
            stderr: captured_text(output.stderr()),
            hints: Vec::new(),
        },
        Err(error) => Evidence {
            stage: EvidenceStage::Probe,
            exit_code: None,
            message: Some(error.to_string()),
            stdout: None,
            stderr: None,
            hints: Vec::new(),
        },
    }
}

fn captured_text(output: Option<&[u8]>) -> Option<String> {
    output
        .filter(|output| !output.is_empty())
        .map(|output| String::from_utf8_lossy(output).into_owned())
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderProbeError {
    #[error("failed to resolve provider activate: {0}")]
    ActivateInterpolation(#[source] InterpolationError),
    #[error("failed to apply provider activate: {0}")]
    ActivatePreparation(#[source] CommandPreparationError),
    #[error("failed to resolve provider probe: {0}")]
    ProbeInterpolation(#[source] InterpolationError),
    #[error("failed to prepare provider probe: {0}")]
    ProbePreparation(#[source] CommandPreparationError),
    #[error("failed to execute probe: {0}")]
    Execution(#[from] ExecutionError),
}

#[derive(Clone, Copy, Debug)]
pub struct ProviderChecker<'a> {
    base_environment: &'a ExecutionEnvironment,
    dot_paths: DotPaths<'a>,
    xdg_paths: &'a XdgPaths,
}

impl<'a> ProviderChecker<'a> {
    pub const fn new(
        base_environment: &'a ExecutionEnvironment,
        dot_paths: DotPaths<'a>,
        xdg_paths: &'a XdgPaths,
    ) -> Self {
        Self {
            base_environment,
            dot_paths,
            xdg_paths,
        }
    }

    pub fn check(&self, providers: &Entries<Provider>) -> ProviderCheckReport {
        let results = providers
            .iter()
            .map(|(provider_id, provider)| ProviderCheckResult {
                provider: provider_id.to_string(),
                outcome: self.check_one(provider),
            })
            .collect();
        ProviderCheckReport { results }
    }

    fn check_one(&self, provider: &Provider) -> Result<ExecutionResult, ProviderProbeError> {
        let mut environment = self.base_environment.clone();

        if let Some(activate) = &provider.activate {
            let activate = {
                let context = ResolveContext::new(&environment, self.dot_paths, self.xdg_paths);
                resolve_environment_patch(activate, &context)
                    .map_err(ProviderProbeError::ActivateInterpolation)?
            };
            apply_environment_patch(&mut environment, &activate)
                .map_err(ProviderProbeError::ActivatePreparation)?;
        }

        let probe = {
            let context = ResolveContext::new(&environment, self.dot_paths, self.xdg_paths);
            resolve_exec_action(&provider.probe, &context)
                .map_err(ProviderProbeError::ProbeInterpolation)?
        };
        let command = PreparedCommand::from_exec_action(&probe, &environment)
            .map_err(ProviderProbeError::ProbePreparation)?;
        ProcessExecutor::new()
            .execute(&command, IoMode::Capture)
            .map_err(ProviderProbeError::from)
    }
}

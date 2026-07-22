use std::error::Error;
use std::fmt;
use std::path::Path;

use super::Selection;
use crate::action::{ExecutionEnvironment, ExecutionResult};
use crate::action_runner::{ActionOutcome, ActionRunError, ActionRunner, ActionStage};
use crate::config::{ConfigLoadError, LoadedConfig};
use crate::diagnostic::lookup;
use crate::interpolation::{DotPaths, XdgPaths};
use crate::link::{self, LinkOutcome, LinkPhaseError, LinkReport};
use crate::manifest::{EffectiveManifest, ManifestError};
use crate::plan::{ExecutionPlan, ExecutionPlanner, PlanningError};
use crate::platform::PlatformInfo;
use crate::provider::{
    ProviderBatchExecution, ProviderBatchOutcome, ProviderError, ProviderOutcome,
    ProviderReadiness, ProviderRunner, ProviderStage,
};
use crate::report::{
    ActionInfo, ActionItem, CommandInfo, CommandReport, Diagnostic, DiagnosticLevel, Evidence,
    EvidenceStage, ItemStatus, LinkItem, PackageItem, PackageSource, ProviderItem, ReportCommand,
    ReportContext, ReportItem, ReportStatus, ReportSubject,
};

pub(super) fn run(selection: &Selection) -> Result<CommandReport, CommandError> {
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
    let planner = ExecutionPlanner::new(loaded.environment(), dot_paths, &xdg_paths, &platform);
    let plan = planner.plan(&manifest)?;
    let result = execute(&plan, loaded.environment());

    Ok(build_report(loaded.path(), &plan, &result))
}

fn execute(plan: &ExecutionPlan, environment: &ExecutionEnvironment) -> ApplyResult {
    let provider_runner = ProviderRunner::new(environment);
    let providers = provider_runner.ensure_all(plan.providers());
    let provider_batches = provider_runner.install_batches(plan.provider_installs(), &providers);
    let action_runner = ActionRunner::new(environment);
    let manual_packages = plan
        .manual_packages()
        .iter()
        .map(|package| NamedActionResult {
            id: package.id().to_owned(),
            outcome: action_runner.run(package.install()),
        })
        .collect();
    let actions = plan
        .actions()
        .iter()
        .map(|action| NamedActionResult {
            id: action.id().to_owned(),
            outcome: action_runner.run(action.action()),
        })
        .collect();
    let links = link::reconcile(plan.links());

    ApplyResult {
        providers,
        provider_batches,
        manual_packages,
        actions,
        links,
    }
}

#[derive(Debug)]
struct NamedActionResult {
    id: String,
    outcome: Result<ActionOutcome, ActionRunError>,
}

#[derive(Debug)]
struct ApplyResult {
    providers: ProviderReadiness,
    provider_batches: ProviderBatchExecution,
    manual_packages: Vec<NamedActionResult>,
    actions: Vec<NamedActionResult>,
    links: Result<LinkReport, LinkPhaseError>,
}

impl ApplyResult {
    fn succeeded(&self) -> bool {
        self.providers.all_ready()
            && self.provider_batches.all_succeeded()
            && self
                .manual_packages
                .iter()
                .all(|result| result.outcome.is_ok())
            && self.actions.iter().all(|result| result.outcome.is_ok())
            && self
                .links
                .as_ref()
                .is_ok_and(|report| report.all_succeeded())
    }
}

fn build_report(config: &Path, plan: &ExecutionPlan, result: &ApplyResult) -> CommandReport {
    let mut items = provider_items(plan, &result.providers);
    items.extend(provider_package_items(&result.provider_batches));
    items.extend(action_items(plan, &result.manual_packages, &result.actions));
    let (links, diagnostics) = link_items(plan, &result.links);
    items.extend(links);

    CommandReport {
        command: ReportCommand::Apply,
        context: ReportContext {
            config: config.to_owned(),
            target: plan.target().to_owned(),
            profile: plan.profile().map(str::to_owned),
            platform: plan.platform().clone(),
        },
        status: if result.succeeded() {
            ReportStatus::Succeeded
        } else {
            ReportStatus::Failed
        },
        items,
        diagnostics,
    }
}

fn provider_items(plan: &ExecutionPlan, readiness: &ProviderReadiness) -> Vec<ReportItem> {
    plan.providers()
        .iter()
        .map(|provider| {
            let result = readiness
                .get(provider.id())
                .expect("provider results must match the execution plan");
            let (status, evidence) = match result.outcome() {
                Ok(ProviderOutcome::AlreadyReady { probe }) => (
                    ItemStatus::Ready,
                    vec![execution_evidence(
                        EvidenceStage::Probe,
                        probe,
                        Some("already available"),
                    )],
                ),
                Ok(ProviderOutcome::Ensured { ensure, probe }) => {
                    let mut evidence = ensure
                        .iter()
                        .map(|result| execution_evidence(EvidenceStage::Ensure, result, None))
                        .collect::<Vec<_>>();
                    evidence.push(execution_evidence(
                        EvidenceStage::Probe,
                        probe,
                        Some("installed and verified"),
                    ));
                    (ItemStatus::Ready, evidence)
                }
                Err(error) => (ItemStatus::NotReady, vec![provider_error_evidence(error)]),
            };
            ReportItem {
                id: provider.id().to_owned(),
                status,
                subject: ReportSubject::Provider(ProviderItem {
                    probe: CommandInfo::from(provider.probe()),
                    ensure: provider.ensure().iter().map(CommandInfo::from).collect(),
                    has_activation: provider.activate().is_some(),
                }),
                evidence,
            }
        })
        .collect()
}

fn provider_package_items(execution: &ProviderBatchExecution) -> Vec<ReportItem> {
    execution
        .statuses()
        .iter()
        .flat_map(|batch| {
            let (status, evidence) = match batch.outcome() {
                Ok(ProviderBatchOutcome::Executed { install }) => (
                    ItemStatus::Installed,
                    vec![execution_evidence(EvidenceStage::Install, install, None)],
                ),
                Ok(ProviderBatchOutcome::NotRunProviderUnavailable) => (
                    ItemStatus::Blocked,
                    vec![message_evidence(
                        EvidenceStage::Install,
                        "provider unavailable",
                    )],
                ),
                Err(error) => (
                    ItemStatus::Failed,
                    vec![error_evidence(
                        EvidenceStage::Install,
                        error.to_string(),
                        error.exit_result(),
                    )],
                ),
            };
            batch.packages().iter().map(move |package| ReportItem {
                id: package.clone(),
                status,
                subject: ReportSubject::Package(PackageItem {
                    source: PackageSource::Provider {
                        provider: batch.provider().to_owned(),
                        provider_args: batch.provider_args().to_owned(),
                    },
                }),
                evidence: evidence.clone(),
            })
        })
        .collect()
}

fn action_items(
    plan: &ExecutionPlan,
    manual_results: &[NamedActionResult],
    action_results: &[NamedActionResult],
) -> Vec<ReportItem> {
    let mut items = plan
        .manual_packages()
        .iter()
        .zip(manual_results)
        .map(|(package, result)| {
            debug_assert_eq!(package.id(), result.id);
            let (status, evidence) = action_result(&result.outcome, ItemStatus::Installed);
            ReportItem {
                id: package.id().to_owned(),
                status,
                subject: ReportSubject::Package(PackageItem {
                    source: PackageSource::Manual {
                        install: ActionInfo::from(package.install()),
                    },
                }),
                evidence,
            }
        })
        .collect::<Vec<_>>();
    items.extend(
        plan.actions()
            .iter()
            .zip(action_results)
            .map(|(action, result)| {
                debug_assert_eq!(action.id(), result.id);
                let (status, evidence) = action_result(&result.outcome, ItemStatus::Executed);
                ReportItem {
                    id: action.id().to_owned(),
                    status,
                    subject: ReportSubject::Action(ActionItem {
                        action: ActionInfo::from(action.action()),
                    }),
                    evidence,
                }
            }),
    );
    items
}

fn action_result(
    result: &Result<ActionOutcome, ActionRunError>,
    executed_status: ItemStatus,
) -> (ItemStatus, Vec<Evidence>) {
    match result {
        Ok(ActionOutcome::AlreadySatisfied { check }) => (
            ItemStatus::Satisfied,
            vec![execution_evidence(
                EvidenceStage::Check,
                check,
                Some("check passed; no action needed"),
            )],
        ),
        Ok(ActionOutcome::Executed {
            initial_check,
            exec,
            post_check,
        }) => {
            let mut evidence = Vec::new();
            if let Some(check) = initial_check {
                evidence.push(execution_evidence(EvidenceStage::Check, check, None));
            }
            evidence.push(execution_evidence(EvidenceStage::Execute, exec, None));
            if let Some(check) = post_check {
                evidence.push(execution_evidence(EvidenceStage::PostCheck, check, None));
            }
            (executed_status, evidence)
        }
        Err(error) => (
            ItemStatus::Failed,
            vec![error_evidence(
                action_stage(error.stage()),
                error.to_string(),
                error.exit_result(),
            )],
        ),
    }
}

fn link_items(
    plan: &ExecutionPlan,
    result: &Result<LinkReport, LinkPhaseError>,
) -> (Vec<ReportItem>, Vec<Diagnostic>) {
    match result {
        Ok(report) => (
            plan.links()
                .iter()
                .zip(report.results())
                .map(|(link, result)| {
                    debug_assert_eq!(link.id(), result.id());
                    let (status, evidence) = match result.outcome() {
                        Ok(LinkOutcome::Satisfied) => (ItemStatus::Satisfied, Vec::new()),
                        Ok(LinkOutcome::Created) => (ItemStatus::Created, Vec::new()),
                        Ok(LinkOutcome::Replaced) => (ItemStatus::Replaced, Vec::new()),
                        Ok(LinkOutcome::SkippedMissingParent) => (
                            ItemStatus::Skipped,
                            vec![message_evidence(
                                EvidenceStage::Link,
                                "target parent is missing",
                            )],
                        ),
                        Err(error) => (
                            ItemStatus::Failed,
                            vec![link_error_evidence(error, &plan.platform().os)],
                        ),
                    };
                    report_link_item(link, status, evidence)
                })
                .collect(),
            Vec::new(),
        ),
        Err(error) => {
            let message = error.to_string();
            (
                plan.links()
                    .iter()
                    .map(|link| {
                        report_link_item(
                            link,
                            ItemStatus::Blocked,
                            vec![message_evidence(EvidenceStage::Link, message.clone())],
                        )
                    })
                    .collect(),
                vec![Diagnostic {
                    level: DiagnosticLevel::Error,
                    message,
                }],
            )
        }
    }
}

fn report_link_item(
    link: &crate::plan::PlannedLink,
    status: ItemStatus,
    evidence: Vec<Evidence>,
) -> ReportItem {
    ReportItem {
        id: link.id().to_owned(),
        status,
        subject: ReportSubject::Link(LinkItem {
            source: link.source().to_owned(),
            target: link.target().to_owned(),
            on_conflict: link.on_conflict(),
            on_missing_parent: link.on_missing_parent(),
        }),
        evidence,
    }
}

fn provider_error_evidence(error: &ProviderError) -> Evidence {
    error_evidence(
        provider_stage(error.stage()),
        error.to_string(),
        error.exit_result(),
    )
}

const fn provider_stage(stage: ProviderStage) -> EvidenceStage {
    match stage {
        ProviderStage::Activate | ProviderStage::Reactivate => EvidenceStage::Activate,
        ProviderStage::InitialProbe | ProviderStage::FinalProbe => EvidenceStage::Probe,
        ProviderStage::Ensure(_) => EvidenceStage::Ensure,
    }
}

const fn action_stage(stage: ActionStage) -> EvidenceStage {
    match stage {
        ActionStage::InitialCheck => EvidenceStage::Check,
        ActionStage::Exec => EvidenceStage::Execute,
        ActionStage::PostCheck => EvidenceStage::PostCheck,
    }
}

fn execution_evidence(
    stage: EvidenceStage,
    result: &ExecutionResult,
    message: Option<&str>,
) -> Evidence {
    Evidence {
        stage,
        exit_code: result.code(),
        message: message.map(str::to_owned),
        stdout: captured_text(result.stdout()),
        stderr: captured_text(result.stderr()),
        hints: Vec::new(),
    }
}

fn error_evidence(
    stage: EvidenceStage,
    message: String,
    result: Option<&ExecutionResult>,
) -> Evidence {
    Evidence {
        stage,
        exit_code: result.and_then(ExecutionResult::code),
        message: Some(message),
        stdout: result.and_then(|result| captured_text(result.stdout())),
        stderr: result.and_then(|result| captured_text(result.stderr())),
        hints: Vec::new(),
    }
}

fn message_evidence(stage: EvidenceStage, message: impl Into<String>) -> Evidence {
    Evidence {
        stage,
        exit_code: None,
        message: Some(message.into()),
        stdout: None,
        stderr: None,
        hints: Vec::new(),
    }
}

fn link_error_evidence(error: &crate::link::LinkError, os: &str) -> Evidence {
    let hints = error
        .diagnostic_context()
        .and_then(|(operation, source)| lookup(os, operation, source))
        .into_iter()
        .collect();

    Evidence {
        stage: EvidenceStage::Link,
        exit_code: None,
        message: Some(error.to_string()),
        stdout: None,
        stderr: None,
        hints,
    }
}

fn captured_text(output: Option<&[u8]>) -> Option<String> {
    output
        .filter(|output| !output.is_empty())
        .map(|output| String::from_utf8_lossy(output).into_owned())
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

#[cfg(test)]
mod tests {
    use std::io;
    use std::path::Path;

    use super::*;
    use crate::diagnostic::Operation;
    use crate::link::LinkError;

    #[test]
    fn link_evidence_keeps_the_native_error_and_structured_hint() {
        let error = LinkError::io_with_diagnostic(
            "create symbolic link",
            Path::new("target"),
            Operation::CreateSymbolicLink,
            io::Error::from_raw_os_error(1314),
        );

        let evidence = link_error_evidence(&error, "windows");

        assert!(
            evidence
                .message
                .as_deref()
                .is_some_and(|message| message.contains("os error 1314"))
        );
        assert_eq!(evidence.hints.len(), 1);
        assert_eq!(evidence.hints[0].code, "windows.symlink.privilege-required");
    }
}

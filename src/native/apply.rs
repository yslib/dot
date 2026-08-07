use crate::ConfigFile;
use crate::interpolation::DotPaths;
use crate::manifest::{EffectiveManifest, ManifestError};
use crate::native::command_action::{
    ActionStage, CommandActionOutcome, CommandActionRunError, FetchContentOutcome,
};
use crate::native::diagnostic::lookup;
use crate::native::job_execution::{
    ActionOutcome, BlockReason, JobExecutionReport, JobOutcome, JobRunner, JobState,
};
use crate::native::link::LinkOutcome;
use crate::native::plan::{
    ExecutionPlan, ExecutionPlanner, PlannedActionKind, PlannedJob, PlannedPackage,
    PlannedProviderInstall,
};
use crate::native::provider::{
    ProviderError, ProviderInstallError, ProviderInstallOutcome, ProviderOutcome, ProviderStage,
};
use crate::report::{
    ActionInfo, ActionItem, CommandActionInfo, CommandInfo, CommandReport, Diagnostic,
    DiagnosticLevel, Evidence, EvidenceStage, ItemStatus, LinkItem, PackageItem, PackageSource,
    ProviderItem, ProviderPackageSource, ReportCommand, ReportContext, ReportItem, ReportStatus,
    ReportSubject,
};
use crate::selection::ExecutionSelection;

use super::process::ExecutionResult;
use super::runtime::NativeRuntime;

pub fn apply(
    config: &ConfigFile,
    runtime: &NativeRuntime,
    selection: &ExecutionSelection,
) -> Result<CommandReport, ApplyError> {
    let platform = runtime.platform();
    let manifest = EffectiveManifest::select_for_execution(
        config.config(),
        platform,
        selection.scope.target.as_ref(),
        selection.scope.profile.named(),
    )?;
    let planner = ExecutionPlanner::new(
        runtime.environment(),
        DotPaths::from(config),
        runtime.xdg_paths(),
        platform,
    );
    let plan = planner.plan(&manifest, &selection.jobs)?;
    let execution = JobRunner::new(runtime.environment()).run(&plan);

    Ok(build_report(&plan, &execution))
}

#[derive(Debug, thiserror::Error)]
pub enum ApplyError {
    #[error(transparent)]
    Manifest(#[from] ManifestError),

    #[error(transparent)]
    Plan(#[from] crate::native::plan::ExecutionPlanError),
}

fn build_report(plan: &ExecutionPlan, execution: &JobExecutionReport) -> CommandReport {
    let items = plan
        .jobs()
        .iter()
        .map(|job| {
            let id = job.id();
            let state = execution
                .get(&id)
                .unwrap_or_else(|| panic!("execution report is missing selected job `{id:?}`"));
            report_item(job, state, &plan.platform().os)
        })
        .collect();
    let diagnostics = execution
        .link_phase_error()
        .map(|error| Diagnostic {
            level: DiagnosticLevel::Error,
            message: error.to_string(),
        })
        .into_iter()
        .collect();

    CommandReport {
        command: ReportCommand::Apply,
        context: ReportContext {
            target: plan.target().to_owned(),
            profile: plan.profile().map(str::to_owned),
            platform: plan.platform().clone(),
        },
        status: if execution.all_succeeded() {
            ReportStatus::Succeeded
        } else {
            ReportStatus::Failed
        },
        items,
        diagnostics,
    }
}

fn report_item(job: &PlannedJob, state: &JobState, platform_os: &str) -> ReportItem {
    match (job, state) {
        (PlannedJob::Provider(provider), JobState::Completed(JobOutcome::Provider(result))) => {
            provider_item(provider, result)
        }
        (
            PlannedJob::Package(PlannedPackage::Provider(package)),
            JobState::Completed(JobOutcome::ProviderPackage(result)),
        ) => provider_package_item(package, result),
        (
            PlannedJob::Package(PlannedPackage::Provider(package)),
            JobState::Blocked(BlockReason::ProviderUnavailable { provider }),
        ) => {
            assert_eq!(
                package.provider(),
                provider.as_str(),
                "provider block reason must match the selected package"
            );
            provider_package_report_item(
                package,
                ItemStatus::Blocked,
                vec![message_evidence(
                    EvidenceStage::Install,
                    "provider unavailable",
                )],
            )
        }
        (
            PlannedJob::Package(PlannedPackage::Manual(package)),
            JobState::Completed(JobOutcome::ManualPackage(result)),
        ) => {
            let (status, evidence) = command_action_result(result, ItemStatus::Installed);
            ReportItem {
                id: package.id().to_owned(),
                status,
                subject: ReportSubject::Package(PackageItem {
                    source: PackageSource::Manual {
                        install: CommandActionInfo::from_resolved(package.install()),
                    },
                }),
                evidence,
            }
        }
        (PlannedJob::Action(action), JobState::Completed(JobOutcome::Action(result))) => {
            let (status, evidence) = selected_action_result(result);
            let action_info = match action.kind() {
                PlannedActionKind::Command(action) => ActionInfo::from_resolved_command(action),
                PlannedActionKind::FetchContent(action) => ActionInfo::fetch_content(
                    action.source().to_string(),
                    action.target().to_owned(),
                    action.on_conflict(),
                ),
            };
            ReportItem {
                id: action.id().to_owned(),
                status,
                subject: ReportSubject::Action(ActionItem {
                    action: action_info,
                }),
                evidence,
            }
        }
        (PlannedJob::Link(link), JobState::Completed(JobOutcome::Link(result))) => {
            link_item(link, result, platform_os)
        }
        (PlannedJob::Link(link), JobState::Blocked(BlockReason::LinkPhase { message })) => {
            report_link_item(
                link,
                ItemStatus::Blocked,
                vec![message_evidence(EvidenceStage::Link, message.clone())],
            )
        }
        _ => panic!(
            "execution state does not match selected job: planned {job:?}, execution {state:?}"
        ),
    }
}

fn provider_item(
    provider: &crate::native::plan::PlannedProvider,
    result: &Result<ProviderOutcome, ProviderError>,
) -> ReportItem {
    let (status, evidence) = match result {
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
            probe: CommandInfo::from_resolved(provider.probe()),
            ensure: provider
                .ensure()
                .iter()
                .map(CommandInfo::from_resolved)
                .collect(),
            has_activation: provider.activate().is_some(),
        }),
        evidence,
    }
}

fn provider_package_item(
    package: &PlannedProviderInstall,
    result: &Result<ProviderInstallOutcome, ProviderInstallError>,
) -> ReportItem {
    let (status, evidence) = match result {
        Ok(ProviderInstallOutcome::Executed { install }) => (
            ItemStatus::Installed,
            vec![execution_evidence(EvidenceStage::Install, install, None)],
        ),
        Ok(ProviderInstallOutcome::NotRunProviderUnavailable) => (
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
    provider_package_report_item(package, status, evidence)
}

fn provider_package_report_item(
    package: &PlannedProviderInstall,
    status: ItemStatus,
    evidence: Vec<Evidence>,
) -> ReportItem {
    let source = match package {
        PlannedProviderInstall::Single(_) => ProviderPackageSource::Single {
            provider: package.provider().to_owned(),
            provider_args: package.provider_args().to_owned(),
        },
        PlannedProviderInstall::Batch(_) => ProviderPackageSource::Batch {
            provider: package.provider().to_owned(),
            names: package.names().map(str::to_owned).collect(),
            provider_args: package.provider_args().to_owned(),
        },
    };
    ReportItem {
        id: package.id().to_owned(),
        status,
        subject: ReportSubject::Package(PackageItem {
            source: PackageSource::Provider(source),
        }),
        evidence,
    }
}

fn link_item(
    link: &crate::native::plan::PlannedLink,
    result: &Result<LinkOutcome, crate::native::link::LinkError>,
    platform_os: &str,
) -> ReportItem {
    let (status, evidence) = match result {
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
            vec![link_error_evidence(error, platform_os)],
        ),
    };
    report_link_item(link, status, evidence)
}

fn command_action_result(
    result: &Result<CommandActionOutcome, CommandActionRunError>,
    executed_status: ItemStatus,
) -> (ItemStatus, Vec<Evidence>) {
    match result {
        Ok(CommandActionOutcome::AlreadySatisfied { check }) => (
            ItemStatus::Satisfied,
            vec![execution_evidence(
                EvidenceStage::Check,
                check,
                Some("check passed; no action needed"),
            )],
        ),
        Ok(CommandActionOutcome::Executed {
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

fn selected_action_result(result: &ActionOutcome) -> (ItemStatus, Vec<Evidence>) {
    match result {
        ActionOutcome::Command(result) => command_action_result(result, ItemStatus::Executed),
        ActionOutcome::FetchContent(Ok(FetchContentOutcome::Created)) => {
            (ItemStatus::Created, Vec::new())
        }
        ActionOutcome::FetchContent(Ok(FetchContentOutcome::Replaced)) => {
            (ItemStatus::Replaced, Vec::new())
        }
        ActionOutcome::FetchContent(Err(error)) => (
            ItemStatus::Failed,
            vec![message_evidence(EvidenceStage::Fetch, error.to_string())],
        ),
    }
}

fn report_link_item(
    link: &crate::native::plan::PlannedLink,
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

fn link_error_evidence(error: &crate::native::link::LinkError, os: &str) -> Evidence {
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

#[cfg(test)]
mod tests {
    use std::io;
    use std::path::Path;

    use super::link_error_evidence;
    use crate::native::diagnostic::Operation;
    use crate::native::link::LinkError;

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

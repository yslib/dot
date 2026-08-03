use std::path::Path;

use super::{ExecutionCommandError, ExecutionRequest};
use crate::action::ExecutionResult;
use crate::action_runner::{ActionOutcome, ActionRunError, ActionStage};
use crate::config::LoadedConfig;
use crate::diagnostic::lookup;
use crate::interpolation::{DotPaths, XdgPaths};
use crate::job_runner::{BlockReason, JobExecutionReport, JobOutcome, JobRunner, JobState};
use crate::link::LinkOutcome;
use crate::manifest::EffectiveManifest;
use crate::plan::{
    ExecutionPlan, ExecutionPlanner, PlannedJob, PlannedPackage, PlannedProviderInstall,
};
use crate::platform::PlatformInfo;
use crate::provider::{
    ProviderError, ProviderInstallError, ProviderInstallOutcome, ProviderOutcome, ProviderStage,
};
use crate::report::{
    ActionInfo, ActionItem, CommandInfo, CommandReport, Diagnostic, DiagnosticLevel, Evidence,
    EvidenceStage, ItemStatus, LinkItem, PackageItem, PackageSource, ProviderItem,
    ProviderPackageSource, ReportCommand, ReportContext, ReportItem, ReportStatus, ReportSubject,
};

pub(super) fn run(
    config: &Path,
    request: &ExecutionRequest,
) -> Result<CommandReport, ExecutionCommandError> {
    let loaded = LoadedConfig::load(config)?;
    let platform = PlatformInfo::detect();
    let manifest = EffectiveManifest::select_for_execution(
        loaded.config(),
        &platform,
        request.scope.target.as_ref(),
        request.scope.profile.named(),
    )?;
    let xdg_paths = XdgPaths::detect();
    let dot_paths = DotPaths::from(&loaded);
    let planner = ExecutionPlanner::new(loaded.environment(), dot_paths, &xdg_paths, &platform);
    let plan = planner.plan(&manifest, &request.jobs)?;
    let execution = JobRunner::new(loaded.environment()).run(&plan);

    Ok(build_report(loaded.path(), &plan, &execution))
}

fn build_report(
    config: &Path,
    plan: &ExecutionPlan,
    execution: &JobExecutionReport,
) -> CommandReport {
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
            config: config.to_owned(),
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
            let (status, evidence) = action_result(result, ItemStatus::Installed);
            ReportItem {
                id: package.id().to_owned(),
                status,
                subject: ReportSubject::Package(PackageItem {
                    source: PackageSource::Manual {
                        install: ActionInfo::from_resolved_command(package.install()),
                    },
                }),
                evidence,
            }
        }
        (PlannedJob::Action(action), JobState::Completed(JobOutcome::Action(result))) => {
            let (status, evidence) = action_result(result, ItemStatus::Executed);
            ReportItem {
                id: action.id().to_owned(),
                status,
                subject: ReportSubject::Action(ActionItem {
                    action: ActionInfo::from_resolved_command(action.action()),
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
    provider: &crate::plan::PlannedProvider,
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
    link: &crate::plan::PlannedLink,
    result: &Result<LinkOutcome, crate::link::LinkError>,
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

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::process;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::app::{ProfileSelection, ScopeSelection};
    use crate::diagnostic::Operation;
    use crate::link::LinkError;

    static NEXT_WORKSPACE: AtomicU64 = AtomicU64::new(0);

    struct TempWorkspace {
        directory: PathBuf,
    }

    impl TempWorkspace {
        fn new() -> Self {
            let sequence = NEXT_WORKSPACE.fetch_add(1, Ordering::Relaxed);
            let directory =
                env::temp_dir().join(format!("dot-apply-report-{}-{sequence}", process::id()));
            fs::create_dir(&directory).expect("temporary workspace should be created");
            Self { directory }
        }

        fn write_manifest(&self, contents: &str) -> PathBuf {
            let path = self.directory.join("dot.toml");
            fs::write(&path, render_manifest(contents)).expect("test manifest should be written");
            path
        }

        fn write_source(&self, name: &str) {
            fs::write(self.directory.join(name), name).expect("link source should be written");
        }

        fn path(&self, name: &str) -> PathBuf {
            self.directory.join(name)
        }
    }

    impl Drop for TempWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum SubjectKind {
        Provider,
        ProviderPackageSingle,
        ProviderPackageBatch,
        ManualPackage,
        Action,
        Link,
    }

    #[test]
    fn apply_projects_the_complete_selected_plan_in_typed_job_order() {
        let workspace = TempWorkspace::new();
        workspace.write_source("source.txt");
        let contents = read_fixture("apply/valid-complete-plan-template.toml")
            .replace("__PROBE__", &helper_exec("probe-ready"))
            .replace("__INSTALL__", &helper_exec("install-ready"))
            .replace("__MANUAL__", &helper_exec("manual-ok"))
            .replace("__ACTION__", &helper_exec("action-ok"));
        let manifest = workspace.write_manifest(&contents);

        let report = run(&manifest, &request()).expect("complete apply should produce a report");

        assert_eq!(report.status, ReportStatus::Succeeded);
        assert!(report.diagnostics.is_empty());
        assert_eq!(
            item_sequence(&report),
            [
                ("ready", SubjectKind::Provider),
                ("cli-tools", SubjectKind::ProviderPackageBatch),
                ("tool", SubjectKind::ProviderPackageSingle),
                ("manual-tool", SubjectKind::ManualPackage),
                ("configure", SubjectKind::Action),
                ("config", SubjectKind::Link),
            ]
        );
    }

    #[test]
    fn apply_projects_a_duplicate_link_phase_to_typed_blocked_items_and_one_diagnostic() {
        let workspace = TempWorkspace::new();
        workspace.write_source("first.txt");
        workspace.write_source("second.txt");
        let contents = read_fixture("apply/invalid-duplicate-link-target-template.toml")
            .replace("__ACTION__", &helper_exec("action-ok"));
        let manifest = workspace.write_manifest(&contents);

        let report =
            run(&manifest, &request()).expect("link phase failure should produce a report");

        assert_eq!(report.status, ReportStatus::Failed);
        assert_eq!(
            item_sequence(&report),
            [
                ("configure", SubjectKind::Action),
                ("first", SubjectKind::Link),
                ("second", SubjectKind::Link),
            ]
        );
        assert_eq!(report.diagnostics.len(), 1);
        let diagnostic = &report.diagnostics[0];
        assert_eq!(diagnostic.level, DiagnosticLevel::Error);
        for item in &report.items[1..] {
            assert_eq!(item.status, ItemStatus::Blocked);
            assert_eq!(item.evidence.len(), 1);
            assert_eq!(item.evidence[0].stage, EvidenceStage::Link);
            assert_eq!(
                item.evidence[0].message.as_deref(),
                Some(diagnostic.message.as_str())
            );
        }
        assert!(
            !workspace.path("linked.txt").exists(),
            "duplicate-target preflight must not create the normalized target"
        );
    }

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

    #[test]
    fn helper_process() {
        let Ok(mode) = env::var("DOT_APPLY_REPORT_HELPER") else {
            return;
        };

        if matches!(mode.as_str(), "probe-ready" | "install-ready") {
            assert_eq!(
                env::var("DOT_APPLY_PROVIDER_ACTIVE").as_deref(),
                Ok("yes"),
                "provider child process should receive activate environment"
            );
        } else {
            assert!(
                env::var_os("DOT_APPLY_PROVIDER_ACTIVE").is_none(),
                "manual packages and actions must not receive provider environment"
            );
        }

        match mode.as_str() {
            "probe-ready" | "install-ready" | "manual-ok" => {}
            "action-ok" => {
                let link = PathBuf::from(
                    env::var_os("DOT_APPLY_REPORT_LINK")
                        .expect("apply report link path should be present"),
                );
                assert!(!link.exists(), "links must run after global actions");
            }
            unknown => panic!("unknown apply report helper mode: {unknown}"),
        }
    }

    fn request() -> ExecutionRequest {
        ExecutionRequest {
            scope: ScopeSelection {
                target: None,
                profile: ProfileSelection::Root,
            },
            jobs: crate::job::JobSelection::All,
        }
    }

    fn item_sequence(report: &CommandReport) -> Vec<(&str, SubjectKind)> {
        report
            .items
            .iter()
            .map(|item| {
                let kind = match &item.subject {
                    ReportSubject::Provider(_) => SubjectKind::Provider,
                    ReportSubject::Package(PackageItem {
                        source: PackageSource::Provider(ProviderPackageSource::Single { .. }),
                    }) => SubjectKind::ProviderPackageSingle,
                    ReportSubject::Package(PackageItem {
                        source: PackageSource::Provider(ProviderPackageSource::Batch { .. }),
                    }) => SubjectKind::ProviderPackageBatch,
                    ReportSubject::Package(PackageItem {
                        source: PackageSource::Manual { .. },
                    }) => SubjectKind::ManualPackage,
                    ReportSubject::Action(_) => SubjectKind::Action,
                    ReportSubject::Link(_) => SubjectKind::Link,
                };
                (item.id.as_str(), kind)
            })
            .collect()
    }

    fn read_fixture(name: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        fs::read_to_string(path).expect("apply fixture should be readable")
    }

    fn render_manifest(contents: &str) -> String {
        contents
            .replace("__OS__", env::consts::OS)
            .replace("__PROGRAM__", &helper_program_toml())
    }

    fn helper_program_toml() -> String {
        format!(
            "{:?}",
            env::current_exe()
                .expect("test executable should have a path")
                .to_string_lossy()
        )
    }

    fn helper_exec(mode: &str) -> String {
        format!(
            r#"{{ program = __PROGRAM__, args = ["--exact", "app::apply::tests::helper_process", "--nocapture"], env = {{ variables = {{ DOT_APPLY_REPORT_HELPER = "{mode}", DOT_APPLY_REPORT_LINK = "${{dot:config_dir}}/linked.txt" }} }} }}"#
        )
    }
}

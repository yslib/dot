use crate::ConfigFile;
use crate::interpolation::DotPaths;
use crate::manifest::{EffectiveManifest, ManifestError};
use crate::platform::PlatformInfo;
use crate::selection::ExecutionSelection;

use super::plan::{
    ExecutionPlan, ExecutionPlanError, ExecutionPlanner, PlannedAction, PlannedActionKind,
    PlannedJob, PlannedLink, PlannedManualPackage, PlannedPackage, PlannedProvider,
    PlannedProviderInstall,
};
use super::runtime::NativeRuntime;
use crate::report::{
    ActionInfo, ActionItem, CommandActionInfo, CommandInfo, CommandReport, ItemStatus, LinkItem,
    PackageItem, PackageSource, ProviderItem, ProviderPackageSource, ReportCommand, ReportContext,
    ReportItem, ReportStatus, ReportSubject,
};

pub fn dry_run(
    config: &ConfigFile,
    runtime: &NativeRuntime,
    compatibility_platform: &PlatformInfo,
    selection: &ExecutionSelection,
) -> Result<CommandReport, DryRunError> {
    let manifest = EffectiveManifest::select_for_execution(
        config.config(),
        compatibility_platform,
        selection.scope.target.as_ref(),
        selection.scope.profile.named(),
    )?;
    let planner = ExecutionPlanner::new(
        runtime.environment(),
        DotPaths::from(config),
        runtime.xdg_paths(),
        compatibility_platform,
    );
    let plan = planner.plan(&manifest, &selection.jobs)?;

    Ok(build_report(&plan))
}

#[derive(Debug, thiserror::Error)]
pub enum DryRunError {
    #[error(transparent)]
    Manifest(#[from] ManifestError),

    #[error(transparent)]
    Plan(#[from] ExecutionPlanError),
}

pub fn build_report(plan: &ExecutionPlan) -> CommandReport {
    let items = plan
        .jobs()
        .iter()
        .map(|job| match job {
            PlannedJob::Provider(provider) => provider_item(provider),
            PlannedJob::Package(PlannedPackage::Provider(package)) => {
                provider_package_item(package)
            }
            PlannedJob::Package(PlannedPackage::Manual(package)) => manual_package_item(package),
            PlannedJob::Action(action) => action_item(action),
            PlannedJob::Link(link) => link_item(link),
        })
        .collect();

    CommandReport {
        command: ReportCommand::DryRun,
        context: ReportContext {
            target: plan.target().to_owned(),
            profile: plan.profile().map(str::to_owned),
            platform: plan.platform().clone(),
        },
        status: ReportStatus::Planned,
        items,
        diagnostics: Vec::new(),
    }
}

fn provider_item(provider: &PlannedProvider) -> ReportItem {
    ReportItem {
        id: provider.id().to_owned(),
        status: ItemStatus::Planned,
        subject: ReportSubject::Provider(ProviderItem {
            probe: CommandInfo::from_resolved(provider.probe()),
            ensure: provider
                .ensure()
                .iter()
                .map(CommandInfo::from_resolved)
                .collect(),
            has_activation: provider.activate().is_some(),
        }),
        evidence: Vec::new(),
    }
}

fn provider_package_item(install: &PlannedProviderInstall) -> ReportItem {
    let source = match install {
        PlannedProviderInstall::Single(_) => ProviderPackageSource::Single {
            provider: install.provider().to_owned(),
            provider_args: install.provider_args().to_owned(),
        },
        PlannedProviderInstall::Batch(_) => ProviderPackageSource::Batch {
            provider: install.provider().to_owned(),
            names: install.names().map(str::to_owned).collect(),
            provider_args: install.provider_args().to_owned(),
        },
    };
    ReportItem {
        id: install.id().to_owned(),
        status: ItemStatus::Planned,
        subject: ReportSubject::Package(PackageItem {
            source: PackageSource::Provider(source),
        }),
        evidence: Vec::new(),
    }
}

fn manual_package_item(package: &PlannedManualPackage) -> ReportItem {
    ReportItem {
        id: package.id().to_owned(),
        status: ItemStatus::Planned,
        subject: ReportSubject::Package(PackageItem {
            source: PackageSource::Manual {
                install: CommandActionInfo::from_resolved(package.install()),
            },
        }),
        evidence: Vec::new(),
    }
}

fn action_item(action: &PlannedAction) -> ReportItem {
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
        status: ItemStatus::Planned,
        subject: ReportSubject::Action(ActionItem {
            action: action_info,
        }),
        evidence: Vec::new(),
    }
}

fn link_item(link: &PlannedLink) -> ReportItem {
    ReportItem {
        id: link.id().to_owned(),
        status: ItemStatus::Planned,
        subject: ReportSubject::Link(LinkItem {
            source: link.source().to_owned(),
            target: link.target().to_owned(),
            on_conflict: link.on_conflict(),
            on_missing_parent: link.on_missing_parent(),
        }),
        evidence: Vec::new(),
    }
}

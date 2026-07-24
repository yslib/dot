use std::path::Path;

use crate::plan::{ExecutionPlan, PlannedProviderInstall};
use crate::report::{
    ActionInfo, ActionItem, CommandInfo, CommandReport, ItemStatus, LinkItem, PackageItem,
    PackageSource, ProviderItem, ProviderPackageSource, ReportCommand, ReportContext, ReportItem,
    ReportStatus, ReportSubject,
};

pub fn build_report(config: &Path, plan: &ExecutionPlan) -> CommandReport {
    let mut items = Vec::new();

    items.extend(plan.providers().iter().map(|provider| {
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
    }));

    items.extend(plan.provider_installs().iter().map(|install| {
        let source = match install {
            PlannedProviderInstall::Single(_) => ProviderPackageSource::Single {
                provider: install.provider().to_owned(),
                provider_args: install.provider_args().to_owned(),
            },
            PlannedProviderInstall::Batch(_) => ProviderPackageSource::Batch {
                provider: install.provider().to_owned(),
                names: install.names().to_owned(),
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
    }));

    items.extend(plan.manual_packages().iter().map(|package| ReportItem {
        id: package.id().to_owned(),
        status: ItemStatus::Planned,
        subject: ReportSubject::Package(PackageItem {
            source: PackageSource::Manual {
                install: ActionInfo::from_resolved(package.install()),
            },
        }),
        evidence: Vec::new(),
    }));

    items.extend(plan.actions().iter().map(|action| ReportItem {
        id: action.id().to_owned(),
        status: ItemStatus::Planned,
        subject: ReportSubject::Action(ActionItem {
            action: ActionInfo::from_resolved(action.action()),
        }),
        evidence: Vec::new(),
    }));

    items.extend(plan.links().iter().map(|link| ReportItem {
        id: link.id().to_owned(),
        status: ItemStatus::Planned,
        subject: ReportSubject::Link(LinkItem {
            source: link.source().to_owned(),
            target: link.target().to_owned(),
            on_conflict: link.on_conflict(),
            on_missing_parent: link.on_missing_parent(),
        }),
        evidence: Vec::new(),
    }));

    CommandReport {
        command: ReportCommand::DryRun,
        context: ReportContext {
            config: config.to_owned(),
            target: plan.target().to_owned(),
            profile: plan.profile().map(str::to_owned),
            platform: plan.platform().clone(),
        },
        status: ReportStatus::Planned,
        items,
        diagnostics: Vec::new(),
    }
}

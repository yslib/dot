use std::path::Path;

use crate::plan::ExecutionPlan;
use crate::report::{
    ActionInfo, ActionItem, CommandInfo, CommandReport, ItemStatus, LinkItem, PackageItem,
    PackageSource, ProviderItem, ReportCommand, ReportContext, ReportItem, ReportStatus,
    ReportSubject,
};

pub fn build_report(config: &Path, plan: &ExecutionPlan) -> CommandReport {
    let mut items = Vec::new();

    items.extend(plan.providers().iter().map(|provider| ReportItem {
        id: provider.id().to_owned(),
        status: ItemStatus::Planned,
        subject: ReportSubject::Provider(ProviderItem {
            probe: CommandInfo::from(provider.probe()),
            ensure: provider.ensure().iter().map(CommandInfo::from).collect(),
            has_activation: provider.activate().is_some(),
        }),
        evidence: Vec::new(),
    }));

    for batch in plan.provider_installs() {
        items.extend(batch.names().iter().map(|package| ReportItem {
            id: package.clone(),
            status: ItemStatus::Planned,
            subject: ReportSubject::Package(PackageItem {
                source: PackageSource::Provider {
                    provider: batch.provider().to_owned(),
                    provider_args: batch.provider_args().to_owned(),
                },
            }),
            evidence: Vec::new(),
        }));
    }

    items.extend(plan.manual_packages().iter().map(|package| ReportItem {
        id: package.id().to_owned(),
        status: ItemStatus::Planned,
        subject: ReportSubject::Package(PackageItem {
            source: PackageSource::Manual {
                install: ActionInfo::from(package.install()),
            },
        }),
        evidence: Vec::new(),
    }));

    items.extend(plan.actions().iter().map(|action| ReportItem {
        id: action.id().to_owned(),
        status: ItemStatus::Planned,
        subject: ReportSubject::Action(ActionItem {
            action: ActionInfo::from(action.action()),
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

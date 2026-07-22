use std::collections::BTreeSet;
use std::path::PathBuf;

use dot::diagnostic::ErrorHint;
use dot::platform::PlatformInfo;
use dot::report::{
    ActionInfo, ActionItem, CommandInfo, CommandReport, Evidence, EvidenceStage, ItemStatus,
    LinkItem, PackageItem, PackageSource, ProviderItem, ReportCommand, ReportContext, ReportItem,
    ReportStatus, ReportSubject,
};
use dot::schema::{LinkConflict, LinkMissingParent};

fn command(program: &str, args: &[&str]) -> CommandInfo {
    CommandInfo {
        program: program.to_owned(),
        args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        cwd: None,
    }
}

fn context() -> ReportContext {
    ReportContext {
        config: PathBuf::from("/repo/dot.toml"),
        target: "arch".to_owned(),
        profile: Some("laptop".to_owned()),
        platform: PlatformInfo {
            os: "linux".to_owned(),
            arch: "x86_64".to_owned(),
            distro: Some("arch".to_owned()),
            distro_families: BTreeSet::new(),
            environments: BTreeSet::from(["native".to_owned()]),
        },
    }
}

#[test]
fn report_represents_each_logical_item_as_one_entry() {
    let items = vec![
        ReportItem {
            id: "pacman".to_owned(),
            status: ItemStatus::Ready,
            subject: ReportSubject::Provider(ProviderItem {
                probe: command("pacman", &["--version"]),
                ensure: Vec::new(),
                has_activation: false,
            }),
            evidence: Vec::new(),
        },
        ReportItem {
            id: "ripgrep".to_owned(),
            status: ItemStatus::Installed,
            subject: ReportSubject::Package(PackageItem {
                source: PackageSource::Provider {
                    provider: "pacman".to_owned(),
                    provider_args: vec!["--needed".to_owned()],
                },
            }),
            evidence: Vec::new(),
        },
        ReportItem {
            id: "setup-shell".to_owned(),
            status: ItemStatus::Executed,
            subject: ReportSubject::Action(ActionItem {
                action: ActionInfo {
                    check: None,
                    exec: command("sh", &["setup.sh"]),
                },
            }),
            evidence: Vec::new(),
        },
        ReportItem {
            id: "nvim".to_owned(),
            status: ItemStatus::Created,
            subject: ReportSubject::Link(LinkItem {
                source: PathBuf::from("/repo/nvim"),
                target: PathBuf::from("/home/user/.config/nvim"),
                on_conflict: LinkConflict::ReplaceLink,
                on_missing_parent: LinkMissingParent::Create,
            }),
            evidence: Vec::new(),
        },
    ];
    let report = CommandReport {
        command: ReportCommand::Apply,
        context: context(),
        status: ReportStatus::Succeeded,
        items,
        diagnostics: Vec::new(),
    };

    assert_eq!(report.items.len(), 4);
    assert!(matches!(
        &report.items[1].subject,
        ReportSubject::Package(PackageItem {
            source: PackageSource::Provider { provider, .. },
        }) if provider == "pacman"
    ));
}

#[test]
fn evidence_keeps_process_results_structured() {
    let evidence = Evidence {
        stage: EvidenceStage::Probe,
        exit_code: Some(1),
        message: Some("provider probe returned a non-zero status".to_owned()),
        stdout: Some(String::new()),
        stderr: Some("not found".to_owned()),
        hints: vec![ErrorHint {
            code: "test.provider.not-found".to_owned(),
            summary: "provider program is unavailable".to_owned(),
            suggestion: "install the provider before retrying".to_owned(),
        }],
    };

    assert_eq!(evidence.stage, EvidenceStage::Probe);
    assert_eq!(evidence.exit_code, Some(1));
    assert_eq!(evidence.stderr.as_deref(), Some("not found"));
    assert_eq!(evidence.hints[0].code, "test.provider.not-found");
}

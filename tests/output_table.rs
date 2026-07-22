use std::collections::BTreeSet;
use std::path::PathBuf;

use dot::output::TableRenderer;
use dot::platform::PlatformInfo;
use dot::report::{
    ActionInfo, ActionItem, CommandInfo, CommandReport, ItemStatus, LinkItem, PackageItem,
    PackageSource, ProviderItem, ReportCommand, ReportContext, ReportItem, ReportStatus,
    ReportSubject,
};
use dot::schema::{LinkConflict, LinkMissingParent};

fn command(program: &str, args: &[&str]) -> CommandInfo {
    CommandInfo {
        program: program.to_owned(),
        args: args.iter().map(|argument| (*argument).to_owned()).collect(),
        cwd: None,
    }
}

fn report() -> CommandReport {
    CommandReport {
        command: ReportCommand::Apply,
        context: ReportContext {
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
        },
        status: ReportStatus::Succeeded,
        items: vec![
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
                    target: PathBuf::from("/home/me/.config/nvim"),
                    on_conflict: LinkConflict::ReplaceLink,
                    on_missing_parent: LinkMissingParent::Create,
                }),
                evidence: Vec::new(),
            },
        ],
        diagnostics: Vec::new(),
    }
}

#[test]
fn renders_every_subject_through_one_dense_table() {
    let mut output = Vec::new();
    TableRenderer::new(false)
        .render(&report(), &mut output)
        .expect("table should render");
    let output = String::from_utf8(output).expect("table should be UTF-8");

    assert!(
        output.contains(
            "dot apply · target=arch · profile=laptop · platform=linux/x86_64 · distro=arch"
        ),
        "{output}"
    );
    assert!(output.contains("│ TYPE"), "{output}");
    assert!(output.contains("┆ ITEM"), "{output}");
    assert!(output.contains("┆ VIA"), "{output}");
    assert!(output.contains("┆ STATUS"), "{output}");
    assert!(output.contains("┆ DETAIL"), "{output}");
    assert!(
        output.contains("│ provider ┆ pacman"),
        "provider row missing:\n{output}"
    );
    assert!(
        output.contains("│ package  ┆ ripgrep"),
        "package row missing:\n{output}"
    );
    assert!(
        output.contains("│ action   ┆ setup-shell"),
        "action row missing:\n{output}"
    );
    assert!(
        output.contains("│ link     ┆ nvim"),
        "link row missing:\n{output}"
    );
    assert!(
        output.contains("SUCCESS · 4 items · 1 provider · 1 package · 1 action · 1 link"),
        "{output}"
    );
    assert!(
        !output.contains("├╌"),
        "logical items should not be separated by extra border rows:\n{output}"
    );
    assert!(
        !output.contains("\u{1b}["),
        "color must be optional: {output}"
    );
}

#[test]
fn wraps_long_details_instead_of_expanding_the_table_without_bound() {
    let mut report = report();
    let ReportSubject::Action(action) = &mut report.items[2].subject else {
        panic!("third fixture item should be an action");
    };
    action.action.exec.args = vec!["x".repeat(240)];

    let mut output = Vec::new();
    TableRenderer::new(false)
        .render(&report, &mut output)
        .expect("table should render");
    let output = String::from_utf8(output).expect("table should be UTF-8");
    let longest_line = output
        .lines()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or_default();

    assert!(
        longest_line <= 120,
        "table line is {longest_line} columns wide:\n{output}"
    );
}

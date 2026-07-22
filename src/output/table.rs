use std::io::{self, Write};

use comfy_table::{
    Attribute, Cell, Color, ContentArrangement, Table, modifiers::UTF8_ROUND_CORNERS,
    presets::UTF8_FULL_CONDENSED,
};

use crate::report::{
    CommandInfo, CommandReport, DiagnosticLevel, EvidenceStage, ItemStatus, PackageSource,
    ReportCommand, ReportItem, ReportStatus, ReportSubject,
};

#[derive(Clone, Copy, Debug)]
pub struct TableRenderer {
    terminal: bool,
}

impl TableRenderer {
    pub const fn new(terminal: bool) -> Self {
        Self { terminal }
    }

    pub fn render(&self, report: &CommandReport, output: &mut dyn Write) -> io::Result<()> {
        writeln!(output, "{}", report_header(report))?;
        writeln!(output)?;

        let mut table = Table::new();
        table.load_preset(UTF8_FULL_CONDENSED);
        table.apply_modifier(UTF8_ROUND_CORNERS);
        table.set_content_arrangement(ContentArrangement::Dynamic);
        if !self.terminal {
            table.set_width(120);
        }
        table.set_header(
            ["TYPE", "ITEM", "VIA", "STATUS", "DETAIL"]
                .into_iter()
                .map(|value| Cell::new(value).add_attribute(Attribute::Bold)),
        );
        for item in &report.items {
            table.add_row(self.row(item));
        }
        writeln!(output, "{table}")?;

        for diagnostic in &report.diagnostics {
            writeln!(
                output,
                "{}: {}",
                diagnostic_level(diagnostic.level),
                diagnostic.message
            )?;
        }
        if !report.diagnostics.is_empty() {
            writeln!(output)?;
        }

        writeln!(output, "{}", report_summary(report))
    }

    fn row(&self, item: &ReportItem) -> Vec<Cell> {
        let (kind, via, subject_detail) = subject_columns(item);
        let detail = evidence_detail(item).unwrap_or(subject_detail);
        vec![
            Cell::new(kind),
            Cell::new(&item.id),
            Cell::new(via),
            self.status_cell(item.status),
            Cell::new(detail),
        ]
    }

    fn status_cell(&self, status: ItemStatus) -> Cell {
        let cell = Cell::new(item_status(status)).add_attribute(Attribute::Bold);
        if !self.terminal {
            return cell;
        }
        cell.fg(match status {
            ItemStatus::Planned => Color::Cyan,
            ItemStatus::Ready
            | ItemStatus::Installed
            | ItemStatus::Satisfied
            | ItemStatus::Executed
            | ItemStatus::Created
            | ItemStatus::Replaced => Color::Green,
            ItemStatus::Skipped => Color::Yellow,
            ItemStatus::NotReady | ItemStatus::Blocked | ItemStatus::Failed => Color::Red,
        })
    }
}

fn report_header(report: &CommandReport) -> String {
    let context = &report.context;
    let mut fields = vec![
        format!("dot {}", report_command(report.command)),
        format!("target={}", context.target),
        format!("profile={}", context.profile.as_deref().unwrap_or("<root>")),
        format!("platform={}/{}", context.platform.os, context.platform.arch),
    ];
    if let Some(distro) = &context.platform.distro {
        fields.push(format!("distro={distro}"));
    }
    fields.join(" · ")
}

fn report_summary(report: &CommandReport) -> String {
    let providers = count_subjects(report, |subject| {
        matches!(subject, ReportSubject::Provider(_))
    });
    let packages = count_subjects(report, |subject| {
        matches!(subject, ReportSubject::Package(_))
    });
    let actions = count_subjects(report, |subject| {
        matches!(subject, ReportSubject::Action(_))
    });
    let links = count_subjects(report, |subject| matches!(subject, ReportSubject::Link(_)));

    [
        report_status(report.status).to_owned(),
        format_count(report.items.len(), "item"),
        format_count(providers, "provider"),
        format_count(packages, "package"),
        format_count(actions, "action"),
        format_count(links, "link"),
    ]
    .join(" · ")
}

fn count_subjects(report: &CommandReport, predicate: impl Fn(&ReportSubject) -> bool) -> usize {
    report
        .items
        .iter()
        .filter(|item| predicate(&item.subject))
        .count()
}

fn format_count(count: usize, singular: &str) -> String {
    let suffix = if count == 1 { "" } else { "s" };
    format!("{count} {singular}{suffix}")
}

fn subject_columns(item: &ReportItem) -> (&'static str, String, String) {
    match &item.subject {
        ReportSubject::Provider(provider) => {
            let via = if item
                .evidence
                .iter()
                .any(|evidence| evidence.stage == EvidenceStage::Ensure)
            {
                "ensure"
            } else {
                "probe"
            };
            let mut details = Vec::new();
            if provider.has_activation {
                details.push("activate".to_owned());
            }
            details.push(format!("probe: {}", command_line(&provider.probe)));
            if !provider.ensure.is_empty() {
                details.push(format!("{} ensure step(s)", provider.ensure.len()));
            }
            ("provider", via.to_owned(), details.join("; "))
        }
        ReportSubject::Package(package) => match &package.source {
            PackageSource::Provider {
                provider,
                provider_args,
            } => {
                let detail = if provider_args.is_empty() {
                    String::new()
                } else {
                    format!("args: {}", provider_args.join(" "))
                };
                ("package", provider.clone(), detail)
            }
            PackageSource::Manual { install } => (
                "package",
                "manual".to_owned(),
                action_detail(install.check.as_ref(), &install.exec),
            ),
        },
        ReportSubject::Action(action) => (
            "action",
            "exec".to_owned(),
            action_detail(action.action.check.as_ref(), &action.action.exec),
        ),
        ReportSubject::Link(link) => (
            "link",
            "builtin".to_owned(),
            format!("{} → {}", link.source.display(), link.target.display()),
        ),
    }
}

fn action_detail(check: Option<&CommandInfo>, exec: &CommandInfo) -> String {
    match check {
        Some(check) => format!(
            "check: {}; exec: {}",
            command_line(check),
            command_line(exec)
        ),
        None => command_line(exec),
    }
}

fn command_line(command: &CommandInfo) -> String {
    std::iter::once(command.program.as_str())
        .chain(command.args.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ")
}

fn evidence_detail(item: &ReportItem) -> Option<String> {
    item.evidence
        .iter()
        .rev()
        .find_map(|evidence| evidence.message.clone())
        .or_else(|| {
            matches!(
                item.status,
                ItemStatus::NotReady | ItemStatus::Blocked | ItemStatus::Failed
            )
            .then_some(())?;
            item.evidence
                .iter()
                .rev()
                .find_map(|evidence| evidence.exit_code.map(|code| format!("exit {code}")))
        })
}

const fn report_command(command: ReportCommand) -> &'static str {
    match command {
        ReportCommand::DryRun => "dry-run",
        ReportCommand::Apply => "apply",
        ReportCommand::CheckProviders => "check providers",
    }
}

const fn report_status(status: ReportStatus) -> &'static str {
    match status {
        ReportStatus::Planned => "PLANNED",
        ReportStatus::Succeeded => "SUCCESS",
        ReportStatus::Failed => "FAILED",
    }
}

const fn item_status(status: ItemStatus) -> &'static str {
    match status {
        ItemStatus::Planned => "PLANNED",
        ItemStatus::Ready => "READY",
        ItemStatus::Installed => "INSTALLED",
        ItemStatus::Satisfied => "SATISFIED",
        ItemStatus::Executed => "EXECUTED",
        ItemStatus::Created => "CREATED",
        ItemStatus::Replaced => "REPLACED",
        ItemStatus::Skipped => "SKIPPED",
        ItemStatus::NotReady => "NOT_READY",
        ItemStatus::Blocked => "BLOCKED",
        ItemStatus::Failed => "FAILED",
    }
}

const fn diagnostic_level(level: DiagnosticLevel) -> &'static str {
    match level {
        DiagnosticLevel::Info => "INFO",
        DiagnosticLevel::Warning => "WARNING",
        DiagnosticLevel::Error => "ERROR",
    }
}

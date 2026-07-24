use std::path::PathBuf;

use crate::diagnostic::ErrorHint;
use crate::platform::PlatformInfo;
use crate::schema::{
    LinkConflict, LinkMissingParent, ResolvedAction, ResolvedExecAction, SourceAction,
    SourceExecAction,
};

/// Presentation-independent output produced by one dot command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandReport {
    pub command: ReportCommand,
    pub context: ReportContext,
    pub status: ReportStatus,
    pub items: Vec<ReportItem>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportCommand {
    DryRun,
    Apply,
    CheckProviders,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportContext {
    pub config: PathBuf,
    pub target: String,
    pub profile: Option<String>,
    pub platform: PlatformInfo,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportStatus {
    Planned,
    Succeeded,
    Failed,
}

/// One logical object from the effective manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportItem {
    pub id: String,
    pub status: ItemStatus,
    pub subject: ReportSubject,
    pub evidence: Vec<Evidence>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemStatus {
    Planned,
    Ready,
    Installed,
    Satisfied,
    Executed,
    Created,
    Replaced,
    Skipped,
    NotReady,
    Blocked,
    Failed,
}

/// Strongly typed content for a report item. Renderers derive columns from this
/// value instead of consuming preformatted strings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReportSubject {
    Provider(ProviderItem),
    Package(PackageItem),
    Action(ActionItem),
    Link(LinkItem),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderItem {
    pub probe: CommandInfo,
    pub ensure: Vec<CommandInfo>,
    pub has_activation: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageItem {
    pub source: PackageSource,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PackageSource {
    Provider(ProviderPackageSource),
    Manual { install: ActionInfo },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderPackageSource {
    Single {
        provider: String,
        provider_args: Vec<String>,
    },
    Batch {
        provider: String,
        names: Vec<String>,
        provider_args: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionItem {
    pub action: ActionInfo,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkItem {
    pub source: PathBuf,
    pub target: PathBuf,
    pub on_conflict: LinkConflict,
    pub on_missing_parent: LinkMissingParent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionInfo {
    pub check: Option<CommandInfo>,
    pub exec: CommandInfo,
}

/// Resolved command data that is safe for reporting. It deliberately excludes
/// the inherited process environment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandInfo {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
}

impl CommandInfo {
    pub fn from_source(command: &SourceExecAction) -> Self {
        Self {
            program: command.program.source_spelling().to_owned(),
            args: command
                .args
                .iter()
                .map(|argument| argument.source_spelling().to_owned())
                .collect(),
            cwd: command
                .cwd
                .as_ref()
                .map(|cwd| PathBuf::from(cwd.source_spelling())),
        }
    }

    pub fn from_resolved(command: &ResolvedExecAction) -> Self {
        Self {
            program: command.program.value().to_owned(),
            args: command
                .args
                .iter()
                .map(|argument| argument.value().to_owned())
                .collect(),
            cwd: command.cwd.as_ref().map(|cwd| PathBuf::from(cwd.value())),
        }
    }
}

impl ActionInfo {
    pub fn from_source(action: &SourceAction) -> Self {
        Self {
            check: action.check.as_ref().map(CommandInfo::from_source),
            exec: CommandInfo::from_source(&action.exec),
        }
    }

    pub fn from_resolved(action: &ResolvedAction) -> Self {
        Self {
            check: action.check.as_ref().map(CommandInfo::from_resolved),
            exec: CommandInfo::from_resolved(&action.exec),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Evidence {
    pub stage: EvidenceStage,
    pub exit_code: Option<i32>,
    pub message: Option<String>,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
    pub hints: Vec<ErrorHint>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvidenceStage {
    Activate,
    Probe,
    Ensure,
    Install,
    Check,
    Execute,
    PostCheck,
    Link,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub level: DiagnosticLevel,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagnosticLevel {
    Info,
    Warning,
    Error,
}

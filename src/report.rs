use std::path::PathBuf;

use crate::diagnostic::ErrorHint;
use crate::plan::PlannedFetchContentAction;
use crate::platform::PlatformInfo;
use crate::schema::{
    FetchContentConflict, LinkConflict, LinkMissingParent, ResolvedCommandAction,
    ResolvedExecAction, SourceCommandAction, SourceExecAction,
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
    Manual { install: CommandActionInfo },
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
pub enum ActionInfo {
    Command(CommandActionInfo),
    FetchContent {
        source: String,
        target: PathBuf,
        on_conflict: FetchContentConflict,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandActionInfo {
    pub check: Option<CommandInfo>,
    pub exec: CommandInfo,
}

/// Presentation-safe command data that deliberately excludes the inherited
/// process environment.
///
/// [`Self::from_source`] preserves configured source spelling, including
/// interpolation syntax, while [`Self::from_resolved`] records evaluated values.
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

impl CommandActionInfo {
    pub fn from_source(action: &SourceCommandAction) -> Self {
        Self {
            check: action.check.as_ref().map(CommandInfo::from_source),
            exec: CommandInfo::from_source(&action.exec),
        }
    }

    pub fn from_resolved(action: &ResolvedCommandAction) -> Self {
        Self {
            check: action.check.as_ref().map(CommandInfo::from_resolved),
            exec: CommandInfo::from_resolved(&action.exec),
        }
    }
}

impl ActionInfo {
    pub fn from_source_command(action: &SourceCommandAction) -> Self {
        Self::Command(CommandActionInfo::from_source(action))
    }

    pub fn from_resolved_command(action: &ResolvedCommandAction) -> Self {
        Self::Command(CommandActionInfo::from_resolved(action))
    }

    pub fn from_fetch_content(action: &PlannedFetchContentAction) -> Self {
        Self::FetchContent {
            source: action.source().to_string(),
            target: action.target().to_owned(),
            on_conflict: action.on_conflict(),
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
    Fetch,
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

#[cfg(test)]
mod tests {
    use super::*;
    use url::Url;

    #[test]
    fn maps_resolved_command_to_command_facts() {
        let action = ResolvedCommandAction {
            check: Some(ResolvedExecAction {
                program: "tool".into(),
                args: vec!["--version".into()],
                cwd: None,
                env: None,
            }),
            exec: ResolvedExecAction {
                program: "sh".into(),
                args: vec!["install.sh".into()],
                cwd: Some("/resolved".into()),
                env: None,
            },
        };

        assert_eq!(
            ActionInfo::from_resolved_command(&action),
            ActionInfo::Command(CommandActionInfo {
                check: Some(CommandInfo {
                    program: "tool".to_owned(),
                    args: vec!["--version".to_owned()],
                    cwd: None,
                }),
                exec: CommandInfo {
                    program: "sh".to_owned(),
                    args: vec!["install.sh".to_owned()],
                    cwd: Some(PathBuf::from("/resolved")),
                },
            })
        );
    }

    #[test]
    fn maps_planned_fetch_content_to_fetch_facts() {
        let action = PlannedFetchContentAction::new(
            Url::parse("https://example.com/config.toml").expect("URL should parse"),
            PathBuf::from("/resolved/configs/app.toml"),
            FetchContentConflict::Replace,
        );

        assert_eq!(
            ActionInfo::from_fetch_content(&action),
            ActionInfo::FetchContent {
                source: "https://example.com/config.toml".to_owned(),
                target: PathBuf::from("/resolved/configs/app.toml"),
                on_conflict: FetchContentConflict::Replace,
            }
        );
    }
}

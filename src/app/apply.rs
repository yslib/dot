use std::error::Error;
use std::fmt;
use std::io::{self, Write};

use super::Selection;
use crate::action::ExecutionEnvironment;
use crate::action_runner::{ActionOutcome, ActionRunError, ActionRunner};
use crate::config::{ConfigLoadError, LoadedConfig};
use crate::interpolation::{DotPaths, XdgPaths};
use crate::link::{self, LinkOutcome, LinkPhaseError, LinkReport};
use crate::manifest::{EffectiveManifest, ManifestError};
use crate::plan::{ExecutionPlan, ExecutionPlanner, PlanningError};
use crate::platform::PlatformInfo;
use crate::provider::{
    ProviderBatchExecution, ProviderBatchOutcome, ProviderOutcome, ProviderReadiness,
    ProviderRunner,
};

pub(super) fn run(selection: &Selection, output: &mut impl Write) -> Result<bool, CommandError> {
    let loaded = LoadedConfig::load(&selection.config)?;
    let platform = PlatformInfo::detect();
    let manifest = EffectiveManifest::select(
        loaded.config(),
        &platform,
        selection.target.as_deref(),
        selection.profile.as_deref(),
    )?;
    let xdg_paths = XdgPaths::detect();
    let dot_paths = DotPaths::new(loaded.path(), loaded.directory(), loaded.invocation_cwd());
    let planner = ExecutionPlanner::new(loaded.environment(), dot_paths, &xdg_paths, &platform);
    let plan = planner.plan(&manifest)?;
    let result = execute(&plan, loaded.environment());

    writeln!(output, "{result}")?;
    Ok(result.succeeded())
}

fn execute(plan: &ExecutionPlan, environment: &ExecutionEnvironment) -> ApplyResult {
    let provider_runner = ProviderRunner::new(environment);
    let providers = provider_runner.ensure_all(plan.providers());
    let provider_batches = provider_runner.install_batches(plan.provider_batches(), &providers);
    let action_runner = ActionRunner::new(environment);
    let manual_packages = plan
        .manual_packages()
        .iter()
        .map(|package| NamedActionResult {
            id: package.id().to_owned(),
            outcome: action_runner.run(package.install()),
        })
        .collect();
    let actions = plan
        .actions()
        .iter()
        .map(|action| NamedActionResult {
            id: action.id().to_owned(),
            outcome: action_runner.run(action.action()),
        })
        .collect();
    let links = link::reconcile(plan.links());

    ApplyResult {
        target: plan.target().to_owned(),
        profile: plan.profile().map(str::to_owned),
        providers,
        provider_batches,
        manual_packages,
        actions,
        links,
    }
}

#[derive(Debug)]
struct NamedActionResult {
    id: String,
    outcome: Result<ActionOutcome, ActionRunError>,
}

#[derive(Debug)]
struct ApplyResult {
    target: String,
    profile: Option<String>,
    providers: ProviderReadiness,
    provider_batches: ProviderBatchExecution,
    manual_packages: Vec<NamedActionResult>,
    actions: Vec<NamedActionResult>,
    links: Result<LinkReport, LinkPhaseError>,
}

impl ApplyResult {
    fn succeeded(&self) -> bool {
        self.providers.all_ready()
            && self.provider_batches.all_succeeded()
            && self
                .manual_packages
                .iter()
                .all(|result| result.outcome.is_ok())
            && self.actions.iter().all(|result| result.outcome.is_ok())
            && self
                .links
                .as_ref()
                .is_ok_and(|report| report.all_succeeded())
    }
}

impl fmt::Display for ApplyResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(formatter, "target: {}", self.target)?;
        writeln!(
            formatter,
            "profile: {}",
            self.profile.as_deref().unwrap_or("<root>")
        )?;

        writeln!(formatter, "\nproviders:")?;
        if self.providers.statuses().is_empty() {
            writeln!(formatter, "  <none>")?;
        }
        for provider in self.providers.statuses() {
            match provider.outcome() {
                Ok(ProviderOutcome::AlreadyReady { .. }) => {
                    writeln!(formatter, "  READY {} (already ready)", provider.id())?;
                }
                Ok(ProviderOutcome::Ensured { .. }) => {
                    writeln!(formatter, "  READY {} (ensured)", provider.id())?;
                }
                Err(error) => {
                    writeln!(formatter, "  NOT_READY {}: {error}", provider.id())?;
                }
            }
        }

        writeln!(formatter, "\nprovider packages:")?;
        if self.provider_batches.statuses().is_empty() {
            writeln!(formatter, "  <none>")?;
        }
        for batch in self.provider_batches.statuses() {
            match batch.outcome() {
                Ok(ProviderBatchOutcome::Executed { .. }) => writeln!(
                    formatter,
                    "  OK {} {:?} (provider_args: {:?})",
                    batch.provider(),
                    batch.packages(),
                    batch.provider_args()
                )?,
                Ok(ProviderBatchOutcome::NotRunProviderUnavailable) => writeln!(
                    formatter,
                    "  NOT_RUN {} {:?}: provider unavailable",
                    batch.provider(),
                    batch.packages()
                )?,
                Err(error) => writeln!(
                    formatter,
                    "  FAILED {} {:?}: {error}",
                    batch.provider(),
                    batch.packages()
                )?,
            }
        }

        write_action_results(formatter, "manual packages", &self.manual_packages)?;
        write_action_results(formatter, "actions", &self.actions)?;

        writeln!(formatter, "\nlinks:")?;
        match &self.links {
            Ok(report) if report.results().is_empty() => writeln!(formatter, "  <none>")?,
            Ok(report) => {
                for link in report.results() {
                    match link.outcome() {
                        Ok(LinkOutcome::Satisfied) => {
                            writeln!(formatter, "  SATISFIED {}", link.id())?;
                        }
                        Ok(LinkOutcome::Created) => {
                            writeln!(formatter, "  CREATED {}", link.id())?;
                        }
                        Ok(LinkOutcome::Replaced) => {
                            writeln!(formatter, "  REPLACED {}", link.id())?;
                        }
                        Ok(LinkOutcome::SkippedMissingParent) => {
                            writeln!(formatter, "  SKIPPED {} (missing parent)", link.id())?;
                        }
                        Err(error) => {
                            writeln!(formatter, "  FAILED {}: {error}", link.id())?;
                        }
                    }
                }
            }
            Err(error) => writeln!(formatter, "  FAILED link phase: {error}")?,
        }

        write!(
            formatter,
            "\nresult: {}",
            if self.succeeded() {
                "SUCCESS"
            } else {
                "FAILED"
            }
        )
    }
}

fn write_action_results(
    formatter: &mut fmt::Formatter<'_>,
    label: &str,
    results: &[NamedActionResult],
) -> fmt::Result {
    writeln!(formatter, "\n{label}:")?;
    if results.is_empty() {
        writeln!(formatter, "  <none>")?;
    }
    for result in results {
        match &result.outcome {
            Ok(ActionOutcome::AlreadySatisfied { .. }) => {
                writeln!(formatter, "  SATISFIED {}", result.id)?;
            }
            Ok(ActionOutcome::Executed { .. }) => {
                writeln!(formatter, "  OK {} (executed)", result.id)?;
            }
            Err(error) => {
                writeln!(formatter, "  FAILED {}: {error}", result.id)?;
            }
        }
    }
    Ok(())
}

#[derive(Debug)]
pub(super) enum CommandError {
    Config(ConfigLoadError),
    Manifest(ManifestError),
    Planning(PlanningError),
    Output(io::Error),
}

impl fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(source) => source.fmt(formatter),
            Self::Manifest(source) => source.fmt(formatter),
            Self::Planning(source) => source.fmt(formatter),
            Self::Output(source) => write!(formatter, "failed to write apply output: {source}"),
        }
    }
}

impl Error for CommandError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Config(source) => Some(source),
            Self::Manifest(source) => Some(source),
            Self::Planning(source) => Some(source),
            Self::Output(source) => Some(source),
        }
    }
}

impl From<ConfigLoadError> for CommandError {
    fn from(source: ConfigLoadError) -> Self {
        Self::Config(source)
    }
}

impl From<ManifestError> for CommandError {
    fn from(source: ManifestError) -> Self {
        Self::Manifest(source)
    }
}

impl From<PlanningError> for CommandError {
    fn from(source: PlanningError) -> Self {
        Self::Planning(source)
    }
}

impl From<io::Error> for CommandError {
    fn from(source: io::Error) -> Self {
        Self::Output(source)
    }
}

use std::error::Error;
use std::fmt;

use crate::action::{
    CommandPreparationError, ExecutionEnvironment, ExecutionError, ExecutionResult, IoMode,
    PreparedCommand, ProcessExecutor,
};
use crate::interpolation::{
    DotPaths, InterpolationError, ResolveContext, XdgPaths, resolve_environment_patch,
    resolve_exec_action,
};
use crate::schema::{Entries, Provider};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderReadiness {
    Ready,
    NotReady,
}

impl fmt::Display for ProviderReadiness {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ready => formatter.write_str("READY"),
            Self::NotReady => formatter.write_str("NOT_READY"),
        }
    }
}

#[derive(Debug)]
pub struct ProviderCheckResult {
    provider: String,
    outcome: Result<ExecutionResult, ProviderCheckError>,
}

impl ProviderCheckResult {
    pub fn provider(&self) -> &str {
        &self.provider
    }

    pub fn readiness(&self) -> ProviderReadiness {
        match &self.outcome {
            Ok(output) if output.success() => ProviderReadiness::Ready,
            Ok(_) | Err(_) => ProviderReadiness::NotReady,
        }
    }

    pub fn output(&self) -> Option<&ExecutionResult> {
        self.outcome.as_ref().ok()
    }

    pub fn error(&self) -> Option<&ProviderCheckError> {
        self.outcome.as_ref().err()
    }
}

#[derive(Debug, Default)]
pub struct ProviderCheckReport {
    results: Vec<ProviderCheckResult>,
}

impl ProviderCheckReport {
    pub fn results(&self) -> &[ProviderCheckResult] {
        &self.results
    }

    pub fn all_ready(&self) -> bool {
        self.results
            .iter()
            .all(|result| result.readiness() == ProviderReadiness::Ready)
    }
}

impl fmt::Display for ProviderCheckReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.results.is_empty() {
            return formatter.write_str("No providers.");
        }

        for (index, result) in self.results.iter().enumerate() {
            if index > 0 {
                formatter.write_str("\n")?;
            }

            write!(formatter, "{} {}", result.readiness(), result.provider())?;
            match &result.outcome {
                Ok(output) => {
                    match output.code() {
                        Some(code) => write!(formatter, " (exit {code})")?,
                        None => formatter.write_str(" (no exit code)")?,
                    }
                    write_captured_output(formatter, "stdout", output.stdout())?;
                    write_captured_output(formatter, "stderr", output.stderr())?;
                }
                Err(error) => write!(formatter, "\n  error: {error}")?,
            }
        }

        Ok(())
    }
}

fn write_captured_output(
    formatter: &mut fmt::Formatter<'_>,
    label: &str,
    output: Option<&[u8]>,
) -> fmt::Result {
    let Some(output) = output.filter(|output| !output.is_empty()) else {
        return Ok(());
    };

    write!(formatter, "\n  {label}:")?;
    for line in String::from_utf8_lossy(output).lines() {
        write!(formatter, "\n    {line}")?;
    }
    Ok(())
}

#[derive(Debug)]
pub enum ProviderCheckError {
    ActivateInterpolation(InterpolationError),
    ActivatePreparation(CommandPreparationError),
    ProbeInterpolation(InterpolationError),
    ProbePreparation(CommandPreparationError),
    Execution(ExecutionError),
}

impl fmt::Display for ProviderCheckError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ActivateInterpolation(source) => {
                write!(formatter, "failed to resolve provider activate: {source}")
            }
            Self::ActivatePreparation(source) => {
                write!(formatter, "failed to apply provider activate: {source}")
            }
            Self::ProbeInterpolation(source) => {
                write!(formatter, "failed to resolve provider probe: {source}")
            }
            Self::ProbePreparation(source) => {
                write!(formatter, "failed to prepare provider probe: {source}")
            }
            Self::Execution(source) => write!(formatter, "failed to execute probe: {source}"),
        }
    }
}

impl Error for ProviderCheckError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::ActivateInterpolation(source) | Self::ProbeInterpolation(source) => Some(source),
            Self::ActivatePreparation(source) | Self::ProbePreparation(source) => Some(source),
            Self::Execution(source) => Some(source),
        }
    }
}

impl From<ExecutionError> for ProviderCheckError {
    fn from(source: ExecutionError) -> Self {
        Self::Execution(source)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ProviderChecker<'a> {
    base_environment: &'a ExecutionEnvironment,
    dot_paths: DotPaths<'a>,
    xdg_paths: &'a XdgPaths,
}

impl<'a> ProviderChecker<'a> {
    pub const fn new(
        base_environment: &'a ExecutionEnvironment,
        dot_paths: DotPaths<'a>,
        xdg_paths: &'a XdgPaths,
    ) -> Self {
        Self {
            base_environment,
            dot_paths,
            xdg_paths,
        }
    }

    pub fn check(&self, providers: &Entries<Provider>) -> ProviderCheckReport {
        let results = providers
            .iter()
            .map(|(provider_id, provider)| ProviderCheckResult {
                provider: provider_id.to_string(),
                outcome: self.check_one(provider),
            })
            .collect();
        ProviderCheckReport { results }
    }

    fn check_one(&self, provider: &Provider) -> Result<ExecutionResult, ProviderCheckError> {
        let mut environment = self.base_environment.clone();

        if let Some(activate) = &provider.activate {
            let activate = {
                let context = ResolveContext::new(&environment, self.dot_paths, self.xdg_paths);
                resolve_environment_patch(activate, &context)
                    .map_err(ProviderCheckError::ActivateInterpolation)?
            };
            environment
                .apply_patch(&activate)
                .map_err(ProviderCheckError::ActivatePreparation)?;
        }

        let probe = {
            let context = ResolveContext::new(&environment, self.dot_paths, self.xdg_paths);
            resolve_exec_action(&provider.probe, &context)
                .map_err(ProviderCheckError::ProbeInterpolation)?
        };
        let command = PreparedCommand::from_exec_action(&probe, &environment)
            .map_err(ProviderCheckError::ProbePreparation)?;
        ProcessExecutor::new()
            .execute(&command, IoMode::Capture)
            .map_err(ProviderCheckError::from)
    }
}

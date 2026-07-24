use std::error::Error;
use std::fmt;

use crate::action::{
    CommandPreparationError, ExecutionEnvironment, ExecutionError, ExecutionResult, IoMode,
    PreparedCommand, ProcessExecutor,
};
use crate::plan::{PlannedProvider, PlannedProviderInstall};
use crate::schema::ResolvedExecAction;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderStage {
    Activate,
    InitialProbe,
    Ensure(usize),
    Reactivate,
    FinalProbe,
}

impl fmt::Display for ProviderStage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Activate => formatter.write_str("activate"),
            Self::InitialProbe => formatter.write_str("initial probe"),
            Self::Ensure(index) => write!(formatter, "ensure[{index}]"),
            Self::Reactivate => formatter.write_str("post-ensure activate"),
            Self::FinalProbe => formatter.write_str("final probe"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderOutcome {
    AlreadyReady {
        probe: ExecutionResult,
    },
    Ensured {
        ensure: Vec<ExecutionResult>,
        probe: ExecutionResult,
    },
}

#[derive(Debug)]
pub struct ProviderStatus {
    id: String,
    environment: Option<ExecutionEnvironment>,
    outcome: Result<ProviderOutcome, ProviderError>,
}

impl ProviderStatus {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn is_ready(&self) -> bool {
        self.outcome.is_ok()
    }

    pub fn environment(&self) -> Option<&ExecutionEnvironment> {
        self.environment.as_ref()
    }

    pub fn outcome(&self) -> Result<&ProviderOutcome, &ProviderError> {
        self.outcome.as_ref()
    }

    pub fn error(&self) -> Option<&ProviderError> {
        self.outcome.as_ref().err()
    }
}

#[derive(Debug, Default)]
pub struct ProviderReadiness {
    statuses: Vec<ProviderStatus>,
}

impl ProviderReadiness {
    pub fn statuses(&self) -> &[ProviderStatus] {
        &self.statuses
    }

    pub fn get(&self, id: &str) -> Option<&ProviderStatus> {
        self.statuses.iter().find(|status| status.id == id)
    }

    pub fn all_ready(&self) -> bool {
        self.statuses.iter().all(ProviderStatus::is_ready)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderInstallOutcome {
    Executed { install: ExecutionResult },
    NotRunProviderUnavailable,
}

#[derive(Debug)]
pub struct ProviderInstallStatus {
    id: String,
    outcome: Result<ProviderInstallOutcome, ProviderInstallError>,
}

impl ProviderInstallStatus {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn outcome(&self) -> Result<&ProviderInstallOutcome, &ProviderInstallError> {
        self.outcome.as_ref()
    }

    pub fn error(&self) -> Option<&ProviderInstallError> {
        self.outcome.as_ref().err()
    }

    pub fn is_succeeded(&self) -> bool {
        matches!(self.outcome, Ok(ProviderInstallOutcome::Executed { .. }))
    }
}

#[derive(Debug, Default)]
pub struct ProviderInstallExecution {
    statuses: Vec<ProviderInstallStatus>,
}

impl ProviderInstallExecution {
    pub fn statuses(&self) -> &[ProviderInstallStatus] {
        &self.statuses
    }

    pub fn all_succeeded(&self) -> bool {
        self.statuses
            .iter()
            .all(ProviderInstallStatus::is_succeeded)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ProviderRunner<'a> {
    base_environment: &'a ExecutionEnvironment,
}

impl<'a> ProviderRunner<'a> {
    pub const fn new(base_environment: &'a ExecutionEnvironment) -> Self {
        Self { base_environment }
    }

    pub fn ensure_all(&self, providers: &[PlannedProvider]) -> ProviderReadiness {
        let statuses = providers
            .iter()
            .map(|provider| {
                let (environment, outcome) = match self.ensure_one(provider) {
                    Ok((environment, outcome)) => (Some(environment), Ok(outcome)),
                    Err(error) => (None, Err(error)),
                };
                ProviderStatus {
                    id: provider.id().to_owned(),
                    environment,
                    outcome,
                }
            })
            .collect();
        ProviderReadiness { statuses }
    }

    pub fn install_all(
        &self,
        installs: &[PlannedProviderInstall],
        readiness: &ProviderReadiness,
    ) -> ProviderInstallExecution {
        let statuses = installs
            .iter()
            .map(|install| {
                let outcome = match readiness
                    .get(install.provider())
                    .and_then(ProviderStatus::environment)
                {
                    Some(environment) => self.install_one(install, environment),
                    None => Ok(ProviderInstallOutcome::NotRunProviderUnavailable),
                };
                ProviderInstallStatus {
                    id: install.id().to_owned(),
                    outcome,
                }
            })
            .collect();
        ProviderInstallExecution { statuses }
    }

    fn install_one(
        &self,
        install: &PlannedProviderInstall,
        environment: &ExecutionEnvironment,
    ) -> Result<ProviderInstallOutcome, ProviderInstallError> {
        let command = PreparedCommand::from_exec_action(install.install(), environment)
            .map_err(|source| ProviderInstallError::Preparation { source })?;
        let result = ProcessExecutor::new()
            .execute(&command, IoMode::Inherit)
            .map_err(|source| ProviderInstallError::Execution { source })?;
        if !result.success() {
            return Err(ProviderInstallError::UnsuccessfulExit { result });
        }
        Ok(ProviderInstallOutcome::Executed { install: result })
    }

    fn ensure_one(
        &self,
        provider: &PlannedProvider,
    ) -> Result<(ExecutionEnvironment, ProviderOutcome), ProviderError> {
        let environment = self.activate(provider, ProviderStage::Activate)?;

        match self.probe(provider.probe(), &environment, ProviderStage::InitialProbe) {
            Ok(probe) => {
                return Ok((environment, ProviderOutcome::AlreadyReady { probe }));
            }
            Err(error) if provider.ensure().is_empty() || !error.can_be_ensured() => {
                return Err(error);
            }
            Err(_) => {}
        }

        let mut ensure_results = Vec::with_capacity(provider.ensure().len());
        for (index, ensure) in provider.ensure().iter().enumerate() {
            let stage = ProviderStage::Ensure(index);
            let command = self.prepare(ensure, &environment, stage)?;
            let result = self.execute(&command, stage, IoMode::Inherit)?;
            if !result.success() {
                return Err(ProviderError::UnsuccessfulExit { stage, result });
            }
            ensure_results.push(result);
        }

        let environment = self.activate(provider, ProviderStage::Reactivate)?;
        let probe = self.probe(provider.probe(), &environment, ProviderStage::FinalProbe)?;
        Ok((
            environment,
            ProviderOutcome::Ensured {
                ensure: ensure_results,
                probe,
            },
        ))
    }

    fn activate(
        &self,
        provider: &PlannedProvider,
        stage: ProviderStage,
    ) -> Result<ExecutionEnvironment, ProviderError> {
        let mut environment = self.base_environment.clone();
        if let Some(activate) = provider.activate() {
            environment
                .apply_patch(activate)
                .map_err(|source| ProviderError::Environment { stage, source })?;
        }
        Ok(environment)
    }

    fn probe(
        &self,
        probe: &ResolvedExecAction,
        environment: &ExecutionEnvironment,
        stage: ProviderStage,
    ) -> Result<ExecutionResult, ProviderError> {
        let command = self.prepare(probe, environment, stage)?;
        let result = self.execute(&command, stage, IoMode::Capture)?;
        if result.success() {
            Ok(result)
        } else {
            Err(ProviderError::UnsuccessfulExit { stage, result })
        }
    }

    fn prepare(
        &self,
        action: &ResolvedExecAction,
        environment: &ExecutionEnvironment,
        stage: ProviderStage,
    ) -> Result<PreparedCommand, ProviderError> {
        PreparedCommand::from_exec_action(action, environment)
            .map_err(|source| ProviderError::Preparation { stage, source })
    }

    fn execute(
        &self,
        command: &PreparedCommand,
        stage: ProviderStage,
        io_mode: IoMode,
    ) -> Result<ExecutionResult, ProviderError> {
        ProcessExecutor::new()
            .execute(command, io_mode)
            .map_err(|source| ProviderError::Execution { stage, source })
    }
}

#[derive(Debug)]
pub enum ProviderError {
    Environment {
        stage: ProviderStage,
        source: CommandPreparationError,
    },
    Preparation {
        stage: ProviderStage,
        source: CommandPreparationError,
    },
    Execution {
        stage: ProviderStage,
        source: ExecutionError,
    },
    UnsuccessfulExit {
        stage: ProviderStage,
        result: ExecutionResult,
    },
}

impl ProviderError {
    pub const fn stage(&self) -> ProviderStage {
        match self {
            Self::Environment { stage, .. }
            | Self::Preparation { stage, .. }
            | Self::Execution { stage, .. }
            | Self::UnsuccessfulExit { stage, .. } => *stage,
        }
    }

    pub const fn exit_result(&self) -> Option<&ExecutionResult> {
        match self {
            Self::UnsuccessfulExit { result, .. } => Some(result),
            Self::Environment { .. } | Self::Preparation { .. } | Self::Execution { .. } => None,
        }
    }

    const fn can_be_ensured(&self) -> bool {
        matches!(self, Self::Execution { .. } | Self::UnsuccessfulExit { .. })
    }
}

impl fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Environment { stage, source } => {
                write!(formatter, "failed to apply provider {stage}: {source}")
            }
            Self::Preparation { stage, source } => {
                write!(formatter, "failed to prepare provider {stage}: {source}")
            }
            Self::Execution { stage, source } => {
                write!(formatter, "failed to execute provider {stage}: {source}")
            }
            Self::UnsuccessfulExit { stage, result } => {
                write!(formatter, "provider {stage} returned {}", result.status())
            }
        }
    }
}

impl Error for ProviderError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Environment { source, .. } | Self::Preparation { source, .. } => Some(source),
            Self::Execution { source, .. } => Some(source),
            Self::UnsuccessfulExit { .. } => None,
        }
    }
}

#[derive(Debug)]
pub enum ProviderInstallError {
    Preparation { source: CommandPreparationError },
    Execution { source: ExecutionError },
    UnsuccessfulExit { result: ExecutionResult },
}

impl ProviderInstallError {
    pub const fn exit_result(&self) -> Option<&ExecutionResult> {
        match self {
            Self::UnsuccessfulExit { result } => Some(result),
            Self::Preparation { .. } | Self::Execution { .. } => None,
        }
    }
}

impl fmt::Display for ProviderInstallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Preparation { source } => {
                write!(formatter, "failed to prepare provider install: {source}")
            }
            Self::Execution { source } => {
                write!(formatter, "failed to execute provider install: {source}")
            }
            Self::UnsuccessfulExit { result } => {
                write!(formatter, "provider install returned {}", result.status())
            }
        }
    }
}

impl Error for ProviderInstallError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Preparation { source } => Some(source),
            Self::Execution { source } => Some(source),
            Self::UnsuccessfulExit { .. } => None,
        }
    }
}

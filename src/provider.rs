use std::error::Error;
use std::fmt;

use crate::action::{
    CommandPreparationError, ExecutionEnvironment, ExecutionError, ExecutionResult, IoMode,
    PreparedCommand, ProcessExecutor,
};
use crate::plan::{PlannedProvider, PlannedProviderBatch};
use crate::schema::ExecAction;

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
pub enum ProviderBatchOutcome {
    Executed { install: ExecutionResult },
    NotRunProviderUnavailable,
}

#[derive(Debug)]
pub struct ProviderBatchStatus {
    provider: String,
    provider_args: Vec<String>,
    packages: Vec<String>,
    outcome: Result<ProviderBatchOutcome, ProviderBatchError>,
}

impl ProviderBatchStatus {
    pub fn provider(&self) -> &str {
        &self.provider
    }

    pub fn provider_args(&self) -> &[String] {
        &self.provider_args
    }

    pub fn packages(&self) -> &[String] {
        &self.packages
    }

    pub fn outcome(&self) -> Result<&ProviderBatchOutcome, &ProviderBatchError> {
        self.outcome.as_ref()
    }

    pub fn error(&self) -> Option<&ProviderBatchError> {
        self.outcome.as_ref().err()
    }

    pub fn is_succeeded(&self) -> bool {
        matches!(self.outcome, Ok(ProviderBatchOutcome::Executed { .. }))
    }
}

#[derive(Debug, Default)]
pub struct ProviderBatchExecution {
    statuses: Vec<ProviderBatchStatus>,
}

impl ProviderBatchExecution {
    pub fn statuses(&self) -> &[ProviderBatchStatus] {
        &self.statuses
    }

    pub fn all_succeeded(&self) -> bool {
        self.statuses.iter().all(ProviderBatchStatus::is_succeeded)
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

    pub fn install_batches(
        &self,
        batches: &[PlannedProviderBatch],
        readiness: &ProviderReadiness,
    ) -> ProviderBatchExecution {
        let statuses = batches
            .iter()
            .map(|batch| {
                let outcome = match readiness
                    .get(batch.provider())
                    .and_then(ProviderStatus::environment)
                {
                    Some(environment) => self.install_batch(batch, environment),
                    None => Ok(ProviderBatchOutcome::NotRunProviderUnavailable),
                };
                ProviderBatchStatus {
                    provider: batch.provider().to_owned(),
                    provider_args: batch.provider_args().to_owned(),
                    packages: batch.packages().to_owned(),
                    outcome,
                }
            })
            .collect();
        ProviderBatchExecution { statuses }
    }

    fn install_batch(
        &self,
        batch: &PlannedProviderBatch,
        environment: &ExecutionEnvironment,
    ) -> Result<ProviderBatchOutcome, ProviderBatchError> {
        let command = PreparedCommand::from_exec_action(batch.install(), environment)
            .map_err(|source| ProviderBatchError::Preparation { source })?;
        let install = ProcessExecutor::new()
            .execute(&command, IoMode::Inherit)
            .map_err(|source| ProviderBatchError::Execution { source })?;
        if !install.success() {
            return Err(ProviderBatchError::UnsuccessfulExit { result: install });
        }
        Ok(ProviderBatchOutcome::Executed { install })
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
        probe: &ExecAction,
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
        action: &ExecAction,
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
pub enum ProviderBatchError {
    Preparation { source: CommandPreparationError },
    Execution { source: ExecutionError },
    UnsuccessfulExit { result: ExecutionResult },
}

impl ProviderBatchError {
    pub const fn exit_result(&self) -> Option<&ExecutionResult> {
        match self {
            Self::UnsuccessfulExit { result } => Some(result),
            Self::Preparation { .. } | Self::Execution { .. } => None,
        }
    }
}

impl fmt::Display for ProviderBatchError {
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

impl Error for ProviderBatchError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Preparation { source } => Some(source),
            Self::Execution { source } => Some(source),
            Self::UnsuccessfulExit { .. } => None,
        }
    }
}

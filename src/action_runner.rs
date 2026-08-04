use std::fmt;

pub use crate::fetch_content::{FetchContentError, FetchContentOutcome};

use crate::action::{
    CommandPreparationError, ExecutionEnvironment, ExecutionError, ExecutionResult, IoMode,
    PreparedCommand, ProcessExecutor,
};
use crate::schema::{ResolvedCommandAction, ResolvedExecAction};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionStage {
    InitialCheck,
    Exec,
    PostCheck,
}

impl fmt::Display for ActionStage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InitialCheck => "initial check",
            Self::Exec => "exec",
            Self::PostCheck => "post-check",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandActionOutcome {
    AlreadySatisfied {
        check: ExecutionResult,
    },
    Executed {
        initial_check: Option<ExecutionResult>,
        exec: ExecutionResult,
        post_check: Option<ExecutionResult>,
    },
}

#[derive(Clone, Copy, Debug)]
pub struct CommandActionRunner<'a> {
    environment: &'a ExecutionEnvironment,
}

impl<'a> CommandActionRunner<'a> {
    pub const fn new(environment: &'a ExecutionEnvironment) -> Self {
        Self { environment }
    }

    pub fn run(
        &self,
        action: &ResolvedCommandAction,
    ) -> Result<CommandActionOutcome, CommandActionRunError> {
        let initial_check = match &action.check {
            None => None,
            Some(check) => {
                let command = self.prepare(check, ActionStage::InitialCheck)?;
                let result = self.execute(&command, ActionStage::InitialCheck, IoMode::Capture)?;
                match result.code() {
                    Some(0) => return Ok(CommandActionOutcome::AlreadySatisfied { check: result }),
                    Some(1) => Some((command, result)),
                    _ => {
                        return Err(CommandActionRunError::UnsuccessfulExit {
                            stage: ActionStage::InitialCheck,
                            result,
                        });
                    }
                }
            }
        };

        let exec = self.prepare(&action.exec, ActionStage::Exec)?;
        let exec = self.execute(&exec, ActionStage::Exec, IoMode::Inherit)?;
        if !exec.success() {
            return Err(CommandActionRunError::UnsuccessfulExit {
                stage: ActionStage::Exec,
                result: exec,
            });
        }

        let (initial_check, post_check) = match initial_check {
            None => (None, None),
            Some((check, initial_result)) => {
                let result = self.execute(&check, ActionStage::PostCheck, IoMode::Capture)?;
                if result.code() != Some(0) {
                    return Err(CommandActionRunError::UnsuccessfulExit {
                        stage: ActionStage::PostCheck,
                        result,
                    });
                }
                (Some(initial_result), Some(result))
            }
        };

        Ok(CommandActionOutcome::Executed {
            initial_check,
            exec,
            post_check,
        })
    }

    fn prepare(
        &self,
        action: &ResolvedExecAction,
        stage: ActionStage,
    ) -> Result<PreparedCommand, CommandActionRunError> {
        PreparedCommand::from_exec_action(action, self.environment)
            .map_err(|source| CommandActionRunError::Preparation { stage, source })
    }

    fn execute(
        &self,
        command: &PreparedCommand,
        stage: ActionStage,
        io_mode: IoMode,
    ) -> Result<ExecutionResult, CommandActionRunError> {
        ProcessExecutor::new()
            .execute(command, io_mode)
            .map_err(|source| CommandActionRunError::Execution { stage, source })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CommandActionRunError {
    #[error("failed to prepare action {stage}: {source}")]
    Preparation {
        stage: ActionStage,
        #[source]
        source: CommandPreparationError,
    },
    #[error("failed to execute action {stage}: {source}")]
    Execution {
        stage: ActionStage,
        #[source]
        source: ExecutionError,
    },
    #[error("action {stage} returned {}", .result.status())]
    UnsuccessfulExit {
        stage: ActionStage,
        result: ExecutionResult,
    },
}

impl CommandActionRunError {
    pub const fn stage(&self) -> ActionStage {
        match self {
            Self::Preparation { stage, .. }
            | Self::Execution { stage, .. }
            | Self::UnsuccessfulExit { stage, .. } => *stage,
        }
    }

    pub const fn exit_result(&self) -> Option<&ExecutionResult> {
        match self {
            Self::UnsuccessfulExit { result, .. } => Some(result),
            Self::Preparation { .. } | Self::Execution { .. } => None,
        }
    }
}

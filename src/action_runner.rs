use std::error::Error;
use std::fmt;

use crate::action::{
    CommandPreparationError, ExecutionEnvironment, ExecutionError, ExecutionResult, IoMode,
    PreparedCommand, ProcessExecutor,
};
use crate::schema::{Action, ExecAction};

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
pub enum ActionOutcome {
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
pub struct ActionRunner<'a> {
    environment: &'a ExecutionEnvironment,
}

impl<'a> ActionRunner<'a> {
    pub const fn new(environment: &'a ExecutionEnvironment) -> Self {
        Self { environment }
    }

    pub fn run(&self, action: &Action) -> Result<ActionOutcome, ActionRunError> {
        let initial_check = match &action.check {
            None => None,
            Some(check) => {
                let command = self.prepare(check, ActionStage::InitialCheck)?;
                let result = self.execute(&command, ActionStage::InitialCheck, IoMode::Capture)?;
                match result.code() {
                    Some(0) => return Ok(ActionOutcome::AlreadySatisfied { check: result }),
                    Some(1) => Some((command, result)),
                    _ => {
                        return Err(ActionRunError::UnsuccessfulExit {
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
            return Err(ActionRunError::UnsuccessfulExit {
                stage: ActionStage::Exec,
                result: exec,
            });
        }

        let (initial_check, post_check) = match initial_check {
            None => (None, None),
            Some((check, initial_result)) => {
                let result = self.execute(&check, ActionStage::PostCheck, IoMode::Capture)?;
                if result.code() != Some(0) {
                    return Err(ActionRunError::UnsuccessfulExit {
                        stage: ActionStage::PostCheck,
                        result,
                    });
                }
                (Some(initial_result), Some(result))
            }
        };

        Ok(ActionOutcome::Executed {
            initial_check,
            exec,
            post_check,
        })
    }

    fn prepare(
        &self,
        action: &ExecAction,
        stage: ActionStage,
    ) -> Result<PreparedCommand, ActionRunError> {
        PreparedCommand::from_exec_action(action, self.environment)
            .map_err(|source| ActionRunError::Preparation { stage, source })
    }

    fn execute(
        &self,
        command: &PreparedCommand,
        stage: ActionStage,
        io_mode: IoMode,
    ) -> Result<ExecutionResult, ActionRunError> {
        ProcessExecutor::new()
            .execute(command, io_mode)
            .map_err(|source| ActionRunError::Execution { stage, source })
    }
}

#[derive(Debug)]
pub enum ActionRunError {
    Preparation {
        stage: ActionStage,
        source: CommandPreparationError,
    },
    Execution {
        stage: ActionStage,
        source: ExecutionError,
    },
    UnsuccessfulExit {
        stage: ActionStage,
        result: ExecutionResult,
    },
}

impl ActionRunError {
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

impl fmt::Display for ActionRunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Preparation { stage, source } => {
                write!(formatter, "failed to prepare action {stage}: {source}")
            }
            Self::Execution { stage, source } => {
                write!(formatter, "failed to execute action {stage}: {source}")
            }
            Self::UnsuccessfulExit { stage, result } => {
                write!(formatter, "action {stage} returned {}", result.status())
            }
        }
    }
}

impl Error for ActionRunError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Preparation { source, .. } => Some(source),
            Self::Execution { source, .. } => Some(source),
            Self::UnsuccessfulExit { .. } => None,
        }
    }
}

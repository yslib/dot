use std::env;
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

use crate::interpolation::ExecutionEnvironment;
use crate::schema::{OneOrMany, ResolvedEnvironmentPatch, ResolvedExecAction, ResolvedString};

pub fn apply_environment_patch(
    environment: &mut ExecutionEnvironment,
    patch: &ResolvedEnvironmentPatch,
) -> Result<(), CommandPreparationError> {
    for (name, value) in &patch.variables {
        environment.insert(name.as_str(), value.value());
    }

    if patch.path_prepend.is_some() || patch.path_append.is_some() {
        let mut paths = Vec::new();
        if let Some(prepend) = &patch.path_prepend {
            paths.extend(values(prepend).map(PathBuf::from));
        }
        if let Some(current) = environment.get("PATH") {
            paths.extend(env::split_paths(current));
        }
        if let Some(append) = &patch.path_append {
            paths.extend(values(append).map(PathBuf::from));
        }

        let path = env::join_paths(paths)
            .map_err(|source| CommandPreparationError::InvalidPathEnvironment { source })?;
        environment.insert("PATH", path);
    }

    Ok(())
}

fn values(value: &OneOrMany<ResolvedString>) -> impl Iterator<Item = &str> {
    let values = match value {
        OneOrMany::One(value) => std::slice::from_ref(value),
        OneOrMany::Many(values) => values.as_slice(),
    };
    values.iter().map(ResolvedString::value)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedCommand {
    program: OsString,
    args: Vec<OsString>,
    cwd: Option<PathBuf>,
    environment: ExecutionEnvironment,
}

impl PreparedCommand {
    pub fn from_exec_action(
        action: &ResolvedExecAction,
        base_environment: &ExecutionEnvironment,
    ) -> Result<Self, CommandPreparationError> {
        let mut environment = base_environment.clone();
        if let Some(patch) = &action.env {
            apply_environment_patch(&mut environment, patch)?;
        }

        Ok(Self {
            program: OsString::from(action.program.value()),
            args: action
                .args
                .iter()
                .map(|argument| OsString::from(argument.value()))
                .collect(),
            cwd: action
                .cwd
                .as_ref()
                .map(|directory| PathBuf::from(directory.value())),
            environment,
        })
    }

    pub fn program(&self) -> &OsStr {
        &self.program
    }

    pub fn args(&self) -> &[OsString] {
        &self.args
    }

    pub fn cwd(&self) -> Option<&Path> {
        self.cwd.as_deref()
    }

    pub fn environment(&self) -> &ExecutionEnvironment {
        &self.environment
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CommandPreparationError {
    #[error("the effective PATH contains an invalid path entry")]
    InvalidPathEnvironment {
        #[source]
        source: env::JoinPathsError,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IoMode {
    Inherit,
    Capture,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionResult {
    status: ExitStatus,
    stdout: Option<Vec<u8>>,
    stderr: Option<Vec<u8>>,
}

impl ExecutionResult {
    pub fn status(&self) -> ExitStatus {
        self.status
    }

    pub fn success(&self) -> bool {
        self.status.success()
    }

    pub fn code(&self) -> Option<i32> {
        self.status.code()
    }

    pub fn stdout(&self) -> Option<&[u8]> {
        self.stdout.as_deref()
    }

    pub fn stderr(&self) -> Option<&[u8]> {
        self.stderr.as_deref()
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ProcessExecutor;

impl ProcessExecutor {
    pub const fn new() -> Self {
        Self
    }

    pub fn execute(
        &self,
        command: &PreparedCommand,
        io_mode: IoMode,
    ) -> Result<ExecutionResult, ExecutionError> {
        let mut process = Command::new(command.program());
        process.args(command.args());
        process.env_clear();
        process.envs(
            command
                .environment
                .variables
                .iter()
                .map(|(name, value)| (name, value)),
        );
        if let Some(cwd) = command.cwd() {
            process.current_dir(cwd);
        }

        match io_mode {
            IoMode::Inherit => {
                process
                    .stdin(Stdio::inherit())
                    .stdout(Stdio::inherit())
                    .stderr(Stdio::inherit());
            }
            IoMode::Capture => {
                process
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped());
            }
        }

        let mut child = process.spawn().map_err(|source| ExecutionError::Spawn {
            program: command.program.clone(),
            source,
        })?;

        match io_mode {
            IoMode::Inherit => {
                let status = child.wait().map_err(|source| ExecutionError::Wait {
                    program: command.program.clone(),
                    source,
                })?;
                Ok(ExecutionResult {
                    status,
                    stdout: None,
                    stderr: None,
                })
            }
            IoMode::Capture => {
                let output = child
                    .wait_with_output()
                    .map_err(|source| ExecutionError::Wait {
                        program: command.program.clone(),
                        source,
                    })?;
                Ok(ExecutionResult {
                    status: output.status,
                    stdout: Some(output.stdout),
                    stderr: Some(output.stderr),
                })
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ExecutionError {
    #[error("failed to start `{}`: {source}", .program.to_string_lossy())]
    Spawn {
        program: OsString,
        #[source]
        source: io::Error,
    },
    #[error("failed while waiting for `{}`: {source}", .program.to_string_lossy())]
    Wait {
        program: OsString,
        #[source]
        source: io::Error,
    },
}

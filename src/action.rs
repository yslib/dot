use std::env;
use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

use crate::schema::{OneOrMany, ResolvedEnvironmentPatch, ResolvedExecAction, ResolvedString};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExecutionEnvironment {
    variables: Vec<(OsString, OsString)>,
}

impl ExecutionEnvironment {
    pub fn capture() -> Self {
        let mut environment = Self::empty();
        for (name, value) in env::vars_os() {
            environment.insert(name, value);
        }
        environment
    }

    pub const fn empty() -> Self {
        Self {
            variables: Vec::new(),
        }
    }

    pub fn get(&self, name: impl AsRef<OsStr>) -> Option<&OsStr> {
        let name = name.as_ref();
        self.variables
            .iter()
            .find(|(candidate, _)| environment_names_equal(candidate, name))
            .map(|(_, value)| value.as_os_str())
    }

    pub fn apply_patch(
        &mut self,
        patch: &ResolvedEnvironmentPatch,
    ) -> Result<(), CommandPreparationError> {
        for (name, value) in &patch.variables {
            self.insert(name.as_str(), value.value());
        }

        if patch.path_prepend.is_some() || patch.path_append.is_some() {
            let mut paths = Vec::new();
            if let Some(prepend) = &patch.path_prepend {
                paths.extend(values(prepend).map(PathBuf::from));
            }
            if let Some(current) = self.get("PATH") {
                paths.extend(env::split_paths(current));
            }
            if let Some(append) = &patch.path_append {
                paths.extend(values(append).map(PathBuf::from));
            }

            let path = env::join_paths(paths)
                .map_err(|source| CommandPreparationError::InvalidPathEnvironment { source })?;
            self.insert("PATH", path);
        }

        Ok(())
    }

    fn insert(&mut self, name: impl Into<OsString>, value: impl Into<OsString>) {
        let name = name.into();
        let value = value.into();

        if let Some((stored_name, stored_value)) = self
            .variables
            .iter_mut()
            .find(|(candidate, _)| environment_names_equal(candidate, &name))
        {
            *stored_name = name;
            *stored_value = value;
        } else {
            self.variables.push((name, value));
        }
    }

    fn iter(&self) -> impl Iterator<Item = (&OsStr, &OsStr)> {
        self.variables
            .iter()
            .map(|(name, value)| (name.as_os_str(), value.as_os_str()))
    }
}

#[cfg(windows)]
fn environment_names_equal(left: &OsStr, right: &OsStr) -> bool {
    left.to_string_lossy()
        .eq_ignore_ascii_case(&right.to_string_lossy())
}

#[cfg(not(windows))]
fn environment_names_equal(left: &OsStr, right: &OsStr) -> bool {
    left == right
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
    /// Prepares a command only from a resolved action.
    ///
    /// Source expressions cannot cross the execution boundary:
    ///
    /// ```compile_fail,E0308
    /// use dot::action::{ExecutionEnvironment, PreparedCommand};
    /// use dot::schema::{ExecAction, StringExpressionSource};
    ///
    /// let source_action =
    ///     ExecAction::<StringExpressionSource, StringExpressionSource> {
    ///         kind: None,
    ///         program: "echo".into(),
    ///         args: vec!["${env:HOME}".into()],
    ///         cwd: None,
    ///         env: None,
    ///     };
    ///
    /// let _ = PreparedCommand::from_exec_action(
    ///     &source_action,
    ///     &ExecutionEnvironment::empty(),
    /// );
    /// ```
    pub fn from_exec_action(
        action: &ResolvedExecAction,
        base_environment: &ExecutionEnvironment,
    ) -> Result<Self, CommandPreparationError> {
        let mut environment = base_environment.clone();
        if let Some(patch) = &action.env {
            environment.apply_patch(patch)?;
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

#[derive(Debug)]
pub enum CommandPreparationError {
    InvalidPathEnvironment { source: env::JoinPathsError },
}

impl fmt::Display for CommandPreparationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPathEnvironment { .. } => {
                formatter.write_str("the effective PATH contains an invalid path entry")
            }
        }
    }
}

impl Error for CommandPreparationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidPathEnvironment { source } => Some(source),
        }
    }
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
        process.envs(command.environment.iter());
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

#[derive(Debug)]
pub enum ExecutionError {
    Spawn {
        program: OsString,
        source: io::Error,
    },
    Wait {
        program: OsString,
        source: io::Error,
    },
}

impl fmt::Display for ExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn { program, source } => write!(
                formatter,
                "failed to start `{}`: {source}",
                program.to_string_lossy()
            ),
            Self::Wait { program, source } => write!(
                formatter,
                "failed while waiting for `{}`: {source}",
                program.to_string_lossy()
            ),
        }
    }
}

impl Error for ExecutionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Spawn { source, .. } | Self::Wait { source, .. } => Some(source),
        }
    }
}

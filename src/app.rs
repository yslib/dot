mod apply;
mod check_providers;
mod command;
mod dry_run;
mod list_jobs;
mod list_profiles;
mod list_targets;

use std::error::Error;
use std::fmt;
use std::io::{self, IsTerminal};
use std::process::ExitCode;

pub use command::{Dispatch, ExecutionRequest, Operation, ProfileSelection, ScopeSelection};

use crate::config::ConfigLoadError;
use crate::manifest::ManifestError;
use crate::output::{TableRenderer, TsvRecord, TsvRenderer};
use crate::platform::PlatformInfo;
use crate::report::{CommandReport, ReportStatus};

pub fn run(dispatch: Dispatch) -> ExitCode {
    let Dispatch {
        config,
        operation,
        platform_override,
    } = dispatch;

    if platform_override.is_some() {
        print_platform_warning(&operation);
    }

    match operation {
        Operation::Apply(request) => match apply::run(&config, &request) {
            Ok(report) => render_report(&report),
            Err(error) => command_error(error),
        },
        Operation::DryRun(request) => {
            match dry_run::run(&config, &request, platform_override.as_ref()) {
                Ok(report) => render_report(&report),
                Err(error) => command_error(error),
            }
        }
        Operation::CheckProviders(scope) => {
            match check_providers::run(&config, &scope, platform_override.as_ref()) {
                Ok(report) => render_report(&report),
                Err(error) => command_error(error),
            }
        }
        Operation::ListTargets { all } => {
            let platform = compatibility_platform(platform_override.as_ref());
            match list_targets::Catalog::load(&config) {
                Ok(catalog) => render_list(catalog.records(&platform, all)),
                Err(error) => command_error(error),
            }
        }
        Operation::ListProfiles { target } => {
            let platform = compatibility_platform(platform_override.as_ref());
            match list_profiles::Catalog::load(&config, &platform, target.as_ref()) {
                Ok(catalog) => render_list(catalog.records()),
                Err(error) => command_error(error),
            }
        }
        Operation::ListJobs(scope) => {
            let platform = compatibility_platform(platform_override.as_ref());
            match list_jobs::Catalog::load(&config, &platform, &scope) {
                Ok(catalog) => render_list(catalog.records()),
                Err(error) => command_error(error),
            }
        }
    }
}

fn compatibility_platform(platform_override: Option<&PlatformInfo>) -> PlatformInfo {
    platform_override
        .cloned()
        .unwrap_or_else(PlatformInfo::detect)
}

fn print_platform_warning(operation: &Operation) {
    match operation {
        Operation::Apply(_) => {
            eprintln!(
                "dot: warning: --platform is ignored by apply; detected host PlatformInfo will be used"
            );
        }
        Operation::DryRun(_) | Operation::CheckProviders(_) => {
            eprintln!(
                "dot: warning: --platform affects target compatibility only; host environment, XDG paths, commands, and filesystem state remain unchanged"
            );
        }
        Operation::ListTargets { .. } => {
            eprintln!(
                "dot: warning: --platform affects compatibility labels and default filtering for list targets"
            );
        }
        Operation::ListProfiles { .. } | Operation::ListJobs(_) => {
            eprintln!(
                "dot: warning: --platform affects only omitted-target inference for this list command"
            );
        }
    }
}

fn command_error(error: impl fmt::Display) -> ExitCode {
    eprintln!("dot: {error}");
    ExitCode::FAILURE
}

fn render_report(report: &CommandReport) -> ExitCode {
    let stdout = io::stdout();
    let renderer = TableRenderer::new(stdout.is_terminal());
    if let Err(error) = renderer.render(report, &mut stdout.lock()) {
        eprintln!("dot: failed to write command output: {error}");
        return ExitCode::FAILURE;
    }
    match report.status {
        ReportStatus::Planned | ReportStatus::Succeeded => ExitCode::SUCCESS,
        ReportStatus::Failed => ExitCode::FAILURE,
    }
}

fn render_list<R: TsvRecord>(records: Vec<R>) -> ExitCode {
    let prepared = match TsvRenderer.prepare(&records) {
        Ok(prepared) => prepared,
        Err(error) => {
            eprintln!("dot: failed to prepare list output: {error}");
            return ExitCode::FAILURE;
        }
    };

    let stdout = io::stdout();
    match normalize_list_output(prepared.render(&mut stdout.lock())) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("dot: failed to write list output: {error}");
            ExitCode::FAILURE
        }
    }
}

fn normalize_list_output(result: io::Result<()>) -> io::Result<()> {
    match result {
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        result => result,
    }
}

#[derive(Debug)]
enum ListCommandError {
    Config(ConfigLoadError),
    Manifest(ManifestError),
}

impl fmt::Display for ListCommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(source) => source.fmt(formatter),
            Self::Manifest(source) => source.fmt(formatter),
        }
    }
}

impl Error for ListCommandError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Config(source) => Some(source),
            Self::Manifest(source) => Some(source),
        }
    }
}

impl From<ConfigLoadError> for ListCommandError {
    fn from(source: ConfigLoadError) -> Self {
        Self::Config(source)
    }
}

impl From<ManifestError> for ListCommandError {
    fn from(source: ManifestError) -> Self {
        Self::Manifest(source)
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_list_output;
    use std::io;

    #[test]
    fn list_output_treats_broken_pipe_as_success() {
        let result =
            normalize_list_output(Err(io::Error::new(io::ErrorKind::BrokenPipe, "injected")));

        assert!(result.is_ok());
    }

    #[test]
    fn list_output_preserves_other_io_errors() {
        let error = normalize_list_output(Err(io::Error::other("injected")))
            .expect_err("non-pipe output errors should remain errors");

        assert_eq!(error.kind(), io::ErrorKind::Other);
    }
}

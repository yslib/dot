mod apply;
mod check_providers;
mod command;
mod dry_run;

use std::io::{self, IsTerminal};
use std::process::ExitCode;

pub use command::{Dispatch, Operation, Selection};

use crate::output::TableRenderer;
use crate::report::{CommandReport, ReportStatus};

pub fn run(dispatch: Dispatch) -> ExitCode {
    if dispatch.platform_override.is_some() {
        match dispatch.operation {
            Operation::Apply { dry_run: false } => {
                eprintln!(
                    "dot: warning: --platform is ignored by apply; detected host PlatformInfo will be used"
                );
            }
            Operation::Apply { dry_run: true } | Operation::CheckProviders => {
                eprintln!(
                    "dot: warning: --platform overrides PlatformInfo for target selection only; commands, environment, XDG paths, and filesystem state still come from the host"
                );
            }
        }
    }

    match dispatch.operation {
        Operation::Apply { dry_run: true } => {
            match dry_run::run(&dispatch.selection, dispatch.platform_override.as_ref()) {
                Ok(report) => render_report(&report),
                Err(error) => {
                    eprintln!("dot: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        Operation::Apply { dry_run: false } => match apply::run(&dispatch.selection) {
            Ok(report) => render_report(&report),
            Err(error) => {
                eprintln!("dot: {error}");
                ExitCode::FAILURE
            }
        },
        Operation::CheckProviders => {
            match check_providers::run(&dispatch.selection, dispatch.platform_override.as_ref()) {
                Ok(report) => render_report(&report),
                Err(error) => {
                    eprintln!("dot: {error}");
                    ExitCode::FAILURE
                }
            }
        }
    }
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

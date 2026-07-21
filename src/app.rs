mod check_providers;
mod command;
mod dry_run;

use std::io;
use std::process::ExitCode;

pub use command::{Dispatch, Operation, Selection};

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
            let stdout = io::stdout();
            match dry_run::run(
                &dispatch.selection,
                dispatch.platform_override.as_ref(),
                &mut stdout.lock(),
            ) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("dot: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        Operation::Apply { dry_run: false } => {
            println!("{dispatch:#?}");
            ExitCode::SUCCESS
        }
        Operation::CheckProviders => {
            let stdout = io::stdout();
            match check_providers::run(
                &dispatch.selection,
                dispatch.platform_override.as_ref(),
                &mut stdout.lock(),
            ) {
                Ok(true) => ExitCode::SUCCESS,
                Ok(false) => ExitCode::FAILURE,
                Err(error) => {
                    eprintln!("dot: {error}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}

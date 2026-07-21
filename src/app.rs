mod check_providers;
mod command;
mod dry_run;

use std::io;
use std::process::ExitCode;

pub use command::{Dispatch, Operation, Selection};

pub fn run(dispatch: Dispatch) -> ExitCode {
    match dispatch.operation {
        Operation::Apply { dry_run: true } => {
            let stdout = io::stdout();
            match dry_run::run(&dispatch.selection, &mut stdout.lock()) {
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
            match check_providers::run(&dispatch.selection, &mut stdout.lock()) {
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

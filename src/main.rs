use std::process::ExitCode;

fn main() -> ExitCode {
    dot::app::run(dot::cli::parse())
}

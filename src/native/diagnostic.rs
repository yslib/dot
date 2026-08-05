//! Native operating-system diagnostics.

use std::io;

pub use crate::report::ErrorHint;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    CreateSymbolicLink,
    StartProcess,
}

#[derive(Clone, Copy, Debug)]
struct Mapping {
    operation: Operation,
    raw_code: i32,
    code: &'static str,
    summary: &'static str,
    suggestion: &'static str,
}

const WINDOWS_MAPPINGS: &[Mapping] = &[Mapping {
    operation: Operation::CreateSymbolicLink,
    raw_code: 1314,
    code: "windows.symlink.privilege-required",
    summary: "symbolic-link creation requires permission",
    suggestion: "enable Windows Developer Mode or run dot from an elevated shell",
}];

pub fn lookup(os: &str, operation: Operation, error: &io::Error) -> Option<ErrorHint> {
    let raw_code = error.raw_os_error()?;
    let mappings = match os {
        "windows" => WINDOWS_MAPPINGS,
        _ => return None,
    };
    let mapping = mappings
        .iter()
        .find(|mapping| mapping.operation == operation && mapping.raw_code == raw_code)?;

    Some(ErrorHint {
        code: mapping.code.to_owned(),
        summary: mapping.summary.to_owned(),
        suggestion: mapping.suggestion.to_owned(),
    })
}

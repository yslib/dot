use std::io;

mod windows;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    CreateSymbolicLink,
    StartProcess,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ErrorHint {
    pub code: String,
    pub summary: String,
    pub suggestion: String,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Mapping {
    pub operation: Operation,
    pub raw_code: i32,
    pub code: &'static str,
    pub summary: &'static str,
    pub suggestion: &'static str,
}

pub fn lookup(os: &str, operation: Operation, error: &io::Error) -> Option<ErrorHint> {
    let raw_code = error.raw_os_error()?;
    let mappings = match os {
        "windows" => windows::MAPPINGS,
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

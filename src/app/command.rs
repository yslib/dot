use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub config: PathBuf,
    pub target: Option<String>,
    pub profile: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Apply { dry_run: bool },
    CheckProviders,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dispatch {
    pub selection: Selection,
    pub operation: Operation,
}

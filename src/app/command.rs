use std::path::PathBuf;

use crate::job::JobSelection;
use crate::platform::PlatformInfo;
use crate::schema::{SelectorIdentifier, SelectorIdentifierError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub config: PathBuf,
    pub target: Option<String>,
    pub profile: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileSelection {
    Root,
    Named(SelectorIdentifier),
}

impl ProfileSelection {
    pub fn from_cli(value: Option<&str>) -> Result<Self, SelectorIdentifierError> {
        match value {
            None | Some("@root") => Ok(Self::Root),
            Some(value) => SelectorIdentifier::new(value).map(Self::Named),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopeSelection {
    pub target: Option<SelectorIdentifier>,
    pub profile: ProfileSelection,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionRequest {
    pub scope: ScopeSelection,
    pub jobs: JobSelection,
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
    pub platform_override: Option<PlatformInfo>,
}

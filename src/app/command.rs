use std::fmt;
use std::path::PathBuf;

use crate::job::JobSelection;
use crate::platform::PlatformInfo;
use crate::schema::{SelectorIdentifier, SelectorIdentifierError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileSelection {
    Root,
    Named(SelectorIdentifier),
}

impl fmt::Display for ProfileSelection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Root => formatter.write_str("@root"),
            Self::Named(profile) => profile.fmt(formatter),
        }
    }
}

impl ProfileSelection {
    pub fn from_cli(value: Option<&str>) -> Result<Self, SelectorIdentifierError> {
        match value {
            None | Some("@root") => Ok(Self::Root),
            Some(value) => SelectorIdentifier::new(value).map(Self::Named),
        }
    }

    pub(crate) fn named(&self) -> Option<&SelectorIdentifier> {
        match self {
            Self::Root => None,
            Self::Named(profile) => Some(profile),
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operation {
    Apply(ExecutionRequest),
    DryRun(ExecutionRequest),
    CheckProviders(ScopeSelection),
    ListTargets { all: bool },
    ListProfiles { target: Option<SelectorIdentifier> },
    ListJobs(ScopeSelection),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dispatch {
    pub config: PathBuf,
    pub operation: Operation,
    pub platform_override: Option<PlatformInfo>,
}

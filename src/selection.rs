//! Target, profile, and execution selections shared by inspection and native operations.

use std::fmt;

use crate::job::JobSelection;
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
    pub fn parse(value: Option<&str>) -> Result<Self, SelectorIdentifierError> {
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
pub struct ExecutionSelection {
    pub scope: ScopeSelection,
    pub jobs: JobSelection,
}

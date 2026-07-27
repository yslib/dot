use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::str::FromStr;

use crate::schema::{Identifier, SelectorIdentifier, SelectorIdentifierError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobKind {
    Provider,
    Package,
    Action,
    Link,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum JobId {
    Provider(Identifier),
    Package(SelectorIdentifier),
    Action(SelectorIdentifier),
    Link(SelectorIdentifier),
}

impl JobId {
    pub const fn kind(&self) -> JobKind {
        match self {
            Self::Provider(_) => JobKind::Provider,
            Self::Package(_) => JobKind::Package,
            Self::Action(_) => JobKind::Action,
            Self::Link(_) => JobKind::Link,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Provider(identifier) => identifier.as_str(),
            Self::Package(identifier) | Self::Action(identifier) | Self::Link(identifier) => {
                identifier.as_str()
            }
        }
    }
}

impl fmt::Display for JobId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Provider(identifier) => write!(formatter, "provider:{identifier}"),
            Self::Package(identifier) => write!(formatter, "package:{identifier}"),
            Self::Action(identifier) => write!(formatter, "action:{identifier}"),
            Self::Link(identifier) => write!(formatter, "link:{identifier}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum JobSelector {
    Package(SelectorIdentifier),
    Action(SelectorIdentifier),
    Link(SelectorIdentifier),
}

impl fmt::Display for JobSelector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Package(identifier) => write!(formatter, "package:{identifier}"),
            Self::Action(identifier) => write!(formatter, "action:{identifier}"),
            Self::Link(identifier) => write!(formatter, "link:{identifier}"),
        }
    }
}

impl FromStr for JobSelector {
    type Err = JobSelectorParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let Some((kind, identifier)) = value.split_once(':') else {
            return Err(JobSelectorParseError::MissingKind);
        };
        if kind.is_empty() {
            return Err(JobSelectorParseError::MissingKind);
        }

        match kind {
            "package" => SelectorIdentifier::new(identifier)
                .map(Self::Package)
                .map_err(JobSelectorParseError::InvalidIdentifier),
            "action" => SelectorIdentifier::new(identifier)
                .map(Self::Action)
                .map_err(JobSelectorParseError::InvalidIdentifier),
            "link" => SelectorIdentifier::new(identifier)
                .map(Self::Link)
                .map_err(JobSelectorParseError::InvalidIdentifier),
            "provider" => Err(JobSelectorParseError::ProviderNotSelectable),
            unknown => Err(JobSelectorParseError::UnknownKind(unknown.to_owned())),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobSelectorParseError {
    MissingKind,
    InvalidIdentifier(SelectorIdentifierError),
    UnknownKind(String),
    ProviderNotSelectable,
}

impl fmt::Display for JobSelectorParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingKind => {
                formatter.write_str("a job selector must have a kind followed by ':'")
            }
            Self::InvalidIdentifier(error) => {
                write!(formatter, "invalid job selector identifier: {error}")
            }
            Self::UnknownKind(kind) => write!(formatter, "unknown job selector kind '{kind}'"),
            Self::ProviderNotSelectable => {
                formatter.write_str("provider jobs cannot be selected directly")
            }
        }
    }
}

impl Error for JobSelectorParseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidIdentifier(error) => Some(error),
            Self::MissingKind | Self::UnknownKind(_) | Self::ProviderNotSelectable => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobSelection {
    All,
    Only(BTreeSet<JobSelector>),
}

impl JobSelection {
    pub fn only(selector: JobSelector) -> Self {
        Self::Only(BTreeSet::from([selector]))
    }
}

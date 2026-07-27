use std::collections::BTreeSet;

use crate::schema::{Identifier, SelectorIdentifier};

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

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum JobSelector {
    Package(SelectorIdentifier),
    Action(SelectorIdentifier),
    Link(SelectorIdentifier),
}

impl JobSelector {
    pub(crate) fn job_id(&self) -> JobId {
        match self {
            Self::Package(id) => JobId::Package(id.clone()),
            Self::Action(id) => JobId::Action(id.clone()),
            Self::Link(id) => JobId::Link(id.clone()),
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

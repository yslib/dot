//! Effective job catalog inspection.

use std::borrow::Cow;

use crate::job::JobSelector;
use crate::manifest::{EffectiveManifest, ManifestError, ManifestJobRef};
use crate::output::TsvRecord;
use crate::platform::PlatformInfo;
use crate::schema::{Action, Config, Link, Package, ProviderPackage};

use crate::selection::ScopeSelection;

pub(super) struct Catalog {
    manifest: EffectiveManifest,
}

impl Catalog {
    pub(super) fn new(
        config: &Config,
        platform: &PlatformInfo,
        scope: &ScopeSelection,
    ) -> Result<Self, ManifestError> {
        let manifest = EffectiveManifest::select_for_inspection(
            config,
            platform,
            scope.target.as_ref(),
            scope.profile.named(),
        )?;
        Ok(Self { manifest })
    }

    pub(super) fn records(&self) -> Vec<JobRecord> {
        self.manifest
            .unresolved_jobs()
            .map(|job| JobRecord {
                selector: match job {
                    ManifestJobRef::Package(id, _) => JobSelector::Package(id.clone()),
                    ManifestJobRef::Action(id, _) => JobSelector::Action(id.clone()),
                    ManifestJobRef::Link(id, _) => JobSelector::Link(id.clone()),
                },
                job: match job {
                    ManifestJobRef::Package(_, package) => InspectedJob::Package(package.clone()),
                    ManifestJobRef::Action(_, action) => {
                        InspectedJob::Action(Box::new(action.clone()))
                    }
                    ManifestJobRef::Link(_, link) => InspectedJob::Link(link.clone()),
                },
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobRecord {
    selector: JobSelector,
    job: InspectedJob,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InspectedJob {
    Package(Package),
    Action(Box<Action>),
    Link(Link),
}

impl JobRecord {
    pub const fn selector(&self) -> &JobSelector {
        &self.selector
    }

    pub const fn job(&self) -> &InspectedJob {
        &self.job
    }
}

impl TsvRecord for JobRecord {
    fn fields(&self) -> Vec<Cow<'_, str>> {
        let (kind, id) = match &self.selector {
            JobSelector::Package(id) => ("package", id.as_str()),
            JobSelector::Action(id) => ("action", id.as_str()),
            JobSelector::Link(id) => ("link", id.as_str()),
        };
        let (via, detail) = match &self.job {
            InspectedJob::Package(Package::Provider(package)) => (
                Cow::Borrowed(package.provider().as_str()),
                match package {
                    ProviderPackage::Single(_) => Cow::Borrowed(id),
                    ProviderPackage::Batch(package) => Cow::Owned(
                        package
                            .names
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(","),
                    ),
                },
            ),
            InspectedJob::Package(Package::Manual(package)) => (
                Cow::Borrowed("manual"),
                Cow::Borrowed(package.install.exec.program.source_spelling()),
            ),
            InspectedJob::Action(action) => match action.as_ref() {
                Action::Command(action) => (
                    Cow::Borrowed("exec"),
                    Cow::Borrowed(action.exec.program.source_spelling()),
                ),
                Action::FetchContent(action) => (
                    Cow::Borrowed("fetch"),
                    Cow::Owned(format!(
                        "{} -> {}",
                        action.source.source_spelling(),
                        action.target.source_spelling()
                    )),
                ),
            },
            InspectedJob::Link(link) => (
                Cow::Borrowed("builtin"),
                Cow::Owned(format!(
                    "{} -> {}",
                    link.source.source_spelling(),
                    link.target.source_spelling()
                )),
            ),
        };

        vec![
            Cow::Owned(self.selector.to_string()),
            Cow::Borrowed(kind),
            Cow::Borrowed(id),
            via,
            detail,
        ]
    }
}

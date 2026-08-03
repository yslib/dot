use std::borrow::Cow;

use crate::config::LoadedConfigDocument;
use crate::job::JobSelector;
use crate::manifest::{EffectiveManifest, ManifestJobRef};
use crate::output::TsvRecord;
use crate::platform::PlatformInfo;
use crate::schema::{Action, Package, ProviderPackage};

use super::{ManifestCommandError, ScopeSelection};

pub(super) struct Catalog {
    manifest: EffectiveManifest,
}

impl Catalog {
    pub(super) fn load(
        config: &std::path::Path,
        platform: &PlatformInfo,
        scope: &ScopeSelection,
    ) -> Result<Self, ManifestCommandError> {
        let loaded = LoadedConfigDocument::load(config)?;
        let manifest = EffectiveManifest::select_for_inspection(
            loaded.config(),
            platform,
            scope.target.as_ref(),
            scope.profile.named(),
        )?;
        Ok(Self { manifest })
    }

    pub(super) fn records(&self) -> Vec<JobRecord<'_>> {
        self.manifest
            .unresolved_jobs()
            .map(|job| JobRecord {
                selector: match job {
                    ManifestJobRef::Package(id, _) => JobSelector::Package(id.clone()),
                    ManifestJobRef::Action(id, _) => JobSelector::Action(id.clone()),
                    ManifestJobRef::Link(id, _) => JobSelector::Link(id.clone()),
                },
                job,
            })
            .collect()
    }
}

pub(super) struct JobRecord<'a> {
    selector: JobSelector,
    job: ManifestJobRef<'a>,
}

impl TsvRecord for JobRecord<'_> {
    fn fields(&self) -> Vec<Cow<'_, str>> {
        let (kind, id) = match &self.selector {
            JobSelector::Package(id) => ("package", id.as_str()),
            JobSelector::Action(id) => ("action", id.as_str()),
            JobSelector::Link(id) => ("link", id.as_str()),
        };
        let (via, detail) = match self.job {
            ManifestJobRef::Package(_, Package::Provider(package)) => (
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
            ManifestJobRef::Package(_, Package::Manual(package)) => (
                Cow::Borrowed("manual"),
                Cow::Borrowed(package.install.exec.program.source_spelling()),
            ),
            ManifestJobRef::Action(_, Action::Command(action)) => (
                Cow::Borrowed("exec"),
                Cow::Borrowed(action.exec.program.source_spelling()),
            ),
            ManifestJobRef::Action(_, Action::FetchContent(action)) => (
                Cow::Borrowed("fetch"),
                Cow::Owned(format!(
                    "{} -> {}",
                    action.source.source_spelling(),
                    action.target.source_spelling()
                )),
            ),
            ManifestJobRef::Link(_, link) => (
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_preserve_typed_job_facts_until_rendering() {
        fn assert_types<'a>(record: JobRecord<'a>) {
            let _: JobSelector = record.selector;
            let _: ManifestJobRef<'a> = record.job;
        }

        let _ = assert_types;
    }
}

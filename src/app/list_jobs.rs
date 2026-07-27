use std::borrow::Cow;

use crate::config::LoadedConfig;
use crate::manifest::{EffectiveManifest, ManifestJobRef};
use crate::output::TsvRecord;
use crate::platform::PlatformInfo;
use crate::schema::{Package, ProviderPackage};

use super::{ListCommandError, ScopeSelection};

pub(super) struct JobRecord {
    selector: String,
    kind: &'static str,
    id: String,
    via: String,
    detail: String,
}

impl TsvRecord for JobRecord {
    fn fields(&self) -> Vec<Cow<'_, str>> {
        vec![
            Cow::Borrowed(&self.selector),
            Cow::Borrowed(self.kind),
            Cow::Borrowed(&self.id),
            Cow::Borrowed(&self.via),
            Cow::Borrowed(&self.detail),
        ]
    }
}

pub(super) fn records(
    config: &std::path::Path,
    platform: &PlatformInfo,
    scope: &ScopeSelection,
) -> Result<Vec<JobRecord>, ListCommandError> {
    let loaded = LoadedConfig::load(config)?;
    let manifest = EffectiveManifest::select_for_inspection(
        loaded.config(),
        platform,
        scope.target.as_ref(),
        scope.profile.named(),
    )?;
    Ok(manifest.unresolved_jobs().map(job_record).collect())
}

fn job_record(job: ManifestJobRef<'_>) -> JobRecord {
    match job {
        ManifestJobRef::Package(id, Package::Provider(package)) => JobRecord {
            selector: format!("package:{id}"),
            kind: "package",
            id: id.to_string(),
            via: package.provider().to_string(),
            detail: match package {
                ProviderPackage::Single(_) => id.to_string(),
                ProviderPackage::Batch(package) => package
                    .names
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(","),
            },
        },
        ManifestJobRef::Package(id, Package::Manual(package)) => JobRecord {
            selector: format!("package:{id}"),
            kind: "package",
            id: id.to_string(),
            via: "manual".to_owned(),
            detail: package.install.exec.program.source_spelling().to_owned(),
        },
        ManifestJobRef::Action(id, action) => JobRecord {
            selector: format!("action:{id}"),
            kind: "action",
            id: id.to_string(),
            via: "exec".to_owned(),
            detail: action.exec.program.source_spelling().to_owned(),
        },
        ManifestJobRef::Link(id, link) => JobRecord {
            selector: format!("link:{id}"),
            kind: "link",
            id: id.to_string(),
            via: "builtin".to_owned(),
            detail: format!(
                "{} -> {}",
                link.source.source_spelling(),
                link.target.source_spelling()
            ),
        },
    }
}

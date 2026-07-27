use std::borrow::Cow;

use crate::config::LoadedConfig;
use crate::manifest::{EffectiveManifest, profile_entries};
use crate::output::TsvRecord;
use crate::platform::PlatformInfo;
use crate::schema::SelectorIdentifier;

use super::ListCommandError;

pub(super) struct ProfileRecord {
    profile: String,
    path: String,
    depth: String,
}

impl TsvRecord for ProfileRecord {
    fn fields(&self) -> Vec<Cow<'_, str>> {
        vec![
            Cow::Borrowed(&self.profile),
            Cow::Borrowed(&self.path),
            Cow::Borrowed(&self.depth),
        ]
    }
}

pub(super) fn records(
    config: &std::path::Path,
    platform: &PlatformInfo,
    requested_target: Option<&SelectorIdentifier>,
) -> Result<Vec<ProfileRecord>, ListCommandError> {
    let loaded = LoadedConfig::load(config)?;
    let manifest = EffectiveManifest::select_for_inspection(
        loaded.config(),
        platform,
        requested_target,
        None,
    )?;
    let (target_id, target) = loaded
        .config()
        .targets
        .get_key_value(manifest.target())
        .expect("the inspected target came from this configuration");
    let profiles = profile_entries(target_id, target)?;

    let mut records = vec![ProfileRecord {
        profile: "@root".to_owned(),
        path: "<root>".to_owned(),
        depth: "0".to_owned(),
    }];
    records.extend(profiles.map(|entry| {
        ProfileRecord {
            profile: entry.id.to_string(),
            path: entry
                .path
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("/"),
            depth: entry.depth().to_string(),
        }
    }));
    Ok(records)
}

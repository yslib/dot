use std::borrow::Cow;

use crate::config::LoadedConfig;
use crate::manifest::{EffectiveManifest, profile_entries};
use crate::output::TsvRecord;
use crate::platform::PlatformInfo;
use crate::schema::SelectorIdentifier;

use super::{ListCommandError, ProfileSelection};

pub(super) struct Catalog {
    loaded: LoadedConfig,
    target: SelectorIdentifier,
}

impl Catalog {
    pub(super) fn load(
        config: &std::path::Path,
        platform: &PlatformInfo,
        requested_target: Option<&SelectorIdentifier>,
    ) -> Result<Self, ListCommandError> {
        let loaded = LoadedConfig::load(config)?;
        let selected = EffectiveManifest::select_for_inspection(
            loaded.config(),
            platform,
            requested_target,
            None,
        )?;
        let target = loaded
            .config()
            .targets
            .get_key_value(selected.target())
            .expect("the inspected target came from this configuration")
            .0
            .clone();

        Ok(Self { loaded, target })
    }

    pub(super) fn records(&self) -> Vec<ProfileRecord<'_>> {
        let target = self
            .loaded
            .config()
            .targets
            .get(self.target.as_str())
            .expect("the catalog owns the selected target");
        let profiles = profile_entries(&self.target, target)
            .expect("catalog loading validated the complete profile tree");

        let mut records = vec![ProfileRecord {
            profile: ProfileSelection::Root,
            path: Vec::new(),
        }];
        records.extend(profiles.map(|entry| ProfileRecord {
            profile: ProfileSelection::Named(entry.id.clone()),
            path: entry.path,
        }));
        records
    }
}

pub(super) struct ProfileRecord<'a> {
    profile: ProfileSelection,
    path: Vec<&'a SelectorIdentifier>,
}

impl TsvRecord for ProfileRecord<'_> {
    fn fields(&self) -> Vec<Cow<'_, str>> {
        let (profile, path) = match &self.profile {
            ProfileSelection::Root => (Cow::Borrowed("@root"), Cow::Borrowed("<root>")),
            ProfileSelection::Named(profile) => (
                Cow::Borrowed(profile.as_str()),
                Cow::Owned(
                    self.path
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("/"),
                ),
            ),
        };

        vec![profile, path, Cow::Owned(self.path.len().to_string())]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_preserve_typed_profile_facts_until_rendering() {
        fn assert_types<'a>(record: ProfileRecord<'a>) {
            let _: ProfileSelection = record.profile;
            let _: Vec<&'a SelectorIdentifier> = record.path;
        }

        let _ = assert_types;
    }
}

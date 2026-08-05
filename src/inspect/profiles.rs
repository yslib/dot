//! Declared profile catalog inspection.

use std::borrow::Cow;

use crate::manifest::{EffectiveManifest, ManifestError, profile_entries};
use crate::output::TsvRecord;
use crate::platform::PlatformInfo;
use crate::schema::{Config, SelectorIdentifier};

use crate::selection::ProfileSelection;

pub(super) struct Catalog<'a> {
    config: &'a Config,
    target: SelectorIdentifier,
}

impl<'a> Catalog<'a> {
    pub(super) fn new(
        config: &'a Config,
        platform: &PlatformInfo,
        requested_target: Option<&SelectorIdentifier>,
    ) -> Result<Self, ManifestError> {
        let selected =
            EffectiveManifest::select_for_inspection(config, platform, requested_target, None)?;
        let target = config
            .targets
            .get_key_value(selected.target())
            .expect("the inspected target came from this configuration")
            .0
            .clone();

        Ok(Self { config, target })
    }

    pub(super) fn records(&self) -> Vec<ProfileRecord> {
        let target = self
            .config
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
            path: entry.path.into_iter().cloned().collect(),
        }));
        records
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileRecord {
    profile: ProfileSelection,
    path: Vec<SelectorIdentifier>,
}

impl ProfileRecord {
    pub const fn profile(&self) -> &ProfileSelection {
        &self.profile
    }

    pub fn path(&self) -> &[SelectorIdentifier] {
        &self.path
    }
}

impl TsvRecord for ProfileRecord {
    fn fields(&self) -> Vec<Cow<'_, str>> {
        let (profile, path) = match &self.profile {
            ProfileSelection::Root => (
                Cow::Owned(self.profile.to_string()),
                Cow::Borrowed("<root>"),
            ),
            ProfileSelection::Named(_) => (
                Cow::Owned(self.profile.to_string()),
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

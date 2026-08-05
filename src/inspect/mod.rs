//! Typed inspection of declared targets, profiles, and jobs.

mod jobs;
mod profiles;
mod targets;

pub use jobs::{InspectedJob, JobRecord};
pub use profiles::ProfileRecord;
pub use targets::TargetRecord;

use crate::manifest::ManifestError;
use crate::platform::PlatformInfo;
use crate::schema::{Config, SelectorIdentifier};
use crate::selection::ScopeSelection;

pub struct Inspector<'a> {
    config: &'a Config,
    platform: &'a PlatformInfo,
}

impl<'a> Inspector<'a> {
    pub const fn new(config: &'a Config, platform: &'a PlatformInfo) -> Self {
        Self { config, platform }
    }

    pub fn targets(&self, all: bool) -> Vec<TargetRecord> {
        targets::Catalog::new(self.config).records(self.platform, all)
    }

    pub fn profiles(
        &self,
        target: Option<&SelectorIdentifier>,
    ) -> Result<Vec<ProfileRecord>, InspectError> {
        let catalog = profiles::Catalog::new(self.config, self.platform, target)?;
        Ok(catalog.records())
    }

    pub fn jobs(&self, scope: &ScopeSelection) -> Result<Vec<JobRecord>, InspectError> {
        let catalog = jobs::Catalog::new(self.config, self.platform, scope)?;
        Ok(catalog.records())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum InspectError {
    #[error(transparent)]
    Manifest(#[from] ManifestError),
}

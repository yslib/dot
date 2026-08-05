//! Declared target catalog inspection.

use std::borrow::Cow;

use crate::manifest::target_entries;
use crate::output::TsvRecord;
use crate::platform::PlatformInfo;
use crate::schema::{Config, Identifier, OneOrMany, PlatformConstraint, SelectorIdentifier};

pub(super) struct Catalog<'a> {
    config: &'a Config,
}

impl<'a> Catalog<'a> {
    pub(super) const fn new(config: &'a Config) -> Self {
        Self { config }
    }

    pub(super) fn records(&self, platform: &PlatformInfo, all: bool) -> Vec<TargetRecord> {
        target_entries(self.config, platform)
            .filter(|entry| all || entry.compatible)
            .map(|entry| TargetRecord {
                target: entry.id.clone(),
                compatible: entry.compatible,
                constraint: entry.target.platform.clone(),
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetRecord {
    target: SelectorIdentifier,
    compatible: bool,
    constraint: PlatformConstraint,
}

impl TargetRecord {
    pub const fn target(&self) -> &SelectorIdentifier {
        &self.target
    }

    pub const fn compatible(&self) -> bool {
        self.compatible
    }

    pub const fn constraint(&self) -> &PlatformConstraint {
        &self.constraint
    }
}

impl TsvRecord for TargetRecord {
    fn fields(&self) -> Vec<Cow<'_, str>> {
        vec![
            Cow::Borrowed(self.target.as_str()),
            Cow::Borrowed(if self.compatible {
                "compatible"
            } else {
                "incompatible"
            }),
            Cow::Owned(join(&self.constraint.os)),
            Cow::Owned(join_optional(self.constraint.arch.as_ref())),
            Cow::Owned(join_optional(self.constraint.distro.as_ref())),
            Cow::Owned(join_optional(self.constraint.distro_family.as_ref())),
            Cow::Owned(join_optional(self.constraint.environment.as_ref())),
        ]
    }
}

fn join_optional(values: Option<&OneOrMany<Identifier>>) -> String {
    values.map_or_else(String::new, join)
}

fn join(values: &OneOrMany<Identifier>) -> String {
    match values {
        OneOrMany::One(value) => value.to_string(),
        OneOrMany::Many(values) => values
            .iter()
            .map(Identifier::as_str)
            .collect::<Vec<_>>()
            .join(","),
    }
}

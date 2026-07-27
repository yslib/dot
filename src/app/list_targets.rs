use std::borrow::Cow;

use crate::config::LoadedConfig;
use crate::manifest::target_entries;
use crate::output::TsvRecord;
use crate::platform::PlatformInfo;
use crate::schema::{Identifier, OneOrMany, PlatformConstraint, SelectorIdentifier};

use super::ListCommandError;

pub(super) struct Catalog {
    loaded: LoadedConfig,
}

impl Catalog {
    pub(super) fn load(config: &std::path::Path) -> Result<Self, ListCommandError> {
        Ok(Self {
            loaded: LoadedConfig::load(config)?,
        })
    }

    pub(super) fn records<'a>(
        &'a self,
        platform: &PlatformInfo,
        all: bool,
    ) -> Vec<TargetRecord<'a>> {
        target_entries(self.loaded.config(), platform)
            .filter(|entry| all || entry.compatible)
            .map(|entry| TargetRecord {
                target: entry.id,
                compatibility: Compatibility::from(entry.compatible),
                constraint: &entry.target.platform,
            })
            .collect()
    }
}

#[derive(Clone, Copy)]
enum Compatibility {
    Compatible,
    Incompatible,
}

impl Compatibility {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Compatible => "compatible",
            Self::Incompatible => "incompatible",
        }
    }
}

impl From<bool> for Compatibility {
    fn from(compatible: bool) -> Self {
        if compatible {
            Self::Compatible
        } else {
            Self::Incompatible
        }
    }
}

pub(super) struct TargetRecord<'a> {
    target: &'a SelectorIdentifier,
    compatibility: Compatibility,
    constraint: &'a PlatformConstraint,
}

impl TsvRecord for TargetRecord<'_> {
    fn fields(&self) -> Vec<Cow<'_, str>> {
        vec![
            Cow::Borrowed(self.target.as_str()),
            Cow::Borrowed(self.compatibility.as_str()),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_preserve_typed_target_facts_until_rendering() {
        fn assert_types<'a>(record: TargetRecord<'a>) {
            let _: &'a SelectorIdentifier = record.target;
            let _: Compatibility = record.compatibility;
            let _: &'a PlatformConstraint = record.constraint;
        }

        let _ = assert_types;
    }
}

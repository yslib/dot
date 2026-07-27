use std::borrow::Cow;

use crate::config::LoadedConfig;
use crate::manifest::target_entries;
use crate::output::TsvRecord;
use crate::platform::PlatformInfo;
use crate::schema::{Identifier, OneOrMany, PlatformConstraint};

use super::ListCommandError;

pub(super) struct TargetRecord {
    target: String,
    compatibility: &'static str,
    os: String,
    arch: String,
    distro: String,
    distro_family: String,
    environment: String,
}

impl TsvRecord for TargetRecord {
    fn fields(&self) -> Vec<Cow<'_, str>> {
        vec![
            Cow::Borrowed(&self.target),
            Cow::Borrowed(self.compatibility),
            Cow::Borrowed(&self.os),
            Cow::Borrowed(&self.arch),
            Cow::Borrowed(&self.distro),
            Cow::Borrowed(&self.distro_family),
            Cow::Borrowed(&self.environment),
        ]
    }
}

pub(super) fn records(
    config: &std::path::Path,
    platform: &PlatformInfo,
    all: bool,
) -> Result<Vec<TargetRecord>, ListCommandError> {
    let loaded = LoadedConfig::load(config)?;
    Ok(target_entries(loaded.config(), platform)
        .filter(|entry| all || entry.compatible)
        .map(|entry| from_constraint(entry.id.as_str(), entry.compatible, &entry.target.platform))
        .collect())
}

fn from_constraint(
    target: &str,
    compatible: bool,
    constraint: &PlatformConstraint,
) -> TargetRecord {
    TargetRecord {
        target: target.to_owned(),
        compatibility: if compatible {
            "compatible"
        } else {
            "incompatible"
        },
        os: join(&constraint.os),
        arch: join_optional(constraint.arch.as_ref()),
        distro: join_optional(constraint.distro.as_ref()),
        distro_family: join_optional(constraint.distro_family.as_ref()),
        environment: join_optional(constraint.environment.as_ref()),
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

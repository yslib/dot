use std::collections::BTreeSet;

use dot_core::platform::PlatformInfo;
use dot_core::schema::{Identifier, OneOrMany};
use serde::Deserialize;

pub fn parse(input: &str) -> Result<PlatformInfo, String> {
    let document = format!("platform = {input}");
    let parsed: InjectedPlatformDocument = toml::from_str(&document)
        .map_err(|error| format!("invalid platform inline table: {error}"))?;
    Ok(parsed.platform.into())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InjectedPlatformDocument {
    platform: InjectedPlatform,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InjectedPlatform {
    os: Identifier,
    arch: Identifier,
    distro: Option<Identifier>,
    distro_family: Option<OneOrMany<Identifier>>,
    environment: Option<OneOrMany<Identifier>>,
}

impl From<InjectedPlatform> for PlatformInfo {
    fn from(injected: InjectedPlatform) -> Self {
        Self {
            os: injected.os.to_string(),
            arch: injected.arch.to_string(),
            distro: injected.distro.map(|value| value.to_string()),
            distro_families: identifiers(injected.distro_family),
            environments: injected.environment.map_or_else(
                || BTreeSet::from(["native".to_owned()]),
                |values| identifiers(Some(values)),
            ),
        }
    }
}

fn identifiers(values: Option<OneOrMany<Identifier>>) -> BTreeSet<String> {
    match values {
        None => BTreeSet::new(),
        Some(OneOrMany::One(value)) => BTreeSet::from([value.to_string()]),
        Some(OneOrMany::Many(values)) => {
            values.into_iter().map(|value| value.to_string()).collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_inline_platform_facts_and_defaults_the_environment() {
        let platform = parse(
            r#"{ os = "linux", arch = "x86_64", distro = "ubuntu", distro_family = ["debian", "linux"] }"#,
        )
        .expect("platform override should parse");

        assert_eq!(platform.os, "linux");
        assert_eq!(platform.arch, "x86_64");
        assert_eq!(platform.distro.as_deref(), Some("ubuntu"));
        assert_eq!(
            platform.distro_families,
            BTreeSet::from(["debian".to_owned(), "linux".to_owned()])
        );
        assert_eq!(platform.environments, BTreeSet::from(["native".to_owned()]));
    }

    #[test]
    fn rejects_unknown_platform_fields() {
        let error = parse(r#"{ os = "linux", arch = "x86_64", unknown = "value" }"#)
            .expect_err("unknown facts should be rejected");

        assert!(error.starts_with("invalid platform inline table:"));
        assert!(error.contains("unknown field `unknown`"));
    }
}

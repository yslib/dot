use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::Path;

#[cfg(feature = "dev-platform-override")]
use serde::Deserialize;

use crate::schema::{Identifier, OneOrMany, PlatformConstraint};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlatformInfo {
    pub os: String,
    pub arch: String,
    pub distro: Option<String>,
    pub distro_families: BTreeSet<String>,
    pub environments: BTreeSet<String>,
}

impl PlatformInfo {
    pub fn detect() -> Self {
        let os = env::consts::OS.to_owned();
        let distribution = if os == "linux" {
            detect_linux_distribution()
        } else {
            LinuxDistribution::default()
        };
        let is_wsl = os == "linux" && detect_wsl();
        let is_container = os == "linux" && detect_container();

        Self {
            os,
            arch: env::consts::ARCH.to_owned(),
            distro: distribution.id,
            distro_families: distribution.id_like,
            environments: classify_environments(is_wsl, is_container),
        }
    }
}

#[cfg(feature = "dev-platform-override")]
pub fn parse_override(input: &str) -> Result<PlatformInfo, String> {
    let document = format!("platform = {input}");
    let parsed: InjectedPlatformDocument = toml::from_str(&document)
        .map_err(|error| format!("invalid platform inline table: {error}"))?;
    Ok(parsed.platform.into())
}

#[cfg(feature = "dev-platform-override")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InjectedPlatformDocument {
    platform: InjectedPlatform,
}

#[cfg(feature = "dev-platform-override")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InjectedPlatform {
    os: Identifier,
    arch: Identifier,
    distro: Option<Identifier>,
    distro_family: Option<OneOrMany<Identifier>>,
    environment: Option<OneOrMany<Identifier>>,
}

#[cfg(feature = "dev-platform-override")]
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

#[cfg(feature = "dev-platform-override")]
fn identifiers(values: Option<OneOrMany<Identifier>>) -> BTreeSet<String> {
    match values {
        None => BTreeSet::new(),
        Some(OneOrMany::One(value)) => BTreeSet::from([value.to_string()]),
        Some(OneOrMany::Many(values)) => {
            values.into_iter().map(|value| value.to_string()).collect()
        }
    }
}

impl PlatformConstraint {
    pub fn matches(&self, actual: &PlatformInfo) -> bool {
        allowed(&self.os, &actual.os)
            && optional_scalar_matches(self.arch.as_ref(), Some(actual.arch.as_str()))
            && optional_scalar_matches(self.distro.as_ref(), actual.distro.as_deref())
            && optional_set_matches(self.distro_family.as_ref(), &actual.distro_families)
            && optional_set_matches(self.environment.as_ref(), &actual.environments)
    }
}

fn allowed(expected: &OneOrMany<Identifier>, actual: &str) -> bool {
    match expected {
        OneOrMany::One(value) => value.as_str() == actual,
        OneOrMany::Many(values) => values.iter().any(|value| value.as_str() == actual),
    }
}

fn optional_scalar_matches(expected: Option<&OneOrMany<Identifier>>, actual: Option<&str>) -> bool {
    match expected {
        None => true,
        Some(expected) => actual.is_some_and(|actual| allowed(expected, actual)),
    }
}

fn optional_set_matches(
    expected: Option<&OneOrMany<Identifier>>,
    actual: &BTreeSet<String>,
) -> bool {
    match expected {
        None => true,
        Some(OneOrMany::One(value)) => actual.contains(value.as_str()),
        Some(OneOrMany::Many(values)) => values.iter().any(|value| actual.contains(value.as_str())),
    }
}

#[derive(Default)]
struct LinuxDistribution {
    id: Option<String>,
    id_like: BTreeSet<String>,
}

fn detect_linux_distribution() -> LinuxDistribution {
    ["/etc/os-release", "/usr/lib/os-release"]
        .iter()
        .find_map(|path| fs::read_to_string(path).ok())
        .map_or_else(LinuxDistribution::default, |input| parse_os_release(&input))
}

fn parse_os_release(input: &str) -> LinuxDistribution {
    let mut distribution = LinuxDistribution::default();

    for line in input.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = parse_os_release_value(value);

        match key {
            "ID" => distribution.id = (!value.is_empty()).then_some(value),
            "ID_LIKE" => distribution.id_like.extend(
                value
                    .split_whitespace()
                    .filter(|family| !family.is_empty())
                    .map(str::to_owned),
            ),
            _ => {}
        }
    }

    distribution
}

fn parse_os_release_value(value: &str) -> String {
    let value = value.trim();
    let unquoted = if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        &value[1..value.len() - 1]
    } else {
        value
    };

    let mut result = String::with_capacity(unquoted.len());
    let mut chars = unquoted.chars();
    while let Some(character) = chars.next() {
        if character == '\\' {
            if let Some(escaped) = chars.next() {
                result.push(escaped);
            }
        } else {
            result.push(character);
        }
    }
    result
}

fn detect_wsl() -> bool {
    env::var_os("WSL_INTEROP").is_some()
        || env::var_os("WSL_DISTRO_NAME").is_some()
        || fs::read_to_string("/proc/sys/kernel/osrelease")
            .is_ok_and(|release| release.to_ascii_lowercase().contains("microsoft"))
}

fn detect_container() -> bool {
    env::var_os("container").is_some_and(|value| !value.is_empty())
        || Path::new("/.dockerenv").exists()
        || Path::new("/run/.containerenv").exists()
}

fn classify_environments(is_wsl: bool, is_container: bool) -> BTreeSet<String> {
    let mut environments = BTreeSet::new();

    if is_wsl {
        environments.insert("wsl".to_owned());
    }
    if is_container {
        environments.insert("container".to_owned());
    }
    if environments.is_empty() {
        environments.insert("native".to_owned());
    }

    environments
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::schema::{Identifier, OneOrMany, PlatformConstraint};

    fn strings(values: &[&str]) -> BTreeSet<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    fn identifier(value: &str) -> Identifier {
        Identifier::new(value).expect("test identifier should be valid")
    }

    fn linux_platform() -> PlatformInfo {
        PlatformInfo {
            os: "linux".into(),
            arch: "x86_64".into(),
            distro: Some("ubuntu".into()),
            distro_families: strings(&["debian"]),
            environments: strings(&["container", "wsl"]),
        }
    }

    #[test]
    fn matches_allowed_values_across_all_constrained_fields() {
        let constraint = PlatformConstraint {
            os: OneOrMany::Many(vec![identifier("linux"), identifier("macos")]),
            arch: Some(OneOrMany::One(identifier("x86_64"))),
            distro: Some(OneOrMany::Many(vec![
                identifier("fedora"),
                identifier("ubuntu"),
            ])),
            distro_family: Some(OneOrMany::One(identifier("debian"))),
            environment: Some(OneOrMany::Many(vec![
                identifier("native"),
                identifier("wsl"),
            ])),
        };

        assert!(constraint.matches(&linux_platform()));
    }

    #[test]
    fn ignores_optional_constraints_that_are_not_declared() {
        let constraint = PlatformConstraint {
            os: OneOrMany::One(identifier("linux")),
            arch: None,
            distro: None,
            distro_family: None,
            environment: None,
        };

        assert!(constraint.matches(&linux_platform()));
    }

    #[test]
    fn rejects_a_mismatch_in_any_declared_field() {
        let constraint = PlatformConstraint {
            os: OneOrMany::One(identifier("linux")),
            arch: Some(OneOrMany::One(identifier("aarch64"))),
            distro: None,
            distro_family: None,
            environment: None,
        };

        assert!(!constraint.matches(&linux_platform()));
    }

    #[test]
    fn rejects_a_declared_optional_fact_when_detection_has_no_value() {
        let constraint = PlatformConstraint {
            os: OneOrMany::One(identifier("macos")),
            arch: None,
            distro: Some(OneOrMany::One(identifier("ubuntu"))),
            distro_family: None,
            environment: None,
        };
        let actual = PlatformInfo {
            os: "macos".into(),
            arch: "aarch64".into(),
            distro: None,
            distro_families: BTreeSet::new(),
            environments: strings(&["native"]),
        };

        assert!(!constraint.matches(&actual));
    }

    #[test]
    fn parses_linux_distribution_facts_from_os_release() {
        let release = r#"
            NAME="Ubuntu"
            ID=ubuntu
            ID_LIKE="debian linux"
        "#;

        let distribution = parse_os_release(release);

        assert_eq!(distribution.id.as_deref(), Some("ubuntu"));
        assert_eq!(distribution.id_like, strings(&["debian", "linux"]));
    }

    #[test]
    fn environment_facts_can_describe_wsl_inside_a_container() {
        assert_eq!(
            classify_environments(true, true),
            strings(&["container", "wsl"])
        );
        assert_eq!(classify_environments(false, false), strings(&["native"]));
    }

    #[test]
    fn detects_the_rust_runtime_target() {
        let actual = PlatformInfo::detect();

        assert_eq!(actual.os, std::env::consts::OS);
        assert_eq!(actual.arch, std::env::consts::ARCH);
        assert!(!actual.environments.is_empty());
    }
}

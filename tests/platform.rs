use std::collections::BTreeSet;

use dot_core::platform::PlatformInfo;
use dot_core::schema::{Identifier, OneOrMany, PlatformConstraint};

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
fn constraints_match_any_allowed_value_in_every_declared_field() {
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
fn undeclared_optional_constraints_do_not_reject_a_platform() {
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
fn any_declared_mismatch_rejects_a_platform() {
    let wrong_arch = PlatformConstraint {
        os: OneOrMany::One(identifier("linux")),
        arch: Some(OneOrMany::One(identifier("aarch64"))),
        distro: None,
        distro_family: None,
        environment: None,
    };
    let unavailable_distro = PlatformConstraint {
        os: OneOrMany::One(identifier("macos")),
        arch: None,
        distro: Some(OneOrMany::One(identifier("ubuntu"))),
        distro_family: None,
        environment: None,
    };
    let macos = PlatformInfo {
        os: "macos".into(),
        arch: "aarch64".into(),
        distro: None,
        distro_families: BTreeSet::new(),
        environments: strings(&["native"]),
    };

    assert!(!wrong_arch.matches(&linux_platform()));
    assert!(!unavailable_distro.matches(&macos));
}

#[test]
fn detected_platform_uses_the_rust_runtime_target() {
    let actual = PlatformInfo::detect();

    assert_eq!(actual.os, std::env::consts::OS);
    assert_eq!(actual.arch, std::env::consts::ARCH);
    assert!(!actual.environments.is_empty());
}

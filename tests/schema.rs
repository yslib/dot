use dot::schema::{Config, ExecActionType, LinkConflict, LinkMissingParent, OneOrMany, Package};

const REPOSITORY_DOT_TOML: &str = include_str!("fixtures/dot.toml");

#[test]
fn deserializes_the_repository_dotfile() {
    let config: Config =
        toml::from_str(REPOSITORY_DOT_TOML).expect("repository dot.toml should deserialize");

    assert_eq!(config.targets.len(), 6);
    assert_eq!(config.targets["macos"].providers.len(), 1);
    assert!(
        config.targets["arch-personal"].profiles["desktop"]
            .profiles
            .contains_key("laptop")
    );
}

#[test]
fn deserializes_the_complete_schema() {
    let input = r#"
        [targets.workstation]
        platform = { os = ["linux", "macos"], arch = "x86_64", distro = ["arch", "ubuntu"], distro_family = "unix", environment = ["native", "wsl"] }

        [targets.workstation.providers.brew]
        activate = { path_prepend = ["/opt/homebrew/bin", "/usr/local/bin"], path_append = "/custom/bin", variables = { HOMEBREW_NO_ANALYTICS = "1" } }
        probe = { program = "brew", args = ["--version"] }
        ensure = [
          { program = "bash", args = ["install-brew.sh"], cwd = "/tmp" },
          { type = "exec", program = "brew", args = ["tap", "example/tools"], env = { variables = { CI = "1" } } },
        ]
        install = { program = "brew", args = ["install", "${package:provider_args}", "${package:names}"] }

        [targets.workstation.packages]
        ripgrep = { provider = "brew" }
        app = { provider = "brew", provider_args = ["--cask"] }
        manual-tool = { install = { check = { program = "/opt/tools/manual-tool", args = ["--version"] }, exec = { program = "bash", args = ["install-manual-tool.sh"] } } }

        [targets.workstation.links]
        config = { source = "home/config", target = "${env:HOME}/.config/tool", on_conflict = "replace-link", on_missing_parent = "create" }

        [targets.workstation.actions.setup]
        check = { program = "test", args = ["-e", "/tmp/ready"] }
        exec = { type = "exec", program = "touch", args = ["/tmp/ready"] }

        [targets.workstation.profiles.desktop.packages]
        compositor = { provider = "brew" }

        [targets.workstation.profiles.desktop.profiles.laptop.links]
        power = { source = "home/power", target = "${env:HOME}/.config/power", on_conflict = "error", on_missing_parent = "skip" }
    "#;

    let config: Config = toml::from_str(input).expect("complete schema should deserialize");
    let target = &config.targets["workstation"];

    assert_eq!(
        target.platform.os,
        OneOrMany::Many(vec!["linux".into(), "macos".into()])
    );
    assert_eq!(target.platform.arch, Some(OneOrMany::One("x86_64".into())));

    let provider = &target.providers["brew"];
    let OneOrMany::Many(ensure) = provider.ensure.as_ref().expect("ensure is present") else {
        panic!("ensure should preserve its list form");
    };
    assert_eq!(ensure.len(), 2);
    assert_eq!(ensure[1].kind, Some(ExecActionType::Exec));

    let Package::Provider(app) = &target.packages["app"] else {
        panic!("app should be a provider package");
    };
    assert_eq!(app.provider, "brew");
    assert_eq!(
        app.provider_args.as_deref(),
        Some(&[String::from("--cask")][..])
    );
    assert!(matches!(target.packages["manual-tool"], Package::Manual(_)));

    let link = &target.links["config"];
    assert_eq!(link.on_conflict, Some(LinkConflict::ReplaceLink));
    assert_eq!(link.on_missing_parent, Some(LinkMissingParent::Create));

    let laptop = &target.profiles["desktop"].profiles["laptop"];
    let power = &laptop.links["power"];
    assert_eq!(power.on_conflict, Some(LinkConflict::Error));
    assert_eq!(power.on_missing_parent, Some(LinkMissingParent::Skip));
}

#[test]
fn rejects_unknown_fields() {
    let input = r#"
        [targets.server]
        platform = { os = "linux" }
        typo = "must not be ignored"
    "#;

    assert!(toml::from_str::<Config>(input).is_err());
}

#[test]
fn rejects_invalid_fixed_literals() {
    let input = r#"
        [targets.server]
        platform = { os = "linux" }

        [targets.server.links.config]
        source = "home/config"
        target = "/tmp/config"
        on_conflict = "overwrite-file"
    "#;

    assert!(toml::from_str::<Config>(input).is_err());
}

#[test]
fn rejects_a_package_with_both_provider_and_manual_install() {
    let input = r#"
        [targets.server]
        platform = { os = "linux" }

        [targets.server.packages]
        invalid = { provider = "brew", install = { exec = { program = "true" } } }
    "#;

    assert!(toml::from_str::<Config>(input).is_err());
}

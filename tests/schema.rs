mod support;

use dot::schema::{
    Config, EnvironmentName, ExecAction, ExecActionType, Identifier, LinkConflict,
    LinkMissingParent, LiteralString, OneOrMany, Package, ProviderInstallArg, ProviderPackage,
    ScalarTemplate,
};

use support::fixture;

#[test]
fn deserializes_the_repository_dotfile() {
    let input = fixture::read("dot.toml");
    let config: Config = toml::from_str(&input).expect("repository dot.toml should deserialize");

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
    let input = fixture::read("schema/valid-complete.toml");

    let config: Config = toml::from_str(&input).expect("complete schema should deserialize");
    let target = &config.targets["workstation"];

    let OneOrMany::Many(operating_systems) = &target.platform.os else {
        panic!("operating systems should preserve their list form");
    };
    assert_eq!(
        operating_systems
            .iter()
            .map(Identifier::as_str)
            .collect::<Vec<_>>(),
        vec!["linux", "macos"]
    );
    let Some(OneOrMany::One(architecture)) = &target.platform.arch else {
        panic!("architecture should preserve its scalar form");
    };
    assert_eq!(architecture.as_str(), "x86_64");

    let provider = &target.providers["brew"];
    let OneOrMany::Many(ensure) = provider.ensure.as_ref().expect("ensure is present") else {
        panic!("ensure should preserve its list form");
    };
    assert_eq!(ensure.len(), 2);
    assert_eq!(ensure[1].kind, Some(ExecActionType::Exec));

    let Package::Provider(ProviderPackage::Single(app)) = &target.packages["app"] else {
        panic!("app should be a provider package");
    };
    assert_eq!(app.provider.as_str(), "brew");
    assert_eq!(
        app.provider_args
            .as_ref()
            .expect("provider args exist")
            .iter()
            .map(LiteralString::as_str)
            .collect::<Vec<_>>(),
        vec!["--cask"]
    );
    let Package::Provider(ProviderPackage::Batch(cli_tools)) = &target.packages["cli-tools"] else {
        panic!("cli-tools should be a provider package batch");
    };
    assert_eq!(cli_tools.provider.as_str(), "brew");
    assert_eq!(
        cli_tools
            .names
            .iter()
            .map(Identifier::as_str)
            .collect::<Vec<_>>(),
        vec!["bat", "fd", "fzf"]
    );
    assert!(cli_tools.provider_args.is_none());
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
fn deserializes_strings_into_their_declared_schema_roles() {
    let input = fixture::read("schema/valid-string-roles.toml");
    let config: Config = toml::from_str(&input).expect("schema roles should deserialize");

    let (target_id, target) = config.targets.first_key_value().expect("target exists");
    let _: &Identifier = target_id;
    assert_eq!(target_id.as_str(), "machine");

    let provider = &target.providers["brew"];
    let _: &ScalarTemplate = &provider.probe.program;
    let _: &ExecAction<ProviderInstallArg> = &provider.install;
    let _: &ProviderInstallArg = &provider.install.args[1];
    assert_eq!(
        provider.install.args[1].as_str(),
        "${package:provider_args}"
    );

    let Package::Provider(ProviderPackage::Single(package)) = &target.packages["application"]
    else {
        panic!("application should use a provider");
    };
    let provider_arg: &LiteralString = &package.provider_args.as_ref().expect("args exist")[0];
    assert_eq!(provider_arg.as_str(), "--cask");

    let (name, value) = provider
        .activate
        .as_ref()
        .expect("activation exists")
        .variables
        .first_key_value()
        .expect("variable exists");
    let _: &EnvironmentName = name;
    let _: &ScalarTemplate = value;
    assert_eq!(name.as_str(), "HOMEBREW_PREFIX");
    assert_eq!(value.as_str(), "${env:HOME}/.homebrew");
}

#[test]
fn rejects_invalid_identifiers_while_deserializing() {
    let input = fixture::read("schema/invalid-identifier.toml");

    assert!(toml::from_str::<Config>(&input).is_err());
}

#[test]
fn rejects_invalid_environment_names_while_deserializing() {
    let input = fixture::read("schema/invalid-environment-name.toml");

    assert!(toml::from_str::<Config>(&input).is_err());
}

#[test]
fn rejects_unknown_fields() {
    let input = fixture::read("schema/invalid-unknown-field.toml");

    assert!(toml::from_str::<Config>(&input).is_err());
}

#[test]
fn rejects_invalid_fixed_literals() {
    let input = fixture::read("schema/invalid-fixed-literal.toml");

    assert!(toml::from_str::<Config>(&input).is_err());
}

#[test]
fn rejects_a_package_with_both_provider_and_manual_install() {
    let input = fixture::read("schema/invalid-mixed-package-install.toml");

    assert!(toml::from_str::<Config>(&input).is_err());
}

mod support;

use dot_core::schema::{
    Action, Config, EnvironmentName, ExecAction, ExpressionParseError, FetchContentConflict,
    Identifier, LinkConflict, LinkMissingParent, LiteralStringSource, OneOrMany, Package,
    ParsedStringForm, ParsedTemplatePart, ProviderInstallArgSource, ProviderPackage,
    SelectorIdentifier, StringExpressionSource,
};

use support::fixture;

#[test]
fn selector_identifiers_use_the_cli_safe_grammar() {
    for valid in ["a", "A1", "0root", "_root", "arch-personal", "tool.v2"] {
        assert_eq!(SelectorIdentifier::new(valid).unwrap().as_str(), valid);
    }

    for invalid in [
        "",
        "has space",
        "package:name",
        "profile/name",
        r"has\slash",
        "@root",
        ".root",
        "-root",
        "line\nbreak",
        "tab\tvalue",
        "非ascii",
        "a非",
    ] {
        assert!(
            SelectorIdentifier::new(invalid).is_err(),
            "{invalid:?} must be rejected"
        );
    }
}

#[test]
fn classifies_literal_template_variable_and_malformed_sources() {
    #[derive(serde::Deserialize)]
    struct Document {
        value: StringExpressionSource,
    }

    let cases = [
        ("", "literal"),
        ("plain", "literal"),
        ("prefix-${env:HOME}", "template"),
        ("${env:HOME}", "variable"),
        ("${env:HOME}${dot:cwd}", "template"),
        ("${env", "malformed"),
        ("${unknown:value}", "variable"),
    ];

    for (source, expected) in cases {
        let input = format!("value = {source:?}");
        let parsed = toml::from_str::<Document>(&input).unwrap().value;
        let actual = match parsed.parsed() {
            ParsedStringForm::Literal(_) => "literal",
            ParsedStringForm::Template(_) => "template",
            ParsedStringForm::Variable(_) => "variable",
            ParsedStringForm::Malformed(_) => "malformed",
        };
        assert_eq!(actual, expected, "source: {source}");
        assert_eq!(parsed.source_spelling(), source);
    }
}

#[test]
fn preserves_recoverable_source_syntax_details() {
    let empty = StringExpressionSource::from("");
    let ParsedStringForm::Literal(empty) = empty.parsed() else {
        panic!("an empty source is literal");
    };
    assert_eq!(empty.value(), "");

    let adjacent = StringExpressionSource::from("${env:HOME}${dot:cwd}");
    let ParsedStringForm::Template(template) = adjacent.parsed() else {
        panic!("adjacent resolver calls form a template");
    };
    assert_eq!(template.parts().len(), 2);
    let ParsedTemplatePart::Variable(first) = &template.parts()[0] else {
        panic!("first part should be a variable");
    };
    assert_eq!(first.resolver(), "env");
    assert_eq!(first.payload(), "HOME");
    let ParsedTemplatePart::Variable(second) = &template.parts()[1] else {
        panic!("second part should be a variable");
    };
    assert_eq!(second.resolver(), "dot");
    assert_eq!(second.payload(), "cwd");

    assert!(matches!(
        StringExpressionSource::from("${env").parsed(),
        ParsedStringForm::Malformed(ExpressionParseError::UnclosedResolver { offset: 0 })
    ));
    assert!(matches!(
        StringExpressionSource::from("${env}").parsed(),
        ParsedStringForm::Malformed(ExpressionParseError::MissingPayloadSeparator { offset: 0 })
    ));
    assert!(matches!(
        StringExpressionSource::from("${env:${dot:cwd}}").parsed(),
        ParsedStringForm::Malformed(ExpressionParseError::NestedResolver { offset: 6 })
    ));

    let unknown = StringExpressionSource::from("${unknown:value}");
    let ParsedStringForm::Variable(reference) = unknown.parsed() else {
        panic!("resolver lookup must not affect source classification");
    };
    assert_eq!(reference.resolver(), "unknown");
    assert_eq!(reference.payload(), "value");
}

#[test]
fn escaped_resolver_syntax_is_literal_source_text() {
    #[derive(serde::Deserialize)]
    struct Document {
        value: ProviderInstallArgSource,
    }

    let parsed = toml::from_str::<Document>(r#"value = 'prefix-\${package:names}'"#)
        .unwrap()
        .value;
    let ParsedStringForm::Literal(literal) = parsed.parsed() else {
        panic!("escaped syntax must not become a variable");
    };
    assert_eq!(literal.value(), "prefix-${package:names}");
    assert_eq!(parsed.source_spelling(), r"prefix-\${package:names}");

    let literal = LiteralStringSource::from(String::from(r"\${env:HOME}"));
    let ParsedStringForm::Literal(value) = literal.parsed() else {
        panic!("every source role must use the shared classifier");
    };
    assert_eq!(value.value(), "${env:HOME}");
    assert_eq!(literal.source_spelling(), r"\${env:HOME}");
}

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
fn rejects_legacy_exec_action_type() {
    let input = fixture::read("schema/invalid-legacy-exec-type.toml");
    let error = toml::from_str::<Config>(&input).expect_err("legacy type must be unknown");
    assert!(
        error.to_string().contains("unknown field `type`"),
        "{error}"
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

    let Package::Provider(ProviderPackage::Single(app)) = &target.packages["app"] else {
        panic!("app should be a provider package");
    };
    assert_eq!(app.provider.as_str(), "brew");
    assert_eq!(
        app.provider_args
            .as_ref()
            .expect("provider args exist")
            .iter()
            .map(LiteralStringSource::source_spelling)
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

    let Action::Command(setup) = &target.actions["setup"] else {
        panic!("setup should be a command action");
    };
    assert_eq!(setup.exec.program.source_spelling(), "touch");
    let Action::FetchContent(remote_config) = &target.actions["remote-config"] else {
        panic!("remote-config should be a fetch content action");
    };
    assert_eq!(
        remote_config.source.source_spelling(),
        "https://example.com/config.toml"
    );
    assert_eq!(remote_config.target.source_spelling(), "configs/app.toml");
    assert_eq!(
        remote_config.on_conflict,
        Some(FetchContentConflict::Replace)
    );

    let laptop = &target.profiles["desktop"].profiles["laptop"];
    let power = &laptop.links["power"];
    assert_eq!(power.on_conflict, Some(LinkConflict::Error));
    assert_eq!(power.on_missing_parent, Some(LinkMissingParent::Skip));
}

#[test]
fn fetch_content_conflict_literals_defaults_and_omission_are_distinct() {
    #[derive(serde::Deserialize)]
    struct Document {
        action: Action,
    }

    fn deserialize_conflict(value: Option<&str>) -> Option<FetchContentConflict> {
        let conflict = value
            .map(|value| format!("on_conflict = {value:?}"))
            .unwrap_or_default();
        let input = format!(
            r#"
[action]
source = "https://example.com/config.toml"
target = "configs/app.toml"
{conflict}
"#
        );
        let document: Document = toml::from_str(&input).expect("fetch action should deserialize");
        let Action::FetchContent(action) = document.action else {
            panic!("source and target should select the fetch content variant");
        };
        action.on_conflict
    }

    assert_eq!(FetchContentConflict::default(), FetchContentConflict::Error);
    assert_eq!(
        deserialize_conflict(Some("error")),
        Some(FetchContentConflict::Error)
    );
    assert_eq!(
        deserialize_conflict(Some("replace")),
        Some(FetchContentConflict::Replace)
    );
    assert_eq!(deserialize_conflict(None), None);
}

#[test]
fn deserializes_strings_into_their_declared_schema_roles() {
    let input = fixture::read("schema/valid-string-roles.toml");
    let config: Config = toml::from_str(&input).expect("schema roles should deserialize");

    let (target_id, target) = config.targets.first().expect("target exists");
    let _: &SelectorIdentifier = target_id;
    assert_eq!(target_id.as_str(), "machine");

    let provider = &target.providers["brew"];
    let _: &StringExpressionSource = &provider.probe.program;
    let _: &ExecAction<StringExpressionSource, ProviderInstallArgSource> = &provider.install;
    let _: &ProviderInstallArgSource = &provider.install.args[1];
    assert_eq!(
        provider.install.args[1].source_spelling(),
        "${package:provider_args}"
    );

    let Package::Provider(ProviderPackage::Single(package)) = &target.packages["application"]
    else {
        panic!("application should use a provider");
    };
    let provider_arg: &LiteralStringSource =
        &package.provider_args.as_ref().expect("args exist")[0];
    assert_eq!(provider_arg.source_spelling(), "--cask");

    let (name, value) = provider
        .activate
        .as_ref()
        .expect("activation exists")
        .variables
        .first_key_value()
        .expect("variable exists");
    let _: &EnvironmentName = name;
    let _: &StringExpressionSource = value;
    assert_eq!(name.as_str(), "HOMEBREW_PREFIX");
    assert_eq!(value.source_spelling(), "${env:HOME}/.homebrew");
}

#[test]
fn expression_syntax_errors_are_recoverable_during_deserialization() {
    let input = fixture::read("schema/valid-recoverable-string-errors.toml");

    toml::from_str::<Config>(&input)
        .expect("expression syntax is validated only when a consumer uses the field");
}

#[test]
fn rejects_invalid_identifiers_while_deserializing() {
    let input = fixture::read("schema/invalid-identifier.toml");

    assert!(toml::from_str::<Config>(&input).is_err());
}

#[test]
fn selectable_table_keys_use_selector_identifiers() {
    for fixture_name in [
        "schema/invalid-selector-target-id.toml",
        "schema/invalid-selector-profile-id.toml",
        "schema/invalid-selector-job-id.toml",
    ] {
        let input = fixture::read(fixture_name);
        assert!(
            toml::from_str::<Config>(&input).is_err(),
            "{fixture_name} must fail"
        );
    }
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

#[test]
fn rejects_invalid_fetch_content_action_shapes() {
    for fixture_name in [
        "schema/invalid-mixed-action.toml",
        "schema/invalid-incomplete-fetch-action.toml",
        "schema/invalid-fetch-action-unknown-field.toml",
        "schema/invalid-fetch-conflict.toml",
    ] {
        let input = fixture::read(fixture_name);
        assert!(
            toml::from_str::<Config>(&input).is_err(),
            "{fixture_name} must fail"
        );
    }
}

#[test]
fn rejects_a_fetch_content_shape_as_a_manual_package_install() {
    let input = fixture::read("schema/invalid-manual-fetch-install.toml");

    assert!(toml::from_str::<Config>(&input).is_err());
}

#[test]
fn rejects_nested_provider_install_argument_arrays() {
    let input = fixture::read("schema/invalid-nested-provider-install-args.toml");

    assert!(toml::from_str::<Config>(&input).is_err());
}

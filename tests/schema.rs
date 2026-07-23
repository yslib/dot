mod support;

use dot::schema::{
    Config, EnvironmentName, ExecAction, ExecActionType, ExpressionParseError, Identifier,
    LinkConflict, LinkMissingParent, ListType, LiteralString, LiteralStringSource, OneOrMany,
    Package, ParsedStringForm, ParsedTemplatePart, ProviderInstallArg, ProviderInstallArgSource,
    ProviderPackage, RecordTypeId, ResolvedString, ScalarTemplate, SchemaType, SchemaTypeMarker,
    StringExpressionSource, StringKeyType, StringRefinementTypeId, StringType,
};

use support::fixture;

#[test]
fn describes_every_toml_literal_and_nested_schema_type() {
    let primitives = [
        SchemaType::String,
        SchemaType::Integer,
        SchemaType::Float,
        SchemaType::Boolean,
        SchemaType::OffsetDateTime,
        SchemaType::LocalDateTime,
        SchemaType::LocalDate,
        SchemaType::LocalTime,
    ];
    assert_eq!(primitives.len(), 8);

    let nested = SchemaType::List(Box::new(SchemaType::Map(
        StringKeyType::Refinement(StringRefinementTypeId::new("environment_name")),
        Box::new(SchemaType::Record(RecordTypeId::new("exec_action"))),
    )));
    assert_eq!(
        nested,
        SchemaType::List(Box::new(SchemaType::Map(
            StringKeyType::Refinement(StringRefinementTypeId::new("environment_name")),
            Box::new(SchemaType::Record(RecordTypeId::new("exec_action"))),
        )))
    );
}

#[test]
fn logical_markers_have_runtime_schema_signatures() {
    assert_eq!(StringType::schema_type(), SchemaType::String);
    assert_eq!(
        ListType::<StringType>::schema_type(),
        SchemaType::List(Box::new(SchemaType::String))
    );
}

#[test]
fn logical_markers_define_their_resolved_value_types() {
    let owned: <StringType as SchemaTypeMarker>::Resolved = String::from("owned").into();
    let borrowed: ResolvedString = "borrowed".into();
    let list: <ListType<StringType> as SchemaTypeMarker>::Resolved = vec![owned, borrowed];

    assert_eq!(list[0].value(), "owned");
    assert_eq!(list[1].value(), "borrowed");
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

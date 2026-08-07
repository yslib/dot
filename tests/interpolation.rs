use std::collections::BTreeMap;
use std::path::Path;

use dot_core::interpolation::{
    DotPaths, ExecutionEnvironment, InterpolationError, PackageContext, ResolveContext, XdgPaths,
    promote_string_expression, resolve_environment_patch, resolve_exec_action,
    resolve_literal_string, resolve_provider_install_action, resolve_string_expression,
};
use dot_core::schema::{
    EnvironmentName, EnvironmentPatch, ExecAction, LiteralStringSource, OneOrMany,
    ProviderInstallArgSource, ResolvedString, SourceExecAction, StringExpressionSource,
};

fn dot_paths() -> DotPaths<'static> {
    DotPaths::new(
        Path::new("config-dir"),
        Path::new("real-config-dir"),
        Path::new("working-dir"),
    )
}

#[test]
fn resolves_environment_and_dot_values_across_an_action() {
    let environment =
        ExecutionEnvironment::from_variables([("PROGRAM", "tool"), ("ROOT", "/opt/tools")]);
    let xdg = XdgPaths::detect();
    let context = ResolveContext::new(&environment, dot_paths(), &xdg);
    let action: SourceExecAction = ExecAction {
        program: "${env:PROGRAM}".into(),
        args: vec![
            "--config=${dot:config_dir}".into(),
            "--real=${dot:real_config_dir}".into(),
        ],
        cwd: Some("${dot:cwd}".into()),
        env: Some(EnvironmentPatch {
            path_prepend: Some(OneOrMany::One("${env:ROOT}/bin".into())),
            path_append: None,
            variables: BTreeMap::from([(
                EnvironmentName::new("TOOL_HOME").expect("test name should be valid"),
                "${env:ROOT}".into(),
            )]),
        }),
    };

    let resolved = resolve_exec_action(&action, &context).expect("action should resolve");

    assert_eq!(resolved.program.value(), "tool");
    assert_eq!(
        resolved
            .args
            .iter()
            .map(ResolvedString::value)
            .collect::<Vec<_>>(),
        ["--config=config-dir", "--real=real-config-dir"]
    );
    assert_eq!(resolved.cwd.as_ref().unwrap().value(), "working-dir");
    let resolved_environment = resolved.env.expect("environment should resolve");
    assert_eq!(
        resolved_environment
            .path_prepend
            .as_ref()
            .and_then(|values| match values {
                OneOrMany::One(value) => Some(value.value()),
                OneOrMany::Many(_) => None,
            }),
        Some("/opt/tools/bin")
    );
    assert_eq!(
        resolved_environment.variables["TOOL_HOME"].value(),
        "/opt/tools"
    );
}

#[test]
fn literal_strings_unescape_syntax_and_reject_resolvers() {
    assert_eq!(
        resolve_literal_string(&LiteralStringSource::from(r"prefix-\${literal}"))
            .expect("escaped resolver syntax should remain literal")
            .value(),
        "prefix-${literal}"
    );
    assert_eq!(
        resolve_literal_string(&LiteralStringSource::from("${env:HOME}")),
        Err(InterpolationError::ResolverInLiteralString {
            resolver: "env".into(),
        })
    );
}

#[test]
fn rejects_unknown_removed_invalid_and_unavailable_resolvers() {
    let cases = [
        (
            "${future:value}",
            InterpolationError::UnknownResolver {
                name: "future".into(),
            },
        ),
        (
            "${path:cwd}",
            InterpolationError::UnknownResolver {
                name: "path".into(),
            },
        ),
        (
            "${dot:config}",
            InterpolationError::InvalidResolverPayload {
                resolver: "dot".into(),
                payload: "config".into(),
            },
        ),
        (
            "${xdg:repository}",
            InterpolationError::InvalidResolverPayload {
                resolver: "xdg".into(),
                payload: "repository".into(),
            },
        ),
        (
            "${package:names}",
            InterpolationError::ResolverUnavailable {
                resolver: "package".into(),
            },
        ),
    ];

    for (source, expected) in cases {
        assert_eq!(
            promote_string_expression(&StringExpressionSource::from(source)),
            Err(expected),
            "source: {source}"
        );
    }
}

#[test]
fn reports_malformed_resolver_syntax_when_the_value_is_consumed() {
    assert_eq!(
        promote_string_expression(&StringExpressionSource::from("prefix-${env:HOME")),
        Err(InterpolationError::UnclosedResolver { offset: 7 })
    );
}

#[test]
fn every_documented_xdg_payload_is_accepted() {
    for payload in [
        "home",
        "config",
        "config_local",
        "data",
        "data_local",
        "cache",
        "state",
        "runtime",
        "executable",
        "documents",
    ] {
        let source = StringExpressionSource::from(format!("${{xdg:{payload}}}"));
        promote_string_expression(&source)
            .unwrap_or_else(|error| panic!("xdg payload `{payload}` should be valid: {error}"));
    }
}

#[test]
fn provider_install_expands_package_lists_into_arguments() {
    let environment = ExecutionEnvironment::from_variables([("PROVIDER", "brew")]);
    let xdg = XdgPaths::detect();
    let names = vec!["font-one".to_owned(), "font-two".to_owned()];
    let provider_args = vec!["--cask".to_owned(), "--force".to_owned()];
    let context = ResolveContext::new(&environment, dot_paths(), &xdg)
        .with_package(PackageContext::new(&names, &provider_args));
    let action = ExecAction::<StringExpressionSource, ProviderInstallArgSource> {
        program: "${env:PROVIDER}".into(),
        args: vec![
            "install".into(),
            "${package:provider_args}".into(),
            "${package:names}".into(),
        ],
        cwd: None,
        env: None,
    };

    let resolved =
        resolve_provider_install_action(&action, &context).expect("install should resolve");

    assert_eq!(resolved.program.value(), "brew");
    assert_eq!(
        resolved
            .args
            .iter()
            .map(ResolvedString::value)
            .collect::<Vec<_>>(),
        ["install", "--cask", "--force", "font-one", "font-two"]
    );
}

#[test]
fn package_list_resolvers_must_occupy_a_complete_argument() {
    let environment = ExecutionEnvironment::empty();
    let xdg = XdgPaths::detect();
    let names = vec!["ripgrep".to_owned()];
    let provider_args = Vec::new();
    let context = ResolveContext::new(&environment, dot_paths(), &xdg)
        .with_package(PackageContext::new(&names, &provider_args));
    let action = ExecAction::<StringExpressionSource, ProviderInstallArgSource> {
        program: "install".into(),
        args: vec!["prefix-${package:names}".into()],
        cwd: None,
        env: None,
    };

    assert_eq!(
        resolve_provider_install_action(&action, &context),
        Err(InterpolationError::ListResolverMustOccupyArgument {
            resolver: "package".into(),
        })
    );
}

#[test]
fn environment_patch_resolves_every_value_against_one_context() {
    let environment = ExecutionEnvironment::from_variables([("ROOT", "/opt/tools")]);
    let xdg = XdgPaths::detect();
    let context = ResolveContext::new(&environment, dot_paths(), &xdg);
    let patch = EnvironmentPatch {
        path_prepend: Some(OneOrMany::Many(vec![
            "${env:ROOT}/bin".into(),
            "${dot:config_dir}/bin".into(),
        ])),
        path_append: None,
        variables: BTreeMap::new(),
    };

    let resolved = resolve_environment_patch(&patch, &context).expect("patch should resolve");

    let Some(OneOrMany::Many(values)) = resolved.path_prepend else {
        panic!("path prepend should preserve its list form");
    };
    assert_eq!(
        values.iter().map(ResolvedString::value).collect::<Vec<_>>(),
        ["/opt/tools/bin", "config-dir/bin"]
    );
}

#[test]
fn dot_resolver_returns_each_protocol_directory() {
    let environment = ExecutionEnvironment::empty();
    let xdg = XdgPaths::detect();
    let context = ResolveContext::new(&environment, dot_paths(), &xdg);
    let source =
        StringExpressionSource::from("${dot:config_dir}|${dot:real_config_dir}|${dot:cwd}");

    assert_eq!(
        resolve_string_expression(&source, &context)
            .expect("dot paths should resolve")
            .value(),
        "config-dir|real-config-dir|working-dir"
    );
}

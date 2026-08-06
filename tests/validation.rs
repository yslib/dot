mod support;

use std::error::Error;

use dot_core::config::ConfigParseError;
use dot_core::interpolation::InterpolationError;
use dot_core::schema::Config;
use dot_core::validation::{ConfigValidationErrorKind, ConfigValidationJob, validate_config};
use support::fixture;

#[test]
fn rejects_a_static_expression_error_in_an_unselected_target() {
    let error = Config::parse(&fixture::read(
        "validation/invalid-unselected-expression.toml",
    ))
    .expect_err("the complete config must be validated");

    let ConfigParseError::Validation { source } = &error else {
        panic!("expected a validation error, got {error:?}");
    };
    assert_eq!(source.target.as_str(), "unselected");
    assert_eq!(source.profile, None);
    assert_eq!(
        source.job.as_ref(),
        Some(&ConfigValidationJob::Action(
            "broken"
                .try_into()
                .expect("fixture job name should be valid")
        ))
    );
    assert_eq!(source.field.as_deref(), Some("exec.program"));
    assert!(matches!(
        &source.kind,
        ConfigValidationErrorKind::Expression(InterpolationError::UnknownResolver {
            name
        }) if name == "unknown"
    ));
    assert!(error.source().is_some());
    assert!(source.source().is_some());
}

#[test]
fn rejects_a_fetch_source_expression_error_in_an_unselected_target() {
    let error = Config::parse(&fixture::read(
        "validation/invalid-unselected-fetch-expression.toml",
    ))
    .expect_err("the complete config must be validated");
    let ConfigParseError::Validation { source } = error else {
        panic!("expected a validation error");
    };

    assert_eq!(source.target.as_str(), "unselected");
    assert_eq!(source.profile, None);
    assert_eq!(
        source.job,
        Some(ConfigValidationJob::Action(
            "broken".try_into().expect("fixture action id is valid")
        ))
    );
    assert_eq!(source.field.as_deref(), Some("source"));
    assert!(matches!(
        source.kind,
        ConfigValidationErrorKind::Expression(InterpolationError::UnknownResolver { ref name })
            if name == "unknown"
    ));
}

#[test]
fn rejects_a_fetch_target_expression_error_in_a_profile_replacement() {
    let input = r#"
[targets.machine]
platform = { os = "linux" }

[targets.machine.actions.remote-config]
source = "https://example.com/base.toml"
target = "configs/base.toml"

[targets.machine.profiles.work.actions.remote-config]
source = "https://example.com/work.toml"
target = "${env"
"#;
    let config: Config = toml::from_str(input).expect("test config should deserialize");

    let error = validate_config(&config).expect_err("profile replacement must be validated");

    assert_eq!(error.target.as_str(), "machine");
    assert_eq!(error.profile.as_ref().map(AsRef::as_ref), Some("work"));
    assert_eq!(
        error.job,
        Some(ConfigValidationJob::Action(
            "remote-config"
                .try_into()
                .expect("fixture action id is valid")
        ))
    );
    assert_eq!(error.field.as_deref(), Some("target"));
    assert!(matches!(
        error.kind,
        ConfigValidationErrorKind::Expression(InterpolationError::UnclosedResolver { offset: 0 })
    ));
}

#[test]
fn defers_a_missing_runtime_value_in_an_unselected_job() {
    let config = Config::parse(&fixture::read(
        "validation/valid-unselected-runtime-value.toml",
    ))
    .expect("static validation must not resolve runtime environment values");

    assert_eq!(config.targets.len(), 2);
}

#[test]
fn rejects_an_unknown_provider_in_one_effective_profile_atomically() {
    let error = Config::parse(&fixture::read(
        "validation/invalid-unselected-provider-reference.toml",
    ))
    .expect_err("every effective profile manifest must be valid");

    let ConfigParseError::Validation { source } = &error else {
        panic!("expected a validation error, got {error:?}");
    };
    assert_eq!(source.target.as_str(), "machine");
    assert_eq!(
        source.profile.as_ref().map(ToString::to_string).as_deref(),
        Some("broken")
    );
    assert_eq!(
        source.job.as_ref(),
        Some(&ConfigValidationJob::Package(
            "tool".try_into().expect("fixture job name should be valid")
        ))
    );
    assert_eq!(source.field.as_deref(), Some("provider"));
    assert!(matches!(
        &source.kind,
        ConfigValidationErrorKind::UnknownProvider { package, provider }
            if package.as_str() == "tool" && provider.as_str() == "missing"
    ));
}

#[test]
fn validates_provider_package_manual_action_and_link_declarations() {
    let cases = [
        (
            r#"
[targets.machine]
platform = { os = "linux" }
[targets.machine.providers.broken]
probe = { program = "${unknown:value}" }
install = { program = "install" }
"#,
            ConfigValidationJob::Provider(
                "broken"
                    .try_into()
                    .expect("fixture provider name should be valid"),
            ),
        ),
        (
            r#"
[targets.machine]
platform = { os = "linux" }
[targets.machine.providers.system]
probe = { program = "probe" }
install = { program = "install", args = ["${package:provider_args}", "${package:names}"] }
[targets.machine.packages]
broken = { provider = "system", provider_args = ["${env:HOME}"] }
"#,
            ConfigValidationJob::Package(
                "broken"
                    .try_into()
                    .expect("fixture package name should be valid"),
            ),
        ),
        (
            r#"
[targets.machine]
platform = { os = "linux" }
[targets.machine.packages.broken]
install = { exec = { program = "${unknown:value}" } }
"#,
            ConfigValidationJob::Package(
                "broken"
                    .try_into()
                    .expect("fixture package name should be valid"),
            ),
        ),
        (
            r#"
[targets.machine]
platform = { os = "linux" }
[targets.machine.links.broken]
source = "${unknown:value}"
target = "/target"
"#,
            ConfigValidationJob::Link(
                "broken"
                    .try_into()
                    .expect("fixture link name should be valid"),
            ),
        ),
    ];

    for (input, expected_job) in cases {
        let config: Config = toml::from_str(input).expect("test config should deserialize");

        let error = validate_config(&config).expect_err("the declaration must fail validation");

        assert_eq!(error.job, Some(expected_job));
        assert!(
            matches!(
                &error.kind,
                ConfigValidationErrorKind::Expression(
                    InterpolationError::UnknownResolver { .. }
                        | InterpolationError::ResolverInLiteralString { .. }
                )
            ),
            "{error}"
        );
    }
}

#[test]
fn validates_package_batch_structure_during_parse() {
    let cases = [
        (
            "dry-run/invalid-empty-package-batch.toml",
            "empty-tools",
            None,
        ),
        (
            "dry-run/invalid-duplicate-package-batch-name.toml",
            "duplicate-tools",
            Some("ripgrep"),
        ),
    ];

    for (fixture_name, expected_package, expected_duplicate) in cases {
        let error = Config::parse(&fixture::read(fixture_name))
            .expect_err("invalid batches must fail during parse");
        let ConfigParseError::Validation { source } = error else {
            panic!("expected a validation error");
        };

        assert!(
            match &source.kind {
                ConfigValidationErrorKind::EmptyPackageBatch { package } => {
                    package.as_str() == expected_package && expected_duplicate.is_none()
                }
                ConfigValidationErrorKind::DuplicatePackageBatchName { package, name } => {
                    package.as_str() == expected_package
                        && Some(name.as_str()) == expected_duplicate
                }
                _ => false,
            },
            "unexpected error for {fixture_name}: {source}"
        );
    }
}

#[test]
fn checks_nested_provider_args_against_the_inherited_ancestor_provider_override() {
    let input = r#"
[targets.machine]
platform = { os = "linux" }

[targets.machine.providers.brew]
probe = { program = "brew" }
install = { program = "brew", args = ["${package:provider_args}", "${package:names}"] }

[targets.machine.profiles.workstation.providers.brew]
probe = { program = "workstation-brew" }
install = { program = "workstation-brew", args = ["${package:names}"] }

[targets.machine.profiles.workstation.profiles.leaf.packages]
app = { provider = "brew", provider_args = ["--cask"] }
"#;
    let config: Config = toml::from_str(input).expect("test config should deserialize");

    let error = validate_config(&config)
        .expect_err("the inherited package must use the effective provider definition");

    assert_eq!(error.profile.as_ref().map(AsRef::as_ref), Some("leaf"));
    assert_eq!(
        error.job,
        Some(ConfigValidationJob::Package(
            "app"
                .try_into()
                .expect("fixture package name should be valid")
        ))
    );
    assert!(matches!(
        &error.kind,
        ConfigValidationErrorKind::ProviderArgsResolverCount {
            provider,
            actual: 0,
        } if provider.as_str() == "brew"
    ));
    assert_eq!(error.field.as_deref(), Some("provider.install.args"));
}

#[test]
fn requires_one_exact_provider_args_resolver_during_parse() {
    let cases = [
        ("invalid-provider-args-resolver.toml", 0),
        ("invalid-provider-args-resolver-twice.toml", 2),
        ("invalid-provider-args-resolver-escaped.toml", 0),
    ];

    for (fixture_name, expected_count) in cases {
        let error = Config::parse(&fixture::read(format!("dry-run/{fixture_name}")))
            .expect_err("provider args must be consumed exactly once");
        let ConfigParseError::Validation { source } = error else {
            panic!("expected a validation error");
        };

        assert!(
            matches!(
                &source.kind,
                ConfigValidationErrorKind::ProviderArgsResolverCount {
                    provider,
                    actual,
                } if provider.as_str() == "brew" && *actual == expected_count
            ),
            "unexpected error for {fixture_name}: {source}"
        );
    }
}

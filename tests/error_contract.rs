use std::env;
use std::error::Error;
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;

use dot::action::{CommandPreparationError, ExecutionError};
use dot::action_runner::{ActionRunError, ActionStage};
use dot::check::ProviderCheckError;
use dot::config::{ConfigDiscoveryError, ConfigLoadError};
use dot::diagnostic::Operation;
use dot::interpolation::InterpolationError;
use dot::job::{JobSelector, JobSelectorParseError};
use dot::link::{LinkError, LinkPhaseError};
use dot::manifest::ManifestError;
use dot::plan::{JobSelectionError, PlanningError};
use dot::platform::PlatformInfo;
use dot::provider::{ProviderError, ProviderInstallError, ProviderStage};
use dot::schema::{
    Identifier, OneOrMany, PlatformConstraint, SchemaType, SelectorIdentifier,
    SelectorIdentifierError,
};
use dot::validation::{ConfigValidationError, ConfigValidationErrorKind, ConfigValidationJob};

fn assert_no_source(error: &(dyn Error + 'static)) {
    assert!(
        error.source().is_none(),
        "expected `{error}` to have no source"
    );
}

fn assert_source_is<'a, T: Error + 'static>(error: &'a (dyn Error + 'static)) -> &'a T {
    let source = error
        .source()
        .unwrap_or_else(|| panic!("expected `{error}` to have an immediate source"));
    source.downcast_ref::<T>().unwrap_or_else(|| {
        panic!(
            "expected immediate source of `{error}` to be `{}`, but it was `{source}`",
            std::any::type_name::<T>()
        )
    })
}

fn io_error() -> io::Error {
    io::Error::other("test I/O failure")
}

fn join_paths_error() -> env::JoinPathsError {
    #[cfg(not(windows))]
    let invalid_path = "invalid:path";
    #[cfg(windows)]
    let invalid_path = "invalid\"path";

    env::join_paths([invalid_path]).expect_err("test path should be invalid in PATH")
}

fn toml_error() -> toml::de::Error {
    toml::from_str::<toml::Value>("invalid = [")
        .expect_err("test TOML should be syntactically invalid")
}

fn identifier(value: &str) -> Identifier {
    Identifier::new(value).expect("test identifier should be valid")
}

fn selector_identifier(value: &str) -> SelectorIdentifier {
    SelectorIdentifier::new(value).expect("test selector identifier should be valid")
}

fn selector_identifier_error() -> SelectorIdentifierError {
    SelectorIdentifier::new("").expect_err("empty selector identifier should be invalid")
}

fn interpolation_error() -> InterpolationError {
    InterpolationError::UnclosedResolver { offset: 0 }
}

fn preparation_error() -> CommandPreparationError {
    CommandPreparationError::InvalidPathEnvironment {
        source: join_paths_error(),
    }
}

fn execution_error() -> ExecutionError {
    ExecutionError::Spawn {
        program: OsString::from("test-command"),
        source: io_error(),
    }
}

fn validation_error(kind: ConfigValidationErrorKind) -> ConfigValidationError {
    ConfigValidationError {
        target: selector_identifier("target"),
        profile: Some(selector_identifier("profile")),
        job: Some(ConfigValidationJob::Provider(identifier("provider"))),
        field: Some("field".to_owned()),
        kind: Box::new(kind),
    }
}

#[test]
fn command_preparation_error_exposes_join_paths_error() {
    assert_source_is::<env::JoinPathsError>(&preparation_error());
}

#[test]
fn execution_errors_expose_io_errors() {
    let errors = [
        ExecutionError::Spawn {
            program: OsString::from("spawn"),
            source: io_error(),
        },
        ExecutionError::Wait {
            program: OsString::from("wait"),
            source: io_error(),
        },
    ];

    for error in &errors {
        assert_source_is::<io::Error>(error);
    }
}

#[test]
fn action_run_errors_expose_their_wrapper_errors() {
    let preparation = ActionRunError::Preparation {
        stage: ActionStage::Exec,
        source: preparation_error(),
    };
    let execution = ActionRunError::Execution {
        stage: ActionStage::Exec,
        source: execution_error(),
    };

    assert_source_is::<CommandPreparationError>(&preparation);
    assert_source_is::<ExecutionError>(&execution);
}

#[test]
fn provider_check_errors_expose_their_wrapper_errors() {
    let errors = [
        ProviderCheckError::ActivateInterpolation(interpolation_error()),
        ProviderCheckError::ProbeInterpolation(interpolation_error()),
    ];
    for error in &errors {
        assert_source_is::<InterpolationError>(error);
    }

    let errors = [
        ProviderCheckError::ActivatePreparation(preparation_error()),
        ProviderCheckError::ProbePreparation(preparation_error()),
    ];
    for error in &errors {
        assert_source_is::<CommandPreparationError>(error);
    }

    let converted = ProviderCheckError::from(execution_error());
    assert!(matches!(&converted, ProviderCheckError::Execution(_)));
    assert_source_is::<ExecutionError>(&converted);
}

#[test]
fn config_discovery_errors_expose_io_errors() {
    let errors = [
        ConfigDiscoveryError::CurrentDirectory { source: io_error() },
        ConfigDiscoveryError::Inspect {
            path: PathBuf::from("candidate"),
            source: io_error(),
        },
    ];

    for error in &errors {
        assert_source_is::<io::Error>(error);
    }
}

#[test]
fn config_load_errors_expose_their_immediate_errors() {
    let current_directory = ConfigLoadError::CurrentDirectory { source: io_error() };
    let read = ConfigLoadError::Read {
        path: PathBuf::from("dot.toml"),
        source: io_error(),
    };
    let parse = ConfigLoadError::Parse {
        path: PathBuf::from("dot.toml"),
        source: toml_error(),
    };

    assert_source_is::<io::Error>(&current_directory);
    assert_source_is::<io::Error>(&read);
    assert_source_is::<toml::de::Error>(&parse);
}

#[test]
fn config_load_validation_preserves_all_three_immediate_source_layers() {
    let error = ConfigLoadError::Validation {
        path: PathBuf::from("dot.toml"),
        source: Box::new(validation_error(ConfigValidationErrorKind::Expression(
            interpolation_error(),
        ))),
    };

    let validation = assert_source_is::<ConfigValidationError>(&error);
    let kind = assert_source_is::<ConfigValidationErrorKind>(validation);
    assert_source_is::<InterpolationError>(kind);
}

#[test]
fn selector_parse_error_exposes_selector_identifier_error() {
    let error = JobSelectorParseError::InvalidIdentifier(selector_identifier_error());

    let source = assert_source_is::<SelectorIdentifierError>(&error);
    assert_no_source(source);
}

#[test]
fn link_io_error_exposes_io_error() {
    let error = LinkError::Io {
        operation: "inspect",
        path: PathBuf::from("target"),
        source: io_error(),
        diagnostic_operation: Some(Operation::CreateSymbolicLink),
    };

    assert_source_is::<io::Error>(&error);
}

#[test]
fn planning_errors_expose_their_wrapper_errors() {
    let interpolation = PlanningError::Interpolation {
        context: "test field".to_owned(),
        source: interpolation_error(),
    };
    let environment_patch = PlanningError::EnvironmentPatch {
        provider: "provider".to_owned(),
        source: preparation_error(),
    };

    assert_source_is::<InterpolationError>(&interpolation);
    assert_source_is::<CommandPreparationError>(&environment_patch);
}

#[test]
fn provider_errors_expose_their_wrapper_errors() {
    let environment = ProviderError::Environment {
        stage: ProviderStage::Activate,
        source: preparation_error(),
    };
    let preparation = ProviderError::Preparation {
        stage: ProviderStage::InitialProbe,
        source: preparation_error(),
    };
    let execution = ProviderError::Execution {
        stage: ProviderStage::InitialProbe,
        source: execution_error(),
    };

    assert_source_is::<CommandPreparationError>(&environment);
    assert_source_is::<CommandPreparationError>(&preparation);
    assert_source_is::<ExecutionError>(&execution);
}

#[test]
fn provider_install_errors_expose_their_wrapper_errors() {
    let preparation = ProviderInstallError::Preparation {
        source: preparation_error(),
    };
    let execution = ProviderInstallError::Execution {
        source: execution_error(),
    };

    assert_source_is::<CommandPreparationError>(&preparation);
    assert_source_is::<ExecutionError>(&execution);
}

#[test]
fn validation_error_kinds_expose_their_wrapper_errors() {
    let expression = ConfigValidationErrorKind::Expression(interpolation_error());
    let manifest = ConfigValidationErrorKind::Manifest(ManifestError::NoCompatibleTargets {
        available: Vec::new(),
    });

    assert_source_is::<InterpolationError>(&expression);
    assert_source_is::<ManifestError>(&manifest);
}

#[test]
fn validation_error_exposes_its_kind() {
    let error = validation_error(ConfigValidationErrorKind::EmptyPackageBatch {
        package: selector_identifier("package"),
    });

    assert_source_is::<ConfigValidationErrorKind>(&error);
}

#[test]
fn every_interpolation_error_has_no_source() {
    let errors = [
        InterpolationError::UnclosedResolver { offset: 0 },
        InterpolationError::MissingPayloadSeparator { offset: 0 },
        InterpolationError::NestedResolver { offset: 0 },
        InterpolationError::UnknownResolver {
            name: "resolver".to_owned(),
        },
        InterpolationError::InvalidResolverPayload {
            resolver: "resolver".to_owned(),
            payload: "payload".to_owned(),
        },
        InterpolationError::ResolverUnavailable {
            resolver: "resolver".to_owned(),
        },
        InterpolationError::ResolverTypeMismatch {
            resolver: "resolver".to_owned(),
            expected: SchemaType::String,
            actual: SchemaType::Integer,
        },
        InterpolationError::ResolverContractViolation {
            resolver: "resolver".to_owned(),
            expected: SchemaType::String,
            actual: SchemaType::Integer,
        },
        InterpolationError::ResolverInLiteralString {
            resolver: "resolver".to_owned(),
        },
        InterpolationError::ListResolverMustOccupyArgument {
            resolver: "resolver".to_owned(),
        },
        InterpolationError::MissingEnvironmentVariable {
            name: "VARIABLE".to_owned(),
        },
        InterpolationError::NonUnicodeEnvironmentVariable {
            name: "VARIABLE".to_owned(),
        },
        InterpolationError::UnavailablePath {
            name: "path".to_owned(),
        },
        InterpolationError::NonUnicodePath {
            name: "path".to_owned(),
        },
        InterpolationError::MissingPackageContext,
    ];

    for error in &errors {
        assert_no_source(error);
    }
}

#[test]
fn source_less_selector_errors_have_no_source() {
    let errors = [
        JobSelectorParseError::MissingKind,
        JobSelectorParseError::UnknownKind("unknown".to_owned()),
        JobSelectorParseError::ProviderNotSelectable,
    ];

    for error in &errors {
        assert_no_source(error);
    }
}

#[test]
fn source_less_link_errors_have_no_source() {
    let errors = [
        LinkError::UnsupportedSourceType {
            source: PathBuf::from("source"),
        },
        LinkError::ExistingNonLink {
            target: PathBuf::from("target"),
        },
        LinkError::Conflict {
            target: PathBuf::from("target"),
            destination: PathBuf::from("destination"),
        },
        LinkError::InvalidTarget {
            target: PathBuf::from("target"),
        },
        LinkError::ParentNotDirectory {
            parent: PathBuf::from("parent"),
        },
        LinkError::VerificationMismatch {
            target: PathBuf::from("target"),
            expected: PathBuf::from("expected"),
            actual: Some(PathBuf::from("actual")),
        },
    ];

    for error in &errors {
        assert_no_source(error);
    }

    assert_no_source(&LinkPhaseError::DuplicateTarget {
        target: PathBuf::from("target"),
        links: vec!["first".to_owned(), "second".to_owned()],
    });
}

#[test]
fn every_manifest_error_has_no_source() {
    let errors = [
        ManifestError::NoCompatibleTargets {
            available: Vec::new(),
        },
        ManifestError::TargetRequired {
            available: vec!["target".to_owned()],
        },
        ManifestError::UnknownTarget {
            requested: "requested".to_owned(),
            available: vec!["target".to_owned()],
        },
        ManifestError::IncompatiblePlatform {
            target: "target".to_owned(),
            expected: Box::new(PlatformConstraint {
                os: OneOrMany::One(identifier("test-os")),
                arch: None,
                distro: None,
                distro_family: None,
                environment: None,
            }),
            actual: Box::new(PlatformInfo::detect()),
        },
        ManifestError::DuplicateProfile {
            target: "target".to_owned(),
            profile: "profile".to_owned(),
            first_path: "first".to_owned(),
            second_path: "second".to_owned(),
        },
        ManifestError::UnknownProfile {
            target: "target".to_owned(),
            requested: "requested".to_owned(),
            available: vec!["profile".to_owned()],
        },
    ];

    for error in &errors {
        assert_no_source(error);
    }
}

#[test]
fn source_less_planning_errors_have_no_source() {
    let errors = [
        PlanningError::UnknownProvider {
            package: "package".to_owned(),
            provider: "provider".to_owned(),
        },
        PlanningError::ProviderArgsResolverCount {
            package: "package".to_owned(),
            provider: "provider".to_owned(),
            actual: 0,
        },
        PlanningError::EmptyPackageBatch {
            package: "package".to_owned(),
        },
        PlanningError::DuplicatePackageBatchName {
            package: "package".to_owned(),
            name: "name".to_owned(),
        },
        PlanningError::RelativeLinkTarget {
            link: "link".to_owned(),
            target: PathBuf::from("relative"),
        },
    ];

    for error in &errors {
        assert_no_source(error);
    }
}

#[test]
fn job_selection_errors_have_no_source() {
    let errors = [
        JobSelectionError::Unknown(JobSelector::Package(selector_identifier("package"))),
        JobSelectionError::MissingProvider {
            package: selector_identifier("package"),
            provider: identifier("provider"),
        },
    ];

    for error in &errors {
        assert_no_source(error);
    }
}

#[test]
fn provider_mismatch_has_no_source() {
    assert_no_source(&ProviderInstallError::ProviderMismatch {
        expected: "expected".to_owned(),
        actual: "actual".to_owned(),
    });
}

#[test]
fn source_less_config_discovery_errors_have_no_source() {
    let errors = [
        ConfigDiscoveryError::UserDirectoryUnavailable,
        ConfigDiscoveryError::NotFound {
            local: PathBuf::from("local"),
            user: PathBuf::from("user"),
        },
    ];

    for error in &errors {
        assert_no_source(error);
    }
}

#[test]
fn source_less_validation_error_kinds_have_no_source() {
    let errors = [
        ConfigValidationErrorKind::UnknownProvider {
            package: selector_identifier("package"),
            provider: identifier("provider"),
        },
        ConfigValidationErrorKind::EmptyPackageBatch {
            package: selector_identifier("package"),
        },
        ConfigValidationErrorKind::DuplicatePackageBatchName {
            package: selector_identifier("package"),
            name: identifier("name"),
        },
        ConfigValidationErrorKind::ProviderArgsResolverCount {
            provider: identifier("provider"),
            actual: 0,
        },
    ];

    for error in &errors {
        assert_no_source(error);
    }
}

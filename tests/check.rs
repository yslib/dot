use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::io::Write;
use std::path::Path;
use std::process;

use dot_core::ConfigFile;
use dot_core::interpolation::{DotPaths, ExecutionEnvironment, InterpolationError, XdgPaths};
use dot_core::native::NativeRuntime;
use dot_core::native::provider_check::{
    ProviderChecker, ProviderProbeError, ProviderReadiness, build_report,
};
use dot_core::platform::PlatformInfo;
use dot_core::report::{EvidenceStage, ItemStatus, ReportCommand, ReportStatus};
use dot_core::schema::{
    Config, Entries, EnvironmentName, EnvironmentPatch, ExecAction, OneOrMany, Provider,
    ProviderInstallArgSource, StringExpressionSource,
};
use dot_core::selection::{ProfileSelection, ScopeSelection};

fn environment_patch(variables: &[(&str, &str)]) -> EnvironmentPatch {
    EnvironmentPatch {
        path_prepend: None,
        path_append: None,
        variables: variables
            .iter()
            .map(|(name, value)| {
                (
                    EnvironmentName::new(*name).expect("test name should be valid"),
                    (*value).into(),
                )
            })
            .collect::<BTreeMap<_, _>>(),
    }
}

fn helper_probe() -> ExecAction {
    ExecAction {
        program: env::current_exe()
            .expect("test executable should have a path")
            .to_string_lossy()
            .into_owned()
            .into(),
        args: vec![
            "--exact".into(),
            "helper_process".into(),
            "--nocapture".into(),
        ],
        cwd: None,
        env: None,
    }
}

fn provider(mode: &str, value: &str) -> Provider {
    Provider {
        probe: helper_probe(),
        activate: Some(environment_patch(&[
            ("DOT_CHECK_TEST_HELPER", mode),
            ("PROVIDER_VALUE", value),
        ])),
        ensure: None,
        install: ExecAction::<StringExpressionSource, ProviderInstallArgSource> {
            program: "unused-install".into(),
            args: Vec::new(),
            cwd: None,
            env: None,
        },
    }
}

fn base_environment() -> ExecutionEnvironment {
    ExecutionEnvironment::from_variables([("BASE_ROOT", "/base")])
}

fn dot_paths() -> DotPaths<'static> {
    DotPaths::new(Path::new("/repo"), Path::new("/repo"), Path::new("/work"))
}

fn platform() -> PlatformInfo {
    PlatformInfo {
        os: env::consts::OS.to_owned(),
        arch: env::consts::ARCH.to_owned(),
        distro: None,
        distro_families: BTreeSet::new(),
        environments: BTreeSet::from(["native".to_owned()]),
    }
}

#[test]
fn high_level_provider_check_projects_an_empty_selected_manifest() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let path = directory.path().join("dot.toml");
    std::fs::write(
        &path,
        format!(
            "[targets.machine]\nplatform = {{ os = {:?} }}\n",
            env::consts::OS
        ),
    )
    .expect("test configuration should be written");
    let source = std::fs::read_to_string(&path).expect("test configuration should be readable");
    let parsed = Config::parse(&source).expect("test configuration should parse");
    let config_dir = path
        .parent()
        .expect("test configuration should have a parent")
        .to_owned();
    let real_path = std::fs::canonicalize(&path).expect("test configuration should canonicalize");
    let real_config_dir = real_path
        .parent()
        .expect("canonical configuration should have a parent")
        .to_owned();
    let cwd = env::current_dir().expect("test should have a current directory");
    let config = ConfigFile::new(parsed, config_dir, real_config_dir, cwd)
        .expect("test configuration context should be absolute");
    let runtime = NativeRuntime::detect();
    let scope = ScopeSelection {
        target: Some("machine".try_into().expect("target should be valid")),
        profile: ProfileSelection::Root,
    };

    let report = dot_core::native::check_providers(&config, &runtime, runtime.platform(), &scope)
        .expect("provider check should complete");

    assert_eq!(report.status, ReportStatus::Succeeded);
    assert!(report.items.is_empty());
}

#[test]
fn checks_every_provider_with_its_activated_environment() {
    let providers: Entries<Provider> = [
        (
            "not-ready".try_into().expect("test id should be valid"),
            provider("not-ready", "${env:BASE_ROOT}/second"),
        ),
        (
            "ready".try_into().expect("test id should be valid"),
            provider("ready", "${env:BASE_ROOT}/first"),
        ),
    ]
    .into_iter()
    .collect::<Entries<_>>();
    let xdg = XdgPaths::detect();
    let environment = base_environment();
    let checker = ProviderChecker::new(&environment, dot_paths(), &xdg);

    let report = checker.check(&providers);

    assert_eq!(report.results().len(), 2);
    let not_ready = &report.results()[0];
    assert_eq!(not_ready.provider(), "not-ready");
    assert_eq!(not_ready.readiness(), ProviderReadiness::NotReady);
    assert_eq!(not_ready.output().unwrap().code(), Some(23));
    assert!(
        String::from_utf8_lossy(not_ready.output().unwrap().stdout().unwrap())
            .contains("value=/base/second")
    );

    let ready = &report.results()[1];
    assert_eq!(ready.provider(), "ready");
    assert_eq!(ready.readiness(), ProviderReadiness::Ready);
    assert_eq!(ready.output().unwrap().code(), Some(0));
    assert!(
        String::from_utf8_lossy(ready.output().unwrap().stdout().unwrap())
            .contains("value=/base/first")
    );
    assert!(!report.all_ready());
}

#[test]
fn projects_readiness_and_captured_output_to_structured_evidence() {
    let providers: Entries<Provider> = [(
        "system".try_into().expect("test id should be valid"),
        provider("not-ready", "${env:BASE_ROOT}/system"),
    )]
    .into_iter()
    .collect::<Entries<_>>();
    let xdg = XdgPaths::detect();
    let environment = base_environment();
    let checker = ProviderChecker::new(&environment, dot_paths(), &xdg);

    let checks = checker.check(&providers);
    let report = build_report("machine", None, &platform(), &providers, &checks);

    assert_eq!(report.command, ReportCommand::CheckProviders);
    assert_eq!(report.status, ReportStatus::Failed);
    assert_eq!(report.items.len(), 1);
    assert_eq!(report.items[0].status, ItemStatus::NotReady);
    assert_eq!(report.items[0].evidence.len(), 1);
    let evidence = &report.items[0].evidence[0];
    assert_eq!(evidence.stage, EvidenceStage::Probe);
    assert_eq!(evidence.exit_code, Some(23));
    assert!(
        evidence
            .stdout
            .as_deref()
            .is_some_and(|stdout| stdout.contains("value=/base/system"))
    );
    assert!(
        evidence
            .stderr
            .as_deref()
            .is_some_and(|stderr| stderr.contains("provider is not ready"))
    );
}

#[test]
fn ignores_provider_ensure_and_install_expression_errors() {
    let mut ignored = provider("ready", "ignored-provider-fields");
    ignored.ensure = Some(OneOrMany::Many(vec![
        ExecAction {
            program: "${ensure-program".into(),
            args: Vec::new(),
            cwd: None,
            env: None,
        },
        ExecAction {
            program: "${unknown:ensure}".into(),
            args: Vec::new(),
            cwd: None,
            env: None,
        },
        ExecAction {
            program: "${package:names}".into(),
            args: Vec::new(),
            cwd: None,
            env: None,
        },
    ]));
    ignored.install = ExecAction::<StringExpressionSource, ProviderInstallArgSource> {
        program: "${install-program".into(),
        args: vec![
            "${unknown:install}".into(),
            "prefix-${package:names}".into(),
        ],
        cwd: Some("${package:names}".into()),
        env: None,
    };
    let providers: Entries<Provider> = [(
        "ignored".try_into().expect("test id should be valid"),
        ignored,
    )]
    .into_iter()
    .collect::<Entries<_>>();
    let xdg = XdgPaths::detect();
    let environment = base_environment();
    let checker = ProviderChecker::new(&environment, dot_paths(), &xdg);

    let report = checker.check(&providers);

    assert_eq!(report.results().len(), 1);
    assert_eq!(report.results()[0].readiness(), ProviderReadiness::Ready);
    assert!(
        String::from_utf8_lossy(report.results()[0].output().unwrap().stdout().unwrap())
            .contains("value=ignored-provider-fields")
    );
}

#[test]
fn an_unstartable_probe_does_not_stop_later_providers() {
    let mut missing = provider("unused", "unused");
    missing.probe.program = "dot-provider-probe-that-must-not-exist-3b33529b".into();
    missing.activate = None;
    let providers: Entries<Provider> = [
        (
            "a-missing".try_into().expect("test id should be valid"),
            missing,
        ),
        (
            "z-ready".try_into().expect("test id should be valid"),
            provider("ready", "later-provider-ran"),
        ),
    ]
    .into_iter()
    .collect::<Entries<_>>();
    let xdg = XdgPaths::detect();
    let environment = base_environment();
    let checker = ProviderChecker::new(&environment, dot_paths(), &xdg);

    let report = checker.check(&providers);

    assert_eq!(report.results().len(), 2);
    assert_eq!(report.results()[0].readiness(), ProviderReadiness::NotReady);
    assert!(report.results()[0].output().is_none());
    assert!(
        report.results()[0]
            .error()
            .unwrap()
            .to_string()
            .contains("dot-provider-probe-that-must-not-exist")
    );
    assert_eq!(report.results()[1].readiness(), ProviderReadiness::Ready);
}

#[test]
fn source_promotion_errors_are_provider_local_and_do_not_stop_later_probes() {
    let mut malformed_activate = provider("ready", "unused");
    malformed_activate.activate = Some(environment_patch(&[("BROKEN", "${env:BASE_ROOT")]));

    let mut unknown_probe = provider("ready", "unused");
    unknown_probe.activate = None;
    unknown_probe.probe.program = "${unknown:probe}".into();

    let mut unavailable_probe = provider("ready", "unused");
    unavailable_probe.activate = None;
    unavailable_probe.probe.program = "${package:names}".into();

    let providers: Entries<Provider> = [
        (
            "a-malformed-activate"
                .try_into()
                .expect("test id should be valid"),
            malformed_activate,
        ),
        (
            "b-unknown-probe"
                .try_into()
                .expect("test id should be valid"),
            unknown_probe,
        ),
        (
            "c-unavailable-probe"
                .try_into()
                .expect("test id should be valid"),
            unavailable_probe,
        ),
        (
            "z-ready".try_into().expect("test id should be valid"),
            provider("ready", "later-provider-probe-ran"),
        ),
    ]
    .into_iter()
    .collect::<Entries<_>>();
    let xdg = XdgPaths::detect();
    let environment = base_environment();
    let checker = ProviderChecker::new(&environment, dot_paths(), &xdg);

    let report = checker.check(&providers);

    assert_eq!(
        report
            .results()
            .iter()
            .map(|result| result.provider())
            .collect::<Vec<_>>(),
        [
            "a-malformed-activate",
            "b-unknown-probe",
            "c-unavailable-probe",
            "z-ready",
        ]
    );
    assert!(matches!(
        report.results()[0].error(),
        Some(ProviderProbeError::ActivateInterpolation(
            InterpolationError::UnclosedResolver { .. }
        ))
    ));
    assert!(matches!(
        report.results()[1].error(),
        Some(ProviderProbeError::ProbeInterpolation(
            InterpolationError::UnknownResolver { name }
        )) if name == "unknown"
    ));
    assert!(matches!(
        report.results()[2].error(),
        Some(ProviderProbeError::ProbeInterpolation(
            InterpolationError::ResolverUnavailable { resolver }
        )) if resolver == "package"
    ));
    assert!(
        report.results()[..3]
            .iter()
            .all(|result| result.readiness() == ProviderReadiness::NotReady)
    );
    let ready = &report.results()[3];
    assert_eq!(ready.readiness(), ProviderReadiness::Ready);
    assert!(
        String::from_utf8_lossy(ready.output().unwrap().stdout().unwrap())
            .contains("value=later-provider-probe-ran")
    );
}

#[test]
fn reports_when_provider_activation_cannot_be_resolved() {
    let providers: Entries<Provider> = [(
        "broken".try_into().expect("test id should be valid"),
        provider("ready", "${env:DOT_CHECK_UNDEFINED_VALUE}"),
    )]
    .into_iter()
    .collect::<Entries<_>>();
    let xdg = XdgPaths::detect();
    let environment = base_environment();
    let checker = ProviderChecker::new(&environment, dot_paths(), &xdg);

    let report = checker.check(&providers);

    assert_eq!(report.results()[0].readiness(), ProviderReadiness::NotReady);
    assert!(
        report.results()[0]
            .error()
            .unwrap()
            .to_string()
            .contains("failed to resolve provider activate")
    );
}

#[test]
fn helper_process() {
    let Ok(mode) = env::var("DOT_CHECK_TEST_HELPER") else {
        return;
    };
    let value = env::var("PROVIDER_VALUE").expect("activated value should exist");
    println!("value={value}");

    if mode == "not-ready" {
        std::io::stderr()
            .write_all(b"provider is not ready")
            .expect("helper should write stderr");
        process::exit(23);
    }
}

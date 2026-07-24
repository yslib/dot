use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::io::Write;
use std::path::Path;
use std::process;

use dot::action::ExecutionEnvironment;
use dot::check::{ProviderChecker, ProviderReadiness, build_report};
use dot::interpolation::{DotPaths, XdgPaths};
use dot::platform::PlatformInfo;
use dot::report::{EvidenceStage, ItemStatus, ReportCommand, ReportStatus};
use dot::schema::{
    Entries, EnvironmentName, EnvironmentPatch, ExecAction, Provider, ProviderInstallArgSource,
    ResolvedEnvironmentPatch, ResolvedString, StringExpressionSource,
};

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

fn resolved_environment_patch(variables: &[(&str, &str)]) -> ResolvedEnvironmentPatch {
    ResolvedEnvironmentPatch {
        path_prepend: None,
        path_append: None,
        variables: variables
            .iter()
            .map(|(name, value)| {
                (
                    EnvironmentName::new(*name).expect("test name should be valid"),
                    ResolvedString::from(*value),
                )
            })
            .collect::<BTreeMap<_, _>>(),
    }
}

fn helper_probe() -> ExecAction {
    ExecAction {
        kind: None,
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
            kind: None,
            program: "unused-install".into(),
            args: Vec::new(),
            cwd: None,
            env: None,
        },
    }
}

fn base_environment() -> ExecutionEnvironment {
    let mut environment = ExecutionEnvironment::empty();
    environment
        .apply_patch(&resolved_environment_patch(&[("BASE_ROOT", "/base")]))
        .expect("base environment should be valid");
    environment
}

fn dot_paths() -> DotPaths<'static> {
    DotPaths::new(
        Path::new("/repo/dot.toml"),
        Path::new("/repo"),
        Path::new("/work"),
    )
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
fn checks_every_provider_with_its_activated_environment() {
    let providers: Entries<Provider> = BTreeMap::from([
        (
            "not-ready".try_into().expect("test id should be valid"),
            provider("not-ready", "${env:BASE_ROOT}/second"),
        ),
        (
            "ready".try_into().expect("test id should be valid"),
            provider("ready", "${env:BASE_ROOT}/first"),
        ),
    ]);
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
    let providers: Entries<Provider> = BTreeMap::from([(
        "system".try_into().expect("test id should be valid"),
        provider("not-ready", "${env:BASE_ROOT}/system"),
    )]);
    let xdg = XdgPaths::detect();
    let environment = base_environment();
    let checker = ProviderChecker::new(&environment, dot_paths(), &xdg);

    let checks = checker.check(&providers);
    let report = build_report(
        Path::new("/repo/dot.toml"),
        "machine",
        None,
        &platform(),
        &providers,
        &checks,
    );

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
fn an_unstartable_probe_does_not_stop_later_providers() {
    let mut missing = provider("unused", "unused");
    missing.probe.program = "dot-provider-probe-that-must-not-exist-3b33529b".into();
    missing.activate = None;
    let providers: Entries<Provider> = BTreeMap::from([
        (
            "a-missing".try_into().expect("test id should be valid"),
            missing,
        ),
        (
            "z-ready".try_into().expect("test id should be valid"),
            provider("ready", "later-provider-ran"),
        ),
    ]);
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
fn reports_when_provider_activation_cannot_be_resolved() {
    let providers: Entries<Provider> = BTreeMap::from([(
        "broken".try_into().expect("test id should be valid"),
        provider("ready", "${env:DOT_CHECK_UNDEFINED_VALUE}"),
    )]);
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

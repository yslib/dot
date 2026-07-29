use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};

use dot::action::ExecutionEnvironment;
use dot::interpolation::{DotPaths, XdgPaths};
use dot::job::JobSelection;
use dot::manifest::EffectiveManifest;
use dot::plan::{ExecutionPlan, ExecutionPlanner};
use dot::platform::PlatformInfo;
use dot::provider::{ProviderInstallError, ProviderInstallOutcome, ProviderRunner};
use dot::schema::{
    BatchProviderPackage, Config, Entries, EnvironmentName, EnvironmentPatch, ExecAction,
    Identifier, OneOrMany, Package, PlatformConstraint, Provider, ProviderInstallArgSource,
    ProviderPackage, SelectableEntries, SelectorIdentifier, SingleProviderPackage,
    StringExpressionSource, Target,
};

static NEXT_STATE: AtomicU64 = AtomicU64::new(0);

struct TempState {
    directory: PathBuf,
}

impl TempState {
    fn new() -> Self {
        let sequence = NEXT_STATE.fetch_add(1, Ordering::Relaxed);
        let directory =
            env::temp_dir().join(format!("dot-provider-batch-{}-{sequence}", process::id()));
        fs::create_dir(&directory).expect("temporary state directory should be created");
        Self { directory }
    }

    fn events(&self) -> PathBuf {
        self.directory.join("events")
    }

    fn recorded_events(&self) -> Vec<String> {
        fs::read_to_string(self.events())
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

impl Drop for TempState {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn provider_id(value: &str) -> Identifier {
    Identifier::new(value).expect("test identifier should be valid")
}

fn selector_id(value: &str) -> SelectorIdentifier {
    SelectorIdentifier::new(value).expect("test selector identifier should be valid")
}

fn variables(values: &[(&str, String)]) -> BTreeMap<EnvironmentName, StringExpressionSource> {
    values
        .iter()
        .map(|(name, value)| {
            (
                EnvironmentName::new(*name).expect("test environment name should be valid"),
                StringExpressionSource::from(value.clone()),
            )
        })
        .collect()
}

fn helper_action<A>(mode: &str, state: &TempState) -> ExecAction<StringExpressionSource, A>
where
    A: From<&'static str>,
{
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
        env: Some(EnvironmentPatch {
            path_prepend: None,
            path_append: None,
            variables: variables(&[
                ("DOT_PROVIDER_BATCH_MODE", mode.to_owned()),
                (
                    "DOT_PROVIDER_BATCH_EVENTS",
                    state.events().to_string_lossy().into_owned(),
                ),
            ]),
        }),
    }
}

fn provider(state: &TempState, probe_mode: &str, install_mode: &str) -> Provider {
    provider_with_activation(state, probe_mode, install_mode, "yes")
}

fn provider_with_activation(
    state: &TempState,
    probe_mode: &str,
    install_mode: &str,
    active: &str,
) -> Provider {
    Provider {
        probe: helper_action::<StringExpressionSource>(probe_mode, state),
        activate: Some(EnvironmentPatch {
            path_prepend: None,
            path_append: None,
            variables: variables(&[("DOT_PROVIDER_BATCH_ACTIVE", active.to_owned())]),
        }),
        ensure: None,
        install: helper_action::<ProviderInstallArgSource>(install_mode, state),
    }
}

enum TestPackage<'a> {
    Single {
        id: &'a str,
        provider: &'a str,
    },
    Batch {
        id: &'a str,
        provider: &'a str,
        names: &'a [&'a str],
    },
}

fn plan_for(providers: Vec<(&str, Provider)>, packages: Vec<TestPackage<'_>>) -> ExecutionPlan {
    let providers = providers
        .into_iter()
        .map(|(id, provider)| (provider_id(id), provider))
        .collect::<Entries<_>>();
    let packages = packages
        .into_iter()
        .map(|package| match package {
            TestPackage::Single { id, provider } => (
                selector_id(id),
                Package::Provider(ProviderPackage::Single(SingleProviderPackage {
                    provider: provider_id(provider),
                    provider_args: None,
                })),
            ),
            TestPackage::Batch {
                id,
                provider,
                names,
            } => (
                selector_id(id),
                Package::Provider(ProviderPackage::Batch(BatchProviderPackage {
                    provider: provider_id(provider),
                    names: names.iter().map(|name| provider_id(name)).collect(),
                    provider_args: None,
                })),
            ),
        })
        .collect::<SelectableEntries<_>>();
    let config = Config {
        targets: BTreeMap::from([(
            selector_id("test"),
            Target {
                platform: PlatformConstraint {
                    os: OneOrMany::One(provider_id(env::consts::OS)),
                    arch: None,
                    distro: None,
                    distro_family: None,
                    environment: None,
                },
                providers,
                packages,
                links: BTreeMap::new(),
                actions: BTreeMap::new(),
                profiles: BTreeMap::new(),
            },
        )]),
    };
    let platform = PlatformInfo::detect();
    let manifest = EffectiveManifest::select_for_execution(&config, &platform, None, None)
        .expect("test manifest should select");
    let environment = ExecutionEnvironment::empty();
    let xdg = XdgPaths::detect();

    ExecutionPlanner::new(
        &environment,
        DotPaths::new(
            Path::new("/tmp/dot-provider-batch-test/dot.toml"),
            Path::new("/tmp/dot-provider-batch-test"),
            Path::new("/tmp"),
        ),
        &xdg,
        &platform,
    )
    .plan(&manifest, &JobSelection::All)
    .expect("provider install plan should build")
}

#[test]
fn install_uses_the_environment_from_one_ready_provider_status() {
    let state = TempState::new();
    let plan = plan_for(
        vec![("ready", provider(&state, "probe-ready", "install-ready"))],
        vec![TestPackage::Single {
            id: "tool",
            provider: "ready",
        }],
    );
    let provider = plan
        .providers()
        .next()
        .expect("planned provider should exist");
    let install = plan
        .provider_installs()
        .next()
        .expect("planned provider install should exist");
    let environment = ExecutionEnvironment::empty();
    let runner = ProviderRunner::new(&environment);
    let readiness = runner.ensure(provider);

    let status = runner.install(install, &readiness);

    assert_eq!(status.id(), install.id());
    assert!(status.is_succeeded());
    assert_eq!(state.recorded_events(), ["probe", "install-ready"]);
}

#[test]
fn install_rejects_a_status_from_a_different_provider_before_execution() {
    let provider_a = TempState::new();
    let provider_b = TempState::new();
    let plan = plan_for(
        vec![
            (
                "provider-a",
                provider_with_activation(
                    &provider_a,
                    "probe-provider-a",
                    "install-provider-a",
                    "provider-a",
                ),
            ),
            (
                "provider-b",
                provider_with_activation(
                    &provider_b,
                    "probe-provider-b",
                    "install-unexpected",
                    "provider-b",
                ),
            ),
        ],
        vec![TestPackage::Single {
            id: "tool",
            provider: "provider-a",
        }],
    );
    let planned_provider_a = plan
        .providers()
        .find(|provider| provider.id() == "provider-a")
        .expect("provider A should exist");
    let planned_provider_b = plan
        .providers()
        .find(|provider| provider.id() == "provider-b")
        .expect("provider B should exist");
    let install = plan
        .provider_installs()
        .next()
        .expect("provider A install should exist");
    let environment = ExecutionEnvironment::empty();
    let runner = ProviderRunner::new(&environment);
    let readiness_a = runner.ensure(planned_provider_a);
    let readiness_b = runner.ensure(planned_provider_b);
    assert!(readiness_a.is_ready());
    assert!(readiness_b.is_ready());

    let status = runner.install(install, &readiness_b);

    assert_eq!(status.id(), install.id());
    assert!(matches!(
        status.error(),
        Some(ProviderInstallError::ProviderMismatch { expected, actual })
            if expected == "provider-a" && actual == "provider-b"
    ));
    assert_eq!(provider_a.recorded_events(), ["probe-provider-a"]);
    assert_eq!(provider_b.recorded_events(), ["probe-provider-b"]);
}

#[test]
fn executes_single_packages_independently_with_the_activated_environment() {
    let state = TempState::new();
    let plan = plan_for(
        vec![("ready", provider(&state, "probe-ready", "install-ready"))],
        vec![
            TestPackage::Single {
                id: "alpha",
                provider: "ready",
            },
            TestPackage::Single {
                id: "beta",
                provider: "ready",
            },
        ],
    );
    let environment = ExecutionEnvironment::empty();
    let runner = ProviderRunner::new(&environment);
    let readiness = runner.ensure_all(plan.providers());

    let execution = runner.install_all(plan.provider_installs(), &readiness);

    assert!(execution.all_succeeded());
    assert_eq!(execution.statuses().len(), 2);
    assert_eq!(execution.statuses()[0].id(), "alpha");
    assert_eq!(execution.statuses()[1].id(), "beta");
    assert!(matches!(
        execution.statuses()[0].outcome(),
        Ok(ProviderInstallOutcome::Executed { install }) if install.code() == Some(0)
    ));
    assert_eq!(
        state.recorded_events(),
        ["probe", "install-ready", "install-ready"]
    );
}

#[test]
fn executes_one_named_batch_as_one_install_unit() {
    let state = TempState::new();
    let plan = plan_for(
        vec![("ready", provider(&state, "probe-ready", "install-ready"))],
        vec![TestPackage::Batch {
            id: "cli-tools",
            provider: "ready",
            names: &["bat", "fd", "fzf"],
        }],
    );
    let environment = ExecutionEnvironment::empty();
    let runner = ProviderRunner::new(&environment);
    let readiness = runner.ensure_all(plan.providers());

    let execution = runner.install_all(plan.provider_installs(), &readiness);

    assert!(execution.all_succeeded());
    assert_eq!(execution.statuses().len(), 1);
    assert_eq!(execution.statuses()[0].id(), "cli-tools");
    assert_eq!(state.recorded_events(), ["probe", "install-ready"]);
}

#[test]
fn blocks_each_install_unit_for_an_unavailable_provider() {
    let state = TempState::new();
    let plan = plan_for(
        vec![(
            "missing",
            provider(&state, "probe-missing", "install-unexpected"),
        )],
        vec![
            TestPackage::Single {
                id: "tool",
                provider: "missing",
            },
            TestPackage::Batch {
                id: "cli-tools",
                provider: "missing",
                names: &["bat", "fd"],
            },
        ],
    );
    let environment = ExecutionEnvironment::empty();
    let runner = ProviderRunner::new(&environment);
    let readiness = runner.ensure_all(plan.providers());

    let execution = runner.install_all(plan.provider_installs(), &readiness);

    assert!(!execution.all_succeeded());
    assert_eq!(execution.statuses().len(), 2);
    assert!(execution.statuses().iter().all(|status| matches!(
        status.outcome(),
        Ok(ProviderInstallOutcome::NotRunProviderUnavailable)
    )));
    assert_eq!(state.recorded_events(), ["probe-missing"]);
}

#[test]
fn a_failed_install_unit_does_not_stop_an_unrelated_unit() {
    let failed = TempState::new();
    let succeeded = TempState::new();
    let plan = plan_for(
        vec![
            ("a-failed", provider(&failed, "probe-ready", "install-fail")),
            (
                "b-succeeded",
                provider(&succeeded, "probe-ready", "install-ready"),
            ),
        ],
        vec![
            TestPackage::Single {
                id: "first",
                provider: "a-failed",
            },
            TestPackage::Single {
                id: "second",
                provider: "b-succeeded",
            },
        ],
    );
    let environment = ExecutionEnvironment::empty();
    let runner = ProviderRunner::new(&environment);
    let readiness = runner.ensure_all(plan.providers());

    let execution = runner.install_all(plan.provider_installs(), &readiness);

    assert_eq!(execution.statuses().len(), 2);
    let error = execution.statuses()[0]
        .error()
        .expect("failed install should retain its error");
    assert!(matches!(
        error,
        ProviderInstallError::UnsuccessfulExit { result } if result.code() == Some(23)
    ));
    assert!(error.source().is_none());
    assert!(matches!(
        execution.statuses()[1].outcome(),
        Ok(ProviderInstallOutcome::Executed { install }) if install.code() == Some(0)
    ));
    assert_eq!(failed.recorded_events(), ["probe", "install-fail"]);
    assert_eq!(succeeded.recorded_events(), ["probe", "install-ready"]);
}

#[test]
fn helper_process() {
    let Ok(mode) = env::var("DOT_PROVIDER_BATCH_MODE") else {
        return;
    };
    let events = PathBuf::from(
        env::var_os("DOT_PROVIDER_BATCH_EVENTS").expect("provider events path should be set"),
    );
    if mode == "install-provider-a" {
        record(&events, "install-provider-a-started");
    }
    let expected_active = match mode.as_str() {
        "probe-provider-a" | "install-provider-a" => "provider-a",
        "probe-provider-b" => "provider-b",
        _ => "yes",
    };
    assert_eq!(
        env::var("DOT_PROVIDER_BATCH_ACTIVE").as_deref(),
        Ok(expected_active),
        "provider activate should be present during probe and install"
    );

    match mode.as_str() {
        "probe-ready" => record(&events, "probe"),
        "probe-provider-a" => record(&events, "probe-provider-a"),
        "probe-provider-b" => record(&events, "probe-provider-b"),
        "probe-missing" => {
            record(&events, "probe-missing");
            process::exit(1);
        }
        "install-ready" => record(&events, "install-ready"),
        "install-provider-a" => record(&events, "install-provider-a"),
        "install-fail" => {
            record(&events, "install-fail");
            process::exit(23);
        }
        "install-unexpected" => panic!("unavailable provider install must not execute"),
        unknown => panic!("unknown provider install helper mode: {unknown}"),
    }
}

fn record(path: &Path, event: &str) {
    use std::io::Write as _;

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("event log should open");
    writeln!(file, "{event}").expect("event should be recorded");
}

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};

use dot::action::ExecutionEnvironment;
use dot::interpolation::{DotPaths, XdgPaths};
use dot::manifest::EffectiveManifest;
use dot::plan::{ExecutionPlan, ExecutionPlanner};
use dot::platform::PlatformInfo;
use dot::provider::{ProviderInstallError, ProviderInstallOutcome, ProviderRunner};
use dot::schema::{
    BatchProviderPackage, Config, Entries, EnvironmentName, EnvironmentPatch, ExecAction,
    Identifier, OneOrMany, Package, PlatformConstraint, Provider, ProviderInstallArgSource,
    ProviderPackage, SingleProviderPackage, StringExpressionSource, Target,
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

fn identifier(value: &str) -> Identifier {
    Identifier::new(value).expect("test identifier should be valid")
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
    Provider {
        probe: helper_action::<StringExpressionSource>(probe_mode, state),
        activate: Some(EnvironmentPatch {
            path_prepend: None,
            path_append: None,
            variables: variables(&[("DOT_PROVIDER_BATCH_ACTIVE", "yes".to_owned())]),
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
        .map(|(id, provider)| (identifier(id), provider))
        .collect::<Entries<_>>();
    let packages = packages
        .into_iter()
        .map(|package| match package {
            TestPackage::Single { id, provider } => (
                identifier(id),
                Package::Provider(ProviderPackage::Single(SingleProviderPackage {
                    provider: identifier(provider),
                    provider_args: None,
                })),
            ),
            TestPackage::Batch {
                id,
                provider,
                names,
            } => (
                identifier(id),
                Package::Provider(ProviderPackage::Batch(BatchProviderPackage {
                    provider: identifier(provider),
                    names: names.iter().map(|name| identifier(name)).collect(),
                    provider_args: None,
                })),
            ),
        })
        .collect::<Entries<_>>();
    let config = Config {
        targets: BTreeMap::from([(
            identifier("test"),
            Target {
                platform: PlatformConstraint {
                    os: OneOrMany::One(identifier(env::consts::OS)),
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
    let manifest = EffectiveManifest::select(&config, &platform, None, None)
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
    .plan(&manifest)
    .expect("provider install plan should build")
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
    assert!(matches!(
        execution.statuses()[0].error(),
        Some(ProviderInstallError::UnsuccessfulExit { result }) if result.code() == Some(23)
    ));
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
    assert_eq!(
        env::var("DOT_PROVIDER_BATCH_ACTIVE").as_deref(),
        Ok("yes"),
        "provider activate should be present during probe and install"
    );
    let events = PathBuf::from(
        env::var_os("DOT_PROVIDER_BATCH_EVENTS").expect("provider events path should be set"),
    );

    match mode.as_str() {
        "probe-ready" => record(&events, "probe"),
        "probe-missing" => {
            record(&events, "probe-missing");
            process::exit(1);
        }
        "install-ready" => record(&events, "install-ready"),
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

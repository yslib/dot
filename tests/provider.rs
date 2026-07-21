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
use dot::provider::{ProviderOutcome, ProviderRunner, ProviderStage};
use dot::schema::{
    Config, Entries, EnvironmentName, EnvironmentPatch, ExecAction, Identifier, OneOrMany,
    PlatformConstraint, Provider, ProviderInstallArg, ScalarTemplate, Target,
};

static NEXT_STATE: AtomicU64 = AtomicU64::new(0);

struct TempState {
    directory: PathBuf,
}

impl TempState {
    fn new() -> Self {
        let sequence = NEXT_STATE.fetch_add(1, Ordering::Relaxed);
        let directory = env::temp_dir().join(format!("dot-provider-{}-{sequence}", process::id()));
        fs::create_dir(&directory).expect("temporary state directory should be created");
        Self { directory }
    }

    fn marker(&self) -> PathBuf {
        self.directory.join("ready")
    }

    fn events(&self) -> PathBuf {
        self.directory.join("events")
    }

    fn missing_cwd(&self) -> PathBuf {
        self.directory.join("created-by-ensure")
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

fn variables(values: &[(&str, String)]) -> BTreeMap<EnvironmentName, ScalarTemplate> {
    values
        .iter()
        .map(|(name, value)| {
            (
                EnvironmentName::new(*name).expect("test environment name should be valid"),
                ScalarTemplate::from(value.clone()),
            )
        })
        .collect()
}

fn helper_action(mode: &str, state: &TempState) -> ExecAction {
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
                ("DOT_PROVIDER_MODE", mode.to_owned()),
                (
                    "DOT_PROVIDER_MARKER",
                    state.marker().to_string_lossy().into_owned(),
                ),
                (
                    "DOT_PROVIDER_EVENTS",
                    state.events().to_string_lossy().into_owned(),
                ),
                (
                    "DOT_PROVIDER_CWD",
                    state.missing_cwd().to_string_lossy().into_owned(),
                ),
            ]),
        }),
    }
}

fn provider(probe: ExecAction, ensure: Vec<ExecAction>) -> Provider {
    Provider {
        probe,
        activate: Some(EnvironmentPatch {
            path_prepend: None,
            path_append: None,
            variables: variables(&[("DOT_PROVIDER_ACTIVE", "yes".to_owned())]),
        }),
        ensure: (!ensure.is_empty()).then_some(OneOrMany::Many(ensure)),
        install: ExecAction::<ProviderInstallArg> {
            kind: None,
            program: "unused-install".into(),
            args: Vec::new(),
            cwd: None,
            env: None,
        },
    }
}

fn plan_for(providers: Vec<(&str, Provider)>) -> ExecutionPlan {
    let providers = providers
        .into_iter()
        .map(|(id, provider)| (identifier(id), provider))
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
                packages: BTreeMap::new(),
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
    let config_path = Path::new("/tmp/dot-provider-test/dot.toml");
    let config_dir = Path::new("/tmp/dot-provider-test");
    let cwd = Path::new("/tmp");

    ExecutionPlanner::new(
        &environment,
        DotPaths::new(config_path, config_dir, cwd),
        &xdg,
        &platform,
    )
    .plan(&manifest)
    .expect("provider plan should build")
}

#[test]
fn reports_an_already_ready_provider_without_running_ensure() {
    let state = TempState::new();
    fs::write(state.marker(), "ready").expect("ready marker should be written");
    let plan = plan_for(vec![(
        "ready",
        provider(
            helper_action("probe-state", &state),
            vec![helper_action("ensure-fail", &state)],
        ),
    )]);

    let readiness =
        ProviderRunner::new(&ExecutionEnvironment::empty()).ensure_all(plan.providers());
    let status = readiness
        .get("ready")
        .expect("provider status should exist");

    assert!(readiness.all_ready());
    assert!(status.is_ready());
    assert_eq!(
        status
            .environment()
            .expect("ready provider should retain its environment")
            .get("DOT_PROVIDER_ACTIVE"),
        Some(std::ffi::OsStr::new("yes"))
    );
    assert!(matches!(
        status.outcome(),
        Ok(ProviderOutcome::AlreadyReady { probe }) if probe.code() == Some(0)
    ));
    assert_eq!(state.recorded_events(), ["probe"]);
}

#[test]
fn runs_ensure_in_order_then_reapplies_activate_and_probes_again() {
    let state = TempState::new();
    let plan = plan_for(vec![(
        "missing",
        provider(
            helper_action("probe-state", &state),
            vec![
                helper_action("ensure-first", &state),
                helper_action("ensure-satisfy", &state),
            ],
        ),
    )]);

    let readiness =
        ProviderRunner::new(&ExecutionEnvironment::empty()).ensure_all(plan.providers());
    let status = readiness
        .get("missing")
        .expect("provider status should exist");

    assert!(status.is_ready());
    assert!(matches!(
        status.outcome(),
        Ok(ProviderOutcome::Ensured { ensure, probe })
            if ensure.len() == 2 && probe.code() == Some(0)
    ));
    assert_eq!(
        state.recorded_events(),
        ["probe", "ensure-first", "ensure-satisfy", "probe"]
    );
}

#[test]
fn reports_a_failed_probe_when_no_ensure_is_declared() {
    let state = TempState::new();
    let plan = plan_for(vec![(
        "missing",
        provider(helper_action("probe-state", &state), Vec::new()),
    )]);

    let readiness =
        ProviderRunner::new(&ExecutionEnvironment::empty()).ensure_all(plan.providers());
    let status = readiness
        .get("missing")
        .expect("provider status should exist");

    assert!(!readiness.all_ready());
    assert!(!status.is_ready());
    assert!(status.environment().is_none());
    let error = status.error().expect("provider should have an error");
    assert_eq!(error.stage(), ProviderStage::InitialProbe);
    assert_eq!(
        error.exit_result().and_then(|result| result.code()),
        Some(1)
    );
    assert_eq!(state.recorded_events(), ["probe"]);
}

#[test]
fn stops_the_ensure_list_at_the_first_failure() {
    let state = TempState::new();
    let plan = plan_for(vec![(
        "broken",
        provider(
            helper_action("probe-state", &state),
            vec![
                helper_action("ensure-first", &state),
                helper_action("ensure-fail", &state),
                helper_action("ensure-satisfy", &state),
            ],
        ),
    )]);

    let readiness =
        ProviderRunner::new(&ExecutionEnvironment::empty()).ensure_all(plan.providers());
    let status = readiness
        .get("broken")
        .expect("provider status should exist");
    let error = status.error().expect("provider should fail");

    assert_eq!(error.stage(), ProviderStage::Ensure(1));
    assert_eq!(
        error.exit_result().and_then(|result| result.code()),
        Some(19)
    );
    assert_eq!(
        state.recorded_events(),
        ["probe", "ensure-first", "ensure-fail"]
    );
}

#[test]
fn requires_the_final_probe_to_succeed() {
    let state = TempState::new();
    let plan = plan_for(vec![(
        "still-missing",
        provider(
            helper_action("probe-always-missing", &state),
            vec![helper_action("ensure-first", &state)],
        ),
    )]);

    let readiness =
        ProviderRunner::new(&ExecutionEnvironment::empty()).ensure_all(plan.providers());
    let status = readiness
        .get("still-missing")
        .expect("provider status should exist");
    let error = status.error().expect("provider should fail final probe");

    assert_eq!(error.stage(), ProviderStage::FinalProbe);
    assert_eq!(
        error.exit_result().and_then(|result| result.code()),
        Some(1)
    );
    assert_eq!(state.recorded_events(), ["probe", "ensure-first", "probe"]);
}

#[test]
fn one_provider_failure_does_not_stop_an_unrelated_provider() {
    let failed = TempState::new();
    let ready = TempState::new();
    fs::write(ready.marker(), "ready").expect("ready marker should be written");
    let plan = plan_for(vec![
        (
            "a-failed",
            provider(helper_action("probe-state", &failed), Vec::new()),
        ),
        (
            "b-ready",
            provider(helper_action("probe-state", &ready), Vec::new()),
        ),
    ]);

    let readiness =
        ProviderRunner::new(&ExecutionEnvironment::empty()).ensure_all(plan.providers());

    assert_eq!(readiness.statuses().len(), 2);
    assert!(!readiness.get("a-failed").expect("failed status").is_ready());
    assert!(readiness.get("b-ready").expect("ready status").is_ready());
    assert_eq!(failed.recorded_events(), ["probe"]);
    assert_eq!(ready.recorded_events(), ["probe"]);
}

#[test]
fn an_unstartable_initial_probe_can_be_repaired_by_ensure() {
    let state = TempState::new();
    let mut probe = helper_action("probe-ready", &state);
    probe.cwd = Some(state.missing_cwd().to_string_lossy().into_owned().into());
    let plan = plan_for(vec![(
        "repaired",
        provider(probe, vec![helper_action("ensure-create-cwd", &state)]),
    )]);

    let readiness =
        ProviderRunner::new(&ExecutionEnvironment::empty()).ensure_all(plan.providers());
    let status = readiness
        .get("repaired")
        .expect("provider status should exist");

    assert!(status.is_ready());
    assert!(matches!(
        status.outcome(),
        Ok(ProviderOutcome::Ensured { ensure, probe })
            if ensure.len() == 1 && probe.code() == Some(0)
    ));
    assert_eq!(state.recorded_events(), ["ensure-create-cwd", "probe"]);
}

#[test]
fn helper_process() {
    let Ok(mode) = env::var("DOT_PROVIDER_MODE") else {
        return;
    };
    assert_eq!(
        env::var("DOT_PROVIDER_ACTIVE").as_deref(),
        Ok("yes"),
        "provider activate should be applied"
    );
    let marker = PathBuf::from(
        env::var_os("DOT_PROVIDER_MARKER").expect("provider marker path should be set"),
    );
    let events = PathBuf::from(
        env::var_os("DOT_PROVIDER_EVENTS").expect("provider events path should be set"),
    );
    let created_cwd =
        PathBuf::from(env::var_os("DOT_PROVIDER_CWD").expect("provider cwd path should be set"));

    match mode.as_str() {
        "probe-state" => {
            record(&events, "probe");
            println!("ready={}", marker.exists());
            if !marker.exists() {
                process::exit(1);
            }
        }
        "probe-always-missing" => {
            record(&events, "probe");
            process::exit(1);
        }
        "probe-ready" => record(&events, "probe"),
        "ensure-first" => record(&events, "ensure-first"),
        "ensure-satisfy" => {
            record(&events, "ensure-satisfy");
            fs::write(marker, "ready").expect("ensure should write ready marker");
        }
        "ensure-fail" => {
            record(&events, "ensure-fail");
            process::exit(19);
        }
        "ensure-create-cwd" => {
            record(&events, "ensure-create-cwd");
            fs::create_dir(created_cwd).expect("ensure should create probe cwd");
        }
        unknown => panic!("unknown provider helper mode: {unknown}"),
    }
}

fn record(path: &Path, event: &str) {
    use std::io::Write as _;

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("provider event file should open");
    writeln!(file, "{event}").expect("provider event should be recorded");
}

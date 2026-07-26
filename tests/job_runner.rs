mod support;

use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};

use dot::action::ExecutionEnvironment;
use dot::interpolation::{DotPaths, XdgPaths};
use dot::job::{JobId, JobSelection, JobSelector};
use dot::job_runner::{BlockReason, JobOutcome, JobRunner, JobState};
use dot::link::{LinkOutcome, LinkPhaseError};
use dot::manifest::EffectiveManifest;
use dot::plan::{ExecutionPlan, ExecutionPlanner, PlannedJob};
use dot::platform::PlatformInfo;
use dot::provider::ProviderInstallOutcome;
use dot::schema::{Config, Identifier};
use support::fixture;

static NEXT_WORKSPACE: AtomicU64 = AtomicU64::new(0);

struct TempWorkspace {
    directory: PathBuf,
}

impl TempWorkspace {
    fn new() -> Self {
        let sequence = NEXT_WORKSPACE.fetch_add(1, Ordering::Relaxed);
        let directory =
            env::temp_dir().join(format!("dot-job-runner-{}-{sequence}", process::id()));
        fs::create_dir(&directory).expect("temporary workspace should be created");
        Self { directory }
    }

    fn write_source(&self, name: &str) -> PathBuf {
        let path = self.directory.join(name);
        fs::write(&path, name).expect("link source should be written");
        path
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

    fn path(&self, name: &str) -> PathBuf {
        self.directory.join(name)
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn identifier(value: &str) -> Identifier {
    Identifier::new(value).expect("test identifier should be valid")
}

fn helper_program_toml() -> String {
    format!(
        "{:?}",
        env::current_exe()
            .expect("test executable should have a path")
            .to_string_lossy()
    )
}

fn helper_exec(mode: &str) -> String {
    format!(
        r#"{{ program = __PROGRAM__, args = ["--exact", "helper_process", "--nocapture"], env = {{ variables = {{ DOT_JOB_HELPER = "{mode}", DOT_JOB_EVENTS = "${{dot:config_dir}}/events", DOT_JOB_LINK = "${{dot:config_dir}}/linked.txt" }} }} }}"#
    )
}

fn plan_fixture(
    workspace: &TempWorkspace,
    fixture_name: &str,
    replacements: &[(&str, String)],
) -> ExecutionPlan {
    let mut input = fixture::read(fixture_name).replace("__OS__", env::consts::OS);
    for (token, value) in replacements {
        input = input.replace(token, value);
    }
    input = input.replace("__PROGRAM__", &helper_program_toml());

    let config: Config = toml::from_str(&input).expect("test config should deserialize");
    let platform = PlatformInfo::detect();
    let manifest = EffectiveManifest::select(&config, &platform, Some("current"), None)
        .expect("test manifest should select");
    let environment = ExecutionEnvironment::empty();
    let xdg = XdgPaths::detect();
    let config_path = workspace.path("dot.toml");
    let dot_paths = DotPaths::new(&config_path, &workspace.directory, &workspace.directory);

    ExecutionPlanner::new(&environment, dot_paths, &xdg, &platform)
        .plan(&manifest)
        .expect("test execution plan should build")
}

fn serial_plan(workspace: &TempWorkspace, probe_mode: &str) -> ExecutionPlan {
    plan_fixture(
        workspace,
        "jobs/valid-serial-execution-template.toml",
        &[
            ("__PROBE__", helper_exec(probe_mode)),
            ("__INSTALL__", helper_exec("provider-install")),
            ("__MANUAL__", helper_exec("manual-install")),
            ("__ACTION__", helper_exec("action")),
        ],
    )
}

#[test]
fn runs_all_selected_jobs_in_stable_serial_order() {
    let workspace = TempWorkspace::new();
    let source = workspace.write_source("source.txt");
    let plan = serial_plan(&workspace, "probe");
    let selected = plan
        .select(&JobSelection::All)
        .expect("all jobs should select");
    let selected_ids = selected.jobs().map(PlannedJob::id).collect::<Vec<_>>();
    let environment = ExecutionEnvironment::empty();

    let report = JobRunner::new(&environment).run(&selected);

    assert!(report.all_succeeded());
    assert_eq!(
        workspace.recorded_events(),
        ["probe", "provider-install", "manual-install", "action"]
    );
    assert_eq!(
        selected_ids,
        [
            JobId::Provider(identifier("ready")),
            JobId::Package(identifier("provider-tool")),
            JobId::Package(identifier("manual-tool")),
            JobId::Action(identifier("configure")),
            JobId::Link(identifier("config")),
        ]
    );
    assert_eq!(report.len(), 5);
    assert!(report.link_phase_error().is_none());
    assert!(matches!(
        report.get(&JobId::Provider(identifier("ready"))),
        Some(JobState::Completed(JobOutcome::Provider(Ok(_))))
    ));
    assert!(matches!(
        report.get(&JobId::Package(identifier("provider-tool"))),
        Some(JobState::Completed(JobOutcome::ProviderPackage(Ok(
            ProviderInstallOutcome::Executed { .. }
        ))))
    ));
    assert!(matches!(
        report.get(&JobId::Package(identifier("manual-tool"))),
        Some(JobState::Completed(JobOutcome::ManualPackage(Ok(_))))
    ));
    assert!(matches!(
        report.get(&JobId::Action(identifier("configure"))),
        Some(JobState::Completed(JobOutcome::Action(Ok(_))))
    ));
    assert!(matches!(
        report.get(&JobId::Link(identifier("config"))),
        Some(JobState::Completed(JobOutcome::Link(Ok(
            LinkOutcome::Created
        ))))
    ));
    assert_eq!(
        fs::canonicalize(workspace.path("linked.txt")).expect("link should resolve"),
        fs::canonicalize(source).expect("source should resolve")
    );
}

#[test]
fn exact_provider_package_runs_only_its_provider_closure() {
    let workspace = TempWorkspace::new();
    workspace.write_source("source.txt");
    let plan = serial_plan(&workspace, "probe");
    let selected = plan
        .select(&JobSelection::only(JobSelector::Package(identifier(
            "provider-tool",
        ))))
        .expect("provider package should select");
    let environment = ExecutionEnvironment::empty();

    let report = JobRunner::new(&environment).run(&selected);

    assert!(report.all_succeeded());
    assert_eq!(workspace.recorded_events(), ["probe", "provider-install"]);
    assert_eq!(report.len(), 2);
    assert!(!workspace.path("linked.txt").exists());
}

#[test]
fn provider_failure_blocks_its_package_but_continues_unrelated_work() {
    let workspace = TempWorkspace::new();
    let source = workspace.write_source("source.txt");
    let plan = serial_plan(&workspace, "probe-fail");
    let selected = plan
        .select(&JobSelection::All)
        .expect("all jobs should select");
    let environment = ExecutionEnvironment::empty();

    let report = JobRunner::new(&environment).run(&selected);

    assert!(!report.all_succeeded());
    assert_eq!(
        workspace.recorded_events(),
        ["probe", "manual-install", "action"]
    );
    assert_eq!(report.len(), 5);
    assert!(report.link_phase_error().is_none());
    assert!(matches!(
        report.get(&JobId::Provider(identifier("ready"))),
        Some(JobState::Completed(JobOutcome::Provider(Err(_))))
    ));
    assert!(matches!(
        report.get(&JobId::Package(identifier("provider-tool"))),
        Some(JobState::Blocked(BlockReason::ProviderUnavailable { provider }))
            if provider.as_str() == "ready"
    ));
    assert!(matches!(
        report.get(&JobId::Package(identifier("manual-tool"))),
        Some(JobState::Completed(JobOutcome::ManualPackage(Ok(_))))
    ));
    assert!(matches!(
        report.get(&JobId::Action(identifier("configure"))),
        Some(JobState::Completed(JobOutcome::Action(Ok(_))))
    ));
    assert!(matches!(
        report.get(&JobId::Link(identifier("config"))),
        Some(JobState::Completed(JobOutcome::Link(Ok(
            LinkOutcome::Created
        ))))
    ));
    assert_eq!(
        fs::canonicalize(workspace.path("linked.txt")).expect("link should resolve"),
        fs::canonicalize(source).expect("source should resolve")
    );
}

#[test]
fn provider_package_failure_does_not_block_the_next_package_for_that_provider() {
    let workspace = TempWorkspace::new();
    let plan = plan_fixture(
        &workspace,
        "jobs/valid-provider-package-failure-continuation-template.toml",
        &[("__PROBE__", helper_exec("probe"))],
    );
    let selected = plan
        .select(&JobSelection::All)
        .expect("all jobs should select");
    let environment = ExecutionEnvironment::empty();

    let report = JobRunner::new(&environment).run(&selected);

    assert_eq!(
        workspace.recorded_events(),
        ["probe", "first-fail", "second-success"]
    );
    assert_eq!(report.len(), 3);
    assert!(!report.all_succeeded());
    assert!(matches!(
        report.get(&JobId::Package(identifier("first-tool"))),
        Some(JobState::Completed(JobOutcome::ProviderPackage(Err(_))))
    ));
    assert!(matches!(
        report.get(&JobId::Package(identifier("second-tool"))),
        Some(JobState::Completed(JobOutcome::ProviderPackage(Ok(
            ProviderInstallOutcome::Executed { .. }
        ))))
    ));
}

#[test]
fn duplicate_link_targets_block_the_complete_link_phase_before_mutation() {
    let workspace = TempWorkspace::new();
    workspace.write_source("first.txt");
    workspace.write_source("second.txt");
    let plan = plan_fixture(
        &workspace,
        "jobs/invalid-duplicate-link-target-template.toml",
        &[("__ACTION__", helper_exec("action"))],
    );
    let selected = plan
        .select(&JobSelection::All)
        .expect("all jobs should select");
    let environment = ExecutionEnvironment::empty();

    let report = JobRunner::new(&environment).run(&selected);

    assert!(!report.all_succeeded());
    assert_eq!(workspace.recorded_events(), ["action"]);
    assert_eq!(report.len(), 3);
    let error = report
        .link_phase_error()
        .expect("duplicate targets should fail the link phase");
    assert!(matches!(
        error,
        LinkPhaseError::DuplicateTarget { links, .. }
            if links == &[String::from("first"), String::from("second")]
    ));
    assert!(matches!(
        report.get(&JobId::Action(identifier("configure"))),
        Some(JobState::Completed(JobOutcome::Action(Ok(_))))
    ));
    for link in ["first", "second"] {
        assert!(matches!(
            report.get(&JobId::Link(identifier(link))),
            Some(JobState::Blocked(BlockReason::LinkPhase { message }))
                if message == &error.to_string()
        ));
    }
    assert!(!workspace.path("linked.txt").exists());
}

#[test]
fn helper_process() {
    let Ok(mode) = env::var("DOT_JOB_HELPER") else {
        return;
    };
    let events =
        PathBuf::from(env::var_os("DOT_JOB_EVENTS").expect("job event path should be present"));

    if matches!(mode.as_str(), "probe" | "probe-fail" | "provider-install") {
        assert_eq!(
            env::var("DOT_JOB_PROVIDER_ACTIVE").as_deref(),
            Ok("yes"),
            "provider work should receive activation environment"
        );
    } else {
        assert!(
            env::var_os("DOT_JOB_PROVIDER_ACTIVE").is_none(),
            "manual package and action environments must not receive provider activation"
        );
    }

    match mode.as_str() {
        "probe" | "provider-install" | "manual-install" => record(&events, &mode),
        "probe-fail" => {
            record(&events, "probe");
            process::exit(1);
        }
        "action" => {
            let link = PathBuf::from(
                env::var_os("DOT_JOB_LINK").expect("job link path should be present"),
            );
            assert!(!link.exists(), "links must run after ordinary jobs");
            record(&events, "action");
        }
        unknown => panic!("unknown job helper mode: {unknown}"),
    }
}

#[test]
fn provider_package_first_fails() {
    let Some(events) = provider_package_events() else {
        return;
    };
    assert_eq!(
        env::var("DOT_JOB_PROVIDER_ACTIVE").as_deref(),
        Ok("yes"),
        "provider install should receive activation environment"
    );
    record(&events, "first-fail");
    process::exit(1);
}

#[test]
fn provider_package_second_succeeds() {
    let Some(events) = provider_package_events() else {
        return;
    };
    assert_eq!(
        env::var("DOT_JOB_PROVIDER_ACTIVE").as_deref(),
        Ok("yes"),
        "provider install should receive activation environment"
    );
    record(&events, "second-success");
}

fn provider_package_events() -> Option<PathBuf> {
    if env::var("DOT_JOB_PROVIDER_PACKAGE_HELPER").as_deref() != Ok("yes") {
        return None;
    }
    Some(PathBuf::from(
        env::var_os("DOT_JOB_EVENTS").expect("provider package event path should be present"),
    ))
}

fn record(path: &Path, event: &str) {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("event log should open");
    writeln!(file, "{event}").expect("event should be recorded");
}

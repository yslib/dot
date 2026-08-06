mod support;

use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};

use dot::interpolation::{DotPaths, ExecutionEnvironment, XdgPaths};
use dot::job::{JobId, JobSelection, JobSelector};
use dot::manifest::EffectiveManifest;
use dot::native::job_execution::{ActionOutcome, BlockReason, JobOutcome, JobRunner, JobState};
use dot::native::link::{LinkOutcome, LinkPhaseError};
use dot::native::plan::{ExecutionPlan, ExecutionPlanner, PlannedJob};
use dot::native::provider::ProviderInstallOutcome;
use dot::platform::PlatformInfo;
use dot::schema::{Config, Identifier, SelectorIdentifier};
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

fn provider_id(value: &str) -> Identifier {
    Identifier::new(value).expect("test identifier should be valid")
}

fn selector_id(value: &str) -> SelectorIdentifier {
    SelectorIdentifier::new(value).expect("test selector identifier should be valid")
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
    selection: &JobSelection,
) -> ExecutionPlan {
    let mut input = fixture::read(fixture_name).replace("__OS__", env::consts::OS);
    for (token, value) in replacements {
        input = input.replace(token, value);
    }
    input = input.replace("__PROGRAM__", &helper_program_toml());

    let config: Config = toml::from_str(&input).expect("test config should deserialize");
    let platform = PlatformInfo::detect();
    let target = selector_id("current");
    let manifest = EffectiveManifest::select_for_execution(&config, &platform, Some(&target), None)
        .expect("test manifest should select");
    let environment = ExecutionEnvironment::empty();
    let xdg = XdgPaths::detect();
    let config_path = workspace.path("dot.toml");
    let dot_paths = DotPaths::new(
        &config_path,
        &workspace.directory,
        &config_path,
        &workspace.directory,
        &workspace.directory,
    );

    ExecutionPlanner::new(&environment, dot_paths, &xdg, &platform)
        .plan(&manifest, selection)
        .expect("test execution plan should build")
}

fn serial_plan(
    workspace: &TempWorkspace,
    probe_mode: &str,
    selection: &JobSelection,
) -> ExecutionPlan {
    plan_fixture(
        workspace,
        "jobs/valid-serial-execution-template.toml",
        &[
            ("__PROBE__", helper_exec(probe_mode)),
            ("__INSTALL__", helper_exec("provider-install")),
            ("__MANUAL__", helper_exec("manual-install")),
            ("__ACTION__", helper_exec("action")),
        ],
        selection,
    )
}

#[test]
fn runs_all_selected_jobs_in_stable_serial_order() {
    let workspace = TempWorkspace::new();
    let source = workspace.write_source("source.txt");
    let plan = serial_plan(&workspace, "probe", &JobSelection::All);
    let selected_ids = plan.jobs().iter().map(PlannedJob::id).collect::<Vec<_>>();
    let environment = ExecutionEnvironment::empty();

    let report = JobRunner::new(&environment).run(&plan);

    assert!(report.all_succeeded());
    assert_eq!(
        workspace.recorded_events(),
        ["probe", "manual-install", "provider-install", "action"]
    );
    assert_eq!(
        selected_ids,
        [
            JobId::Provider(provider_id("ready")),
            JobId::Package(selector_id("manual-tool")),
            JobId::Package(selector_id("provider-tool")),
            JobId::Action(selector_id("configure")),
            JobId::Link(selector_id("config")),
        ]
    );
    assert_eq!(report.len(), 5);
    assert!(report.link_phase_error().is_none());
    assert!(matches!(
        report.get(&JobId::Provider(provider_id("ready"))),
        Some(JobState::Completed(JobOutcome::Provider(Ok(_))))
    ));
    assert!(matches!(
        report.get(&JobId::Package(selector_id("provider-tool"))),
        Some(JobState::Completed(JobOutcome::ProviderPackage(Ok(
            ProviderInstallOutcome::Executed { .. }
        ))))
    ));
    assert!(matches!(
        report.get(&JobId::Package(selector_id("manual-tool"))),
        Some(JobState::Completed(JobOutcome::ManualPackage(Ok(_))))
    ));
    assert!(matches!(
        report.get(&JobId::Action(selector_id("configure"))),
        Some(JobState::Completed(JobOutcome::Action(
            ActionOutcome::Command(Ok(_))
        )))
    ));
    assert!(matches!(
        report.get(&JobId::Link(selector_id("config"))),
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
    let plan = serial_plan(
        &workspace,
        "probe",
        &JobSelection::only(JobSelector::Package(selector_id("provider-tool"))),
    );
    let environment = ExecutionEnvironment::empty();

    let report = JobRunner::new(&environment).run(&plan);

    assert!(report.all_succeeded());
    assert_eq!(workspace.recorded_events(), ["probe", "provider-install"]);
    assert_eq!(report.len(), 2);
    assert!(!workspace.path("linked.txt").exists());
}

#[test]
fn provider_failure_blocks_its_package_but_continues_unrelated_work() {
    let workspace = TempWorkspace::new();
    let source = workspace.write_source("source.txt");
    let plan = serial_plan(&workspace, "probe-fail", &JobSelection::All);
    let environment = ExecutionEnvironment::empty();

    let report = JobRunner::new(&environment).run(&plan);

    assert!(!report.all_succeeded());
    assert_eq!(
        workspace.recorded_events(),
        ["probe", "manual-install", "action"]
    );
    assert_eq!(report.len(), 5);
    assert!(report.link_phase_error().is_none());
    assert!(matches!(
        report.get(&JobId::Provider(provider_id("ready"))),
        Some(JobState::Completed(JobOutcome::Provider(Err(_))))
    ));
    assert!(matches!(
        report.get(&JobId::Package(selector_id("provider-tool"))),
        Some(JobState::Blocked(BlockReason::ProviderUnavailable { provider }))
            if provider.as_str() == "ready"
    ));
    assert!(matches!(
        report.get(&JobId::Package(selector_id("manual-tool"))),
        Some(JobState::Completed(JobOutcome::ManualPackage(Ok(_))))
    ));
    assert!(matches!(
        report.get(&JobId::Action(selector_id("configure"))),
        Some(JobState::Completed(JobOutcome::Action(
            ActionOutcome::Command(Ok(_))
        )))
    ));
    assert!(matches!(
        report.get(&JobId::Link(selector_id("config"))),
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
        &JobSelection::All,
    );
    let environment = ExecutionEnvironment::empty();

    let report = JobRunner::new(&environment).run(&plan);

    assert_eq!(
        workspace.recorded_events(),
        ["probe", "first-fail", "second-success"]
    );
    assert_eq!(report.len(), 3);
    assert!(!report.all_succeeded());
    assert!(matches!(
        report.get(&JobId::Package(selector_id("zulu-tool"))),
        Some(JobState::Completed(JobOutcome::ProviderPackage(Err(_))))
    ));
    assert!(matches!(
        report.get(&JobId::Package(selector_id("alpha-tool"))),
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
        &JobSelection::All,
    );
    let environment = ExecutionEnvironment::empty();

    let report = JobRunner::new(&environment).run(&plan);

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
        report.get(&JobId::Action(selector_id("configure"))),
        Some(JobState::Completed(JobOutcome::Action(
            ActionOutcome::Command(Ok(_))
        )))
    ));
    for link in ["first", "second"] {
        assert!(matches!(
            report.get(&JobId::Link(selector_id(link))),
            Some(JobState::Blocked(BlockReason::LinkPhase { message }))
                if message == &error.to_string()
        ));
    }
    assert!(!workspace.path("linked.txt").exists());
}

#[test]
fn fetch_failure_does_not_stop_later_commands_or_the_link_phase() {
    let workspace = TempWorkspace::new();
    let source = workspace.write_source("source.txt");
    fs::create_dir(workspace.path("fetch-target"))
        .expect("directory target should force an offline preflight failure");
    let plan = plan_fixture(
        &workspace,
        "jobs/valid-fetch-failure-continuation-template.toml",
        &[
            ("__BEFORE__", helper_exec("before")),
            ("__AFTER__", helper_exec("after")),
        ],
        &JobSelection::All,
    );
    let environment = ExecutionEnvironment::empty();

    let report = JobRunner::new(&environment).run(&plan);

    assert!(!report.all_succeeded());
    assert_eq!(workspace.recorded_events(), ["before", "after"]);
    assert!(matches!(
        report.get(&JobId::Action(selector_id("a-before"))),
        Some(JobState::Completed(JobOutcome::Action(
            ActionOutcome::Command(Ok(_))
        )))
    ));
    assert!(matches!(
        report.get(&JobId::Action(selector_id("b-fetch"))),
        Some(JobState::Completed(JobOutcome::Action(
            ActionOutcome::FetchContent(Err(_))
        )))
    ));
    assert!(matches!(
        report.get(&JobId::Action(selector_id("c-after"))),
        Some(JobState::Completed(JobOutcome::Action(
            ActionOutcome::Command(Ok(_))
        )))
    ));
    assert!(matches!(
        report.get(&JobId::Link(selector_id("config"))),
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
fn exact_fetch_selection_runs_only_the_selected_fetch_action() {
    let workspace = TempWorkspace::new();
    workspace.write_source("source.txt");
    fs::create_dir(workspace.path("fetch-target"))
        .expect("directory target should force an offline preflight failure");
    let plan = plan_fixture(
        &workspace,
        "jobs/valid-fetch-failure-continuation-template.toml",
        &[
            ("__BEFORE__", helper_exec("before")),
            ("__AFTER__", helper_exec("after")),
        ],
        &JobSelection::only(JobSelector::Action(selector_id("b-fetch"))),
    );
    let environment = ExecutionEnvironment::empty();

    let report = JobRunner::new(&environment).run(&plan);

    assert_eq!(report.len(), 1);
    assert!(!report.all_succeeded());
    assert!(workspace.recorded_events().is_empty());
    assert!(!workspace.path("linked.txt").exists());
    assert!(matches!(
        report.get(&JobId::Action(selector_id("b-fetch"))),
        Some(JobState::Completed(JobOutcome::Action(
            ActionOutcome::FetchContent(Err(_))
        )))
    ));
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
        "probe" | "provider-install" | "manual-install" | "before" | "after" => {
            record(&events, &mode)
        }
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

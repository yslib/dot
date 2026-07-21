use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};

use dot::action::ExecutionEnvironment;
use dot::interpolation::{DotPaths, XdgPaths};
use dot::link::{self, LinkError, LinkOutcome, LinkPhaseError};
use dot::manifest::EffectiveManifest;
use dot::plan::{ExecutionPlan, ExecutionPlanner};
use dot::platform::PlatformInfo;
use dot::schema::Config;

static NEXT_WORKSPACE: AtomicU64 = AtomicU64::new(0);

struct TempWorkspace(PathBuf);

impl TempWorkspace {
    fn new() -> Self {
        let sequence = NEXT_WORKSPACE.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!("dot-link-test-{}-{sequence}", process::id()));
        fs::create_dir_all(&path).expect("temporary workspace should be created");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn toml_path(path: &Path) -> String {
    format!("{:?}", path.to_string_lossy())
}

fn plan_with_link(
    workspace: &TempWorkspace,
    source: &Path,
    target: &Path,
    options: &str,
) -> ExecutionPlan {
    let declarations = format!(
        r#"
        [targets.machine.links.config]
        source = {}
        target = {}
        {}
        "#,
        toml_path(source),
        toml_path(target),
        options,
    );
    plan_with_links(workspace, &declarations)
}

fn plan_with_links(workspace: &TempWorkspace, declarations: &str) -> ExecutionPlan {
    let input = format!(
        r#"
        [targets.machine]
        platform = {{ os = {:?} }}

        {}
        "#,
        env::consts::OS,
        declarations,
    );
    let config: Config = toml::from_str(&input).expect("test config should deserialize");
    let platform = PlatformInfo::detect();
    let manifest = EffectiveManifest::select(&config, &platform, Some("machine"), None)
        .expect("test manifest should select");
    let environment = ExecutionEnvironment::capture();
    let xdg = XdgPaths::detect();
    let config_path = workspace.path().join("dot.toml");
    let dot_paths = DotPaths::new(&config_path, workspace.path(), workspace.path());

    ExecutionPlanner::new(&environment, dot_paths, &xdg, &platform)
        .plan(&manifest)
        .expect("test execution plan should build")
}

#[test]
fn creates_a_missing_file_link() {
    let workspace = TempWorkspace::new();
    let source = workspace.path().join("source.txt");
    let target = workspace.path().join("target.txt");
    fs::write(&source, "managed").expect("source should be written");
    let plan = plan_with_link(&workspace, &source, &target, "");

    let report = link::reconcile(plan.links()).expect("link phase should start");

    assert!(report.all_succeeded());
    assert_eq!(report.results().len(), 1);
    assert_eq!(report.results()[0].id(), "config");
    assert_eq!(
        report.results()[0].outcome().expect("link should succeed"),
        LinkOutcome::Created
    );
    assert_eq!(
        fs::canonicalize(&target).expect("target should resolve"),
        fs::canonicalize(&source).expect("source should resolve")
    );
}

#[test]
fn recognizes_an_existing_desired_link_as_satisfied() {
    let workspace = TempWorkspace::new();
    let source = workspace.path().join("source.txt");
    let target = workspace.path().join("target.txt");
    fs::write(&source, "managed").expect("source should be written");
    let plan = plan_with_link(&workspace, &source, &target, "");
    link::reconcile(plan.links()).expect("first link phase should start");

    let report = link::reconcile(plan.links()).expect("second link phase should start");

    assert_eq!(
        report.results()[0].outcome().expect("link should succeed"),
        LinkOutcome::Satisfied
    );
}

#[test]
fn creates_missing_parent_directories_by_default() {
    let workspace = TempWorkspace::new();
    let source = workspace.path().join("source.txt");
    let target = workspace.path().join("nested/config/target.txt");
    fs::write(&source, "managed").expect("source should be written");
    let plan = plan_with_link(&workspace, &source, &target, "");

    let report = link::reconcile(plan.links()).expect("link phase should start");

    assert_eq!(
        report.results()[0].outcome().expect("link should succeed"),
        LinkOutcome::Created
    );
    assert!(target.parent().expect("target has a parent").is_dir());
}

#[test]
fn skips_a_link_when_its_parent_is_missing_and_policy_is_skip() {
    let workspace = TempWorkspace::new();
    let source = workspace.path().join("source.txt");
    let target = workspace.path().join("missing/target.txt");
    fs::write(&source, "managed").expect("source should be written");
    let plan = plan_with_link(
        &workspace,
        &source,
        &target,
        r#"on_missing_parent = "skip""#,
    );

    let report = link::reconcile(plan.links()).expect("link phase should start");

    assert_eq!(
        report.results()[0]
            .outcome()
            .expect("skip should be successful"),
        LinkOutcome::SkippedMissingParent
    );
    assert!(!target.parent().expect("target has a parent").exists());
    assert!(!target.exists());
}

#[test]
fn creates_a_native_directory_link() {
    let workspace = TempWorkspace::new();
    let source = workspace.path().join("source-dir");
    let target = workspace.path().join("target-dir");
    fs::create_dir(&source).expect("source directory should be created");
    let plan = plan_with_link(&workspace, &source, &target, "");

    let report = link::reconcile(plan.links()).expect("link phase should start");

    assert_eq!(
        report.results()[0].outcome().expect("link should succeed"),
        LinkOutcome::Created
    );
    assert_eq!(
        fs::canonicalize(&target).expect("target should resolve"),
        fs::canonicalize(&source).expect("source should resolve")
    );
}

#[test]
fn replaces_an_incorrect_symbolic_link_by_default() {
    let workspace = TempWorkspace::new();
    let desired = workspace.path().join("desired.txt");
    let previous = workspace.path().join("previous.txt");
    let target = workspace.path().join("target.txt");
    fs::write(&desired, "desired").expect("desired source should be written");
    fs::write(&previous, "previous").expect("previous source should be written");
    let previous_plan = plan_with_link(&workspace, &previous, &target, "");
    link::reconcile(previous_plan.links()).expect("initial link phase should start");
    let desired_plan = plan_with_link(&workspace, &desired, &target, "");

    let report = link::reconcile(desired_plan.links()).expect("replacement phase should start");

    assert_eq!(
        report.results()[0]
            .outcome()
            .expect("replacement should succeed"),
        LinkOutcome::Replaced
    );
    assert_eq!(
        fs::canonicalize(&target).expect("target should resolve"),
        fs::canonicalize(&desired).expect("desired source should resolve")
    );
}

#[test]
fn replaces_a_broken_symbolic_link() {
    let workspace = TempWorkspace::new();
    let desired = workspace.path().join("desired.txt");
    let removed = workspace.path().join("removed.txt");
    let target = workspace.path().join("target.txt");
    fs::write(&desired, "desired").expect("desired source should be written");
    fs::write(&removed, "temporary").expect("temporary source should be written");
    let previous_plan = plan_with_link(&workspace, &removed, &target, "");
    link::reconcile(previous_plan.links()).expect("initial link phase should start");
    fs::remove_file(&removed).expect("previous source should be removed");
    let desired_plan = plan_with_link(&workspace, &desired, &target, "");

    let report = link::reconcile(desired_plan.links()).expect("replacement phase should start");

    assert_eq!(
        report.results()[0]
            .outcome()
            .expect("broken link should be replaced"),
        LinkOutcome::Replaced
    );
    assert_eq!(
        fs::canonicalize(&target).expect("target should resolve"),
        fs::canonicalize(&desired).expect("desired source should resolve")
    );
}

#[test]
fn conflict_error_preserves_an_incorrect_symbolic_link() {
    let workspace = TempWorkspace::new();
    let desired = workspace.path().join("desired.txt");
    let previous = workspace.path().join("previous.txt");
    let target = workspace.path().join("target.txt");
    fs::write(&desired, "desired").expect("desired source should be written");
    fs::write(&previous, "previous").expect("previous source should be written");
    let previous_plan = plan_with_link(&workspace, &previous, &target, "");
    link::reconcile(previous_plan.links()).expect("initial link phase should start");
    let desired_plan = plan_with_link(&workspace, &desired, &target, r#"on_conflict = "error""#);

    let report = link::reconcile(desired_plan.links()).expect("conflict phase should start");

    assert!(!report.all_succeeded());
    assert!(matches!(
        report.results()[0].outcome(),
        Err(LinkError::Conflict { .. })
    ));
    assert_eq!(
        fs::canonicalize(&target).expect("target should still resolve"),
        fs::canonicalize(&previous).expect("previous source should resolve")
    );
}

#[test]
fn never_replaces_an_existing_regular_file() {
    let workspace = TempWorkspace::new();
    let source = workspace.path().join("source.txt");
    let target = workspace.path().join("target.txt");
    fs::write(&source, "managed").expect("source should be written");
    fs::write(&target, "user data").expect("target should be written");
    let plan = plan_with_link(&workspace, &source, &target, "");

    let report = link::reconcile(plan.links()).expect("link phase should start");

    assert!(matches!(
        report.results()[0].outcome(),
        Err(LinkError::ExistingNonLink { .. })
    ));
    assert_eq!(
        fs::read_to_string(&target).expect("target should remain readable"),
        "user data"
    );
}

#[test]
fn recognizes_a_relative_symbolic_link_to_the_desired_source() {
    let workspace = TempWorkspace::new();
    let source = workspace.path().join("source.txt");
    let target = workspace.path().join("target.txt");
    fs::write(&source, "managed").expect("source should be written");
    create_relative_file_symlink(Path::new("source.txt"), &target)
        .expect("relative link should be created");
    let plan = plan_with_link(&workspace, &source, &target, "");

    let report = link::reconcile(plan.links()).expect("link phase should start");

    assert_eq!(
        report.results()[0]
            .outcome()
            .expect("relative link should be satisfied"),
        LinkOutcome::Satisfied
    );
}

#[test]
fn rejects_duplicate_targets_before_creating_any_link() {
    let workspace = TempWorkspace::new();
    let first_source = workspace.path().join("first.txt");
    let second_source = workspace.path().join("second.txt");
    let target = workspace.path().join("target.txt");
    fs::write(&first_source, "first").expect("first source should be written");
    fs::write(&second_source, "second").expect("second source should be written");
    let aliased_target = workspace.path().join("future/../target.txt");
    let declarations = format!(
        r#"
        [targets.machine.links.first]
        source = {}
        target = {}

        [targets.machine.links.second]
        source = {}
        target = {}
        "#,
        toml_path(&first_source),
        toml_path(&aliased_target),
        toml_path(&second_source),
        toml_path(&target),
    );
    let plan = plan_with_links(&workspace, &declarations);

    let error = link::reconcile(plan.links()).expect_err("duplicate targets must fail preflight");

    assert!(matches!(
        error,
        LinkPhaseError::DuplicateTarget { links, .. }
            if links == [String::from("first"), String::from("second")]
    ));
    assert!(!target.exists());
}

#[test]
fn duplicate_target_preflight_does_not_depend_on_source_validity() {
    let workspace = TempWorkspace::new();
    let missing_source = workspace.path().join("missing.txt");
    let valid_source = workspace.path().join("valid.txt");
    let target = workspace.path().join("target.txt");
    fs::write(&valid_source, "valid").expect("valid source should be written");
    let declarations = format!(
        r#"
        [targets.machine.links.missing]
        source = {}
        target = {}

        [targets.machine.links.valid]
        source = {}
        target = {}
        "#,
        toml_path(&missing_source),
        toml_path(&target),
        toml_path(&valid_source),
        toml_path(&target),
    );
    let plan = plan_with_links(&workspace, &declarations);

    let error = link::reconcile(plan.links()).expect_err("duplicate targets must fail preflight");

    assert!(matches!(error, LinkPhaseError::DuplicateTarget { .. }));
    assert!(!target.exists());
}

#[test]
fn one_link_failure_does_not_stop_an_unrelated_link() {
    let workspace = TempWorkspace::new();
    let missing_source = workspace.path().join("missing.txt");
    let valid_source = workspace.path().join("valid.txt");
    let missing_target = workspace.path().join("missing-target.txt");
    let valid_target = workspace.path().join("valid-target.txt");
    fs::write(&valid_source, "valid").expect("valid source should be written");
    let declarations = format!(
        r#"
        [targets.machine.links.missing]
        source = {}
        target = {}

        [targets.machine.links.valid]
        source = {}
        target = {}
        "#,
        toml_path(&missing_source),
        toml_path(&missing_target),
        toml_path(&valid_source),
        toml_path(&valid_target),
    );
    let plan = plan_with_links(&workspace, &declarations);

    let report = link::reconcile(plan.links()).expect("link phase should start");

    assert!(!report.all_succeeded());
    assert!(report.results()[0].outcome().is_err());
    assert_eq!(
        report.results()[1]
            .outcome()
            .expect("valid link should still succeed"),
        LinkOutcome::Created
    );
    assert!(valid_target.exists());
}

#[cfg(unix)]
fn create_relative_file_symlink(source: &Path, target: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(source, target)
}

#[cfg(windows)]
fn create_relative_file_symlink(source: &Path, target: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(source, target)
}

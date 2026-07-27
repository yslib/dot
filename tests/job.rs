mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use dot::action::ExecutionEnvironment;
use dot::interpolation::{DotPaths, XdgPaths};
use dot::job::{JobId, JobKind, JobSelection, JobSelector};
use dot::manifest::EffectiveManifest;
use dot::plan::{ExecutionPlan, ExecutionPlanner, JobSelectionError, PlannedJob};
use dot::platform::PlatformInfo;
use dot::schema::{
    Config, EnvironmentName, Identifier, ResolvedEnvironmentPatch, ResolvedString,
    SelectorIdentifier,
};
use support::fixture;

#[cfg(not(windows))]
const TEST_CONFIG: &str = "/repo/dot.toml";
#[cfg(windows)]
const TEST_CONFIG: &str = r"C:\repo\dot.toml";
#[cfg(not(windows))]
const TEST_CONFIG_DIR: &str = "/repo";
#[cfg(windows)]
const TEST_CONFIG_DIR: &str = r"C:\repo";
#[cfg(not(windows))]
const TEST_CWD: &str = "/work";
#[cfg(windows)]
const TEST_CWD: &str = r"C:\work";
#[cfg(not(windows))]
const TEST_HOME: &str = "/home/tester";
#[cfg(windows)]
const TEST_HOME: &str = r"C:\Users\tester";

fn provider_id(value: &str) -> Identifier {
    Identifier::new(value).expect("test identifier should be valid")
}

fn selector_id(value: &str) -> SelectorIdentifier {
    SelectorIdentifier::new(value).expect("test selector identifier should be valid")
}

fn plan_fixture() -> ExecutionPlan {
    let input = fixture::read("dry-run/valid-human-readable-plan.toml");
    let config: Config = toml::from_str(&input).expect("test config should deserialize");
    let platform = PlatformInfo {
        os: "linux".into(),
        arch: "x86_64".into(),
        distro: Some("test".into()),
        distro_families: BTreeSet::new(),
        environments: BTreeSet::from(["native".into()]),
    };
    let manifest = EffectiveManifest::select(&config, &platform, Some("machine"), None)
        .expect("test manifest should select");
    let mut environment = ExecutionEnvironment::empty();
    environment
        .apply_patch(&ResolvedEnvironmentPatch {
            path_prepend: None,
            path_append: None,
            variables: BTreeMap::from([
                (
                    EnvironmentName::new("HOME").expect("test name should be valid"),
                    ResolvedString::from(TEST_HOME),
                ),
                (
                    EnvironmentName::new("ROOT").expect("test name should be valid"),
                    ResolvedString::from("/opt"),
                ),
                (
                    EnvironmentName::new("RUNNER").expect("test name should be valid"),
                    ResolvedString::from("bash"),
                ),
            ]),
        })
        .expect("test environment should be valid");
    let xdg = XdgPaths::detect();
    let dot_paths = DotPaths::new(
        Path::new(TEST_CONFIG),
        Path::new(TEST_CONFIG_DIR),
        Path::new(TEST_CWD),
    );

    ExecutionPlanner::new(&environment, dot_paths, &xdg, &platform)
        .plan(&manifest)
        .expect("execution should plan")
}

#[test]
fn job_identity_is_scoped_by_kind() {
    let package = JobId::Package(selector_id("shared"));
    let action = JobId::Action(selector_id("shared"));
    let link = JobId::Link(selector_id("shared"));

    assert_ne!(package, action);
    assert_ne!(action, link);
    assert_eq!(package.kind(), JobKind::Package);
    assert_eq!(package.name(), "shared");
}

#[test]
fn exact_selection_keeps_its_typed_selector() {
    let selection = JobSelection::only(JobSelector::Package(selector_id("cli-tools")));

    assert!(matches!(
        selection,
        JobSelection::Only(ref selectors)
            if selectors.contains(&JobSelector::Package(selector_id("cli-tools")))
    ));
}

#[test]
fn selecting_provider_package_adds_only_its_provider() {
    let plan = plan_fixture();
    let selected = plan
        .select(&JobSelection::only(JobSelector::Package(selector_id(
            "alpha",
        ))))
        .expect("package should select");

    assert_eq!(
        selected.jobs().map(PlannedJob::id).collect::<Vec<_>>(),
        [
            JobId::Provider(provider_id("system")),
            JobId::Package(selector_id("alpha")),
        ]
    );
}

#[test]
fn selecting_manual_action_or_link_adds_no_provider() {
    let plan = plan_fixture();
    for (selector, expected) in [
        (
            JobSelector::Package(selector_id("manual")),
            JobId::Package(selector_id("manual")),
        ),
        (
            JobSelector::Action(selector_id("configure")),
            JobId::Action(selector_id("configure")),
        ),
        (
            JobSelector::Link(selector_id("gitconfig")),
            JobId::Link(selector_id("gitconfig")),
        ),
    ] {
        let selected = plan
            .select(&JobSelection::only(selector))
            .expect("job should select");
        assert_eq!(
            selected.jobs().map(PlannedJob::id).collect::<Vec<_>>(),
            [expected]
        );
    }
}

#[test]
fn unknown_typed_selector_fails_before_execution() {
    let plan = plan_fixture();
    let error = plan
        .select(&JobSelection::only(JobSelector::Action(selector_id(
            "missing",
        ))))
        .expect_err("unknown action should fail");

    assert!(matches!(
        error,
        JobSelectionError::Unknown(JobSelector::Action(ref id))
            if id.as_str() == "missing"
    ));
}

#[test]
fn mixed_selection_with_an_unknown_selector_returns_unknown() {
    let plan = plan_fixture();
    let selection = JobSelection::Only(BTreeSet::from([
        JobSelector::Package(selector_id("manual")),
        JobSelector::Action(selector_id("missing")),
    ]));

    let error = plan
        .select(&selection)
        .expect_err("an unknown selector should fail the complete selection");

    assert!(matches!(
        error,
        JobSelectionError::Unknown(JobSelector::Action(ref id))
            if id.as_str() == "missing"
    ));
}

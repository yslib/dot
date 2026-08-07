mod support;

use std::collections::BTreeSet;
use std::path::Path;

use dot_core::interpolation::{DotPaths, ExecutionEnvironment, XdgPaths};
use dot_core::job::{JobId, JobKind, JobSelection, JobSelector, JobSelectorParseError};
use dot_core::manifest::EffectiveManifest;
use dot_core::native::plan::{
    ExecutionPlan, ExecutionPlanError, ExecutionPlanner, JobSelectionError, PlannedJob,
    PlanningError,
};
use dot_core::platform::PlatformInfo;
use dot_core::schema::{Config, Identifier, SelectorIdentifier};
use support::fixture;

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

fn platform() -> PlatformInfo {
    PlatformInfo {
        os: "linux".into(),
        arch: "x86_64".into(),
        distro: Some("test".into()),
        distro_families: BTreeSet::new(),
        environments: BTreeSet::from(["native".into()]),
    }
}

fn environment() -> ExecutionEnvironment {
    ExecutionEnvironment::from_variables([
        ("HOME", TEST_HOME),
        ("ROOT", "/opt"),
        ("RUNNER", "bash"),
    ])
}

fn plan_fixture(selection: &JobSelection) -> ExecutionPlan {
    plan_named_fixture("dry-run/valid-human-readable-plan.toml", selection)
}

fn plan_named_fixture(name: &str, selection: &JobSelection) -> ExecutionPlan {
    let input = fixture::read(name);
    let config: Config = toml::from_str(&input).expect("test config should deserialize");
    let platform = platform();
    let target = selector_id("machine");
    let manifest = EffectiveManifest::select_for_execution(&config, &platform, Some(&target), None)
        .expect("test manifest should select");
    let environment = environment();
    let xdg = XdgPaths::detect();
    let dot_paths = DotPaths::new(
        Path::new(TEST_CONFIG_DIR),
        Path::new(TEST_CONFIG_DIR),
        Path::new(TEST_CWD),
    );

    ExecutionPlanner::new(&environment, dot_paths, &xdg, &platform)
        .plan(&manifest, selection)
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
fn job_selectors_round_trip_the_canonical_spelling() {
    for spelling in ["package:editors", "action:setup", "link:nvim"] {
        let selector: JobSelector = spelling.parse().unwrap();
        assert_eq!(selector.to_string(), spelling);
    }
}

#[test]
fn job_selectors_reject_bare_provider_and_malformed_values() {
    for invalid in ["editors", "provider:brew", "package:", "package:bad/id"] {
        assert!(invalid.parse::<JobSelector>().is_err(), "{invalid}");
    }
}

#[test]
fn job_selector_parse_errors_distinguish_invalid_inputs() {
    assert_eq!(
        "editors".parse::<JobSelector>(),
        Err(JobSelectorParseError::MissingKind)
    );
    assert_eq!(
        ":editors".parse::<JobSelector>(),
        Err(JobSelectorParseError::MissingKind)
    );
    assert!(matches!(
        "package:bad/id".parse::<JobSelector>(),
        Err(JobSelectorParseError::InvalidIdentifier(_))
    ));
    assert_eq!(
        "service:editors".parse::<JobSelector>(),
        Err(JobSelectorParseError::UnknownKind("service".into()))
    );
    assert_eq!(
        "provider:brew".parse::<JobSelector>(),
        Err(JobSelectorParseError::ProviderNotSelectable)
    );
}

#[test]
fn job_ids_display_the_canonical_spelling() {
    for (job, spelling) in [
        (JobId::Provider(provider_id("brew")), "provider:brew"),
        (JobId::Package(selector_id("editors")), "package:editors"),
        (JobId::Action(selector_id("setup")), "action:setup"),
        (JobId::Link(selector_id("nvim")), "link:nvim"),
    ] {
        assert_eq!(job.to_string(), spelling);
    }
}

#[test]
fn job_selection_errors_use_canonical_job_selectors() {
    let cases = [
        (
            JobSelectionError::Unknown(JobSelector::Package(selector_id("tool"))),
            "unknown job `package:tool`",
        ),
        (
            JobSelectionError::Unknown(JobSelector::Action(selector_id("setup"))),
            "unknown job `action:setup`",
        ),
        (
            JobSelectionError::Unknown(JobSelector::Link(selector_id("config"))),
            "unknown job `link:config`",
        ),
        (
            JobSelectionError::MissingProvider {
                package: selector_id("tool"),
                provider: provider_id("missing"),
            },
            "package job `tool` references missing provider job `missing`",
        ),
    ];

    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected);
    }
}

#[test]
fn selected_link_does_not_resolve_an_unselected_action() {
    let path = fixture::path("selection/valid-selected-runtime-isolation.toml");
    let config = Config::parse(&fixture::read(
        "selection/valid-selected-runtime-isolation.toml",
    ))
    .expect("the complete config should validate statically");
    let platform = platform();
    let target = selector_id("machine");
    let manifest = EffectiveManifest::select_for_execution(&config, &platform, Some(&target), None)
        .expect("test manifest should select");
    let environment = environment();
    let xdg = XdgPaths::detect();
    let config_dir = path.parent().expect("fixture path should have a parent");
    let dot_paths = DotPaths::new(config_dir, config_dir, Path::new(TEST_CWD));
    let planner = ExecutionPlanner::new(&environment, dot_paths, &xdg, &platform);

    let plan = planner
        .plan(
            &manifest,
            &JobSelection::only(JobSelector::Link(selector_id("nvim"))),
        )
        .expect("an unselected runtime value must not be resolved");

    assert_eq!(
        plan.jobs().iter().map(PlannedJob::id).collect::<Vec<_>>(),
        [JobId::Link(selector_id("nvim"))]
    );
}

#[test]
fn selecting_provider_package_adds_only_its_provider() {
    let plan = plan_fixture(&JobSelection::only(JobSelector::Package(selector_id(
        "alpha",
    ))));

    assert_eq!(
        plan.jobs().iter().map(PlannedJob::id).collect::<Vec<_>>(),
        [
            JobId::Provider(provider_id("system")),
            JobId::Package(selector_id("alpha")),
        ]
    );
}

#[test]
fn selecting_two_packages_that_share_a_provider_adds_the_provider_once() {
    let selection = JobSelection::Only(BTreeSet::from([
        JobSelector::Package(selector_id("alpha")),
        JobSelector::Package(selector_id("beta")),
    ]));
    let plan = plan_named_fixture("dry-run/valid-provider-install-units.toml", &selection);

    assert_eq!(
        plan.jobs().iter().map(PlannedJob::id).collect::<Vec<_>>(),
        [
            JobId::Provider(provider_id("brew")),
            JobId::Package(selector_id("alpha")),
            JobId::Package(selector_id("beta")),
        ]
    );
}

#[test]
fn selecting_manual_action_or_link_adds_no_provider() {
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
        let plan = plan_fixture(&JobSelection::only(selector));
        assert_eq!(
            plan.jobs().iter().map(PlannedJob::id).collect::<Vec<_>>(),
            [expected]
        );
    }
}

#[test]
fn all_includes_every_effective_provider_and_job() {
    let plan = plan_fixture(&JobSelection::All);

    assert_eq!(
        plan.jobs().iter().map(PlannedJob::id).collect::<Vec<_>>(),
        [
            JobId::Provider(provider_id("system")),
            JobId::Package(selector_id("manual")),
            JobId::Package(selector_id("alpha")),
            JobId::Action(selector_id("configure")),
            JobId::Link(selector_id("gitconfig")),
        ]
    );
}

#[test]
fn complete_plan_uses_declaration_order_with_provider_grouped_packages() {
    let plan = plan_named_fixture("jobs/valid-declaration-order.toml", &JobSelection::All);

    assert_eq!(
        plan.jobs().iter().map(PlannedJob::id).collect::<Vec<_>>(),
        [
            JobId::Provider(provider_id("zulu")),
            JobId::Provider(provider_id("alpha")),
            JobId::Package(selector_id("manual-zulu")),
            JobId::Package(selector_id("manual-alpha")),
            JobId::Package(selector_id("zulu-first")),
            JobId::Package(selector_id("zulu-second")),
            JobId::Package(selector_id("alpha-first")),
            JobId::Package(selector_id("alpha-second")),
            JobId::Action(selector_id("zulu")),
            JobId::Action(selector_id("alpha")),
            JobId::Link(selector_id("zulu")),
            JobId::Link(selector_id("alpha")),
        ]
    );
}

#[test]
fn exact_selection_filters_without_reordering_declarations() {
    let selection = JobSelection::Only(BTreeSet::from([
        JobSelector::Link(selector_id("alpha")),
        JobSelector::Action(selector_id("alpha")),
        JobSelector::Package(selector_id("alpha-first")),
        JobSelector::Package(selector_id("manual-alpha")),
        JobSelector::Action(selector_id("zulu")),
        JobSelector::Package(selector_id("zulu-second")),
        JobSelector::Link(selector_id("zulu")),
    ]));
    let plan = plan_named_fixture("jobs/valid-declaration-order.toml", &selection);

    assert_eq!(
        plan.jobs().iter().map(PlannedJob::id).collect::<Vec<_>>(),
        [
            JobId::Provider(provider_id("zulu")),
            JobId::Provider(provider_id("alpha")),
            JobId::Package(selector_id("manual-alpha")),
            JobId::Package(selector_id("zulu-second")),
            JobId::Package(selector_id("alpha-first")),
            JobId::Action(selector_id("zulu")),
            JobId::Action(selector_id("alpha")),
            JobId::Link(selector_id("zulu")),
            JobId::Link(selector_id("alpha")),
        ]
    );
}

#[test]
fn unknown_typed_selector_fails_before_planning() {
    let input = fixture::read("dry-run/valid-human-readable-plan.toml");
    let config: Config = toml::from_str(&input).expect("test config should deserialize");
    let platform = platform();
    let target = selector_id("machine");
    let manifest = EffectiveManifest::select_for_execution(&config, &platform, Some(&target), None)
        .expect("test manifest should select");
    let environment = environment();
    let xdg = XdgPaths::detect();
    let planner = ExecutionPlanner::new(
        &environment,
        DotPaths::new(
            Path::new(TEST_CONFIG_DIR),
            Path::new(TEST_CONFIG_DIR),
            Path::new(TEST_CWD),
        ),
        &xdg,
        &platform,
    );

    let error = planner
        .plan(
            &manifest,
            &JobSelection::only(JobSelector::Action(selector_id("missing"))),
        )
        .expect_err("unknown action should fail");

    assert!(matches!(
        error,
        ExecutionPlanError::Selection(JobSelectionError::Unknown(
            JobSelector::Action(ref id)
        ))
            if id.as_str() == "missing"
    ));
}

#[test]
fn unknown_selector_rejects_the_complete_set_before_runtime_evaluation() {
    let path = fixture::path("selection/valid-selected-runtime-isolation.toml");
    let config = Config::parse(&fixture::read(
        "selection/valid-selected-runtime-isolation.toml",
    ))
    .expect("the complete config should validate statically");
    let platform = platform();
    let target = selector_id("machine");
    let manifest = EffectiveManifest::select_for_execution(&config, &platform, Some(&target), None)
        .expect("test manifest should select");
    let environment = environment();
    let xdg = XdgPaths::detect();
    let config_dir = path.parent().expect("fixture path should have a parent");
    let dot_paths = DotPaths::new(config_dir, config_dir, Path::new(TEST_CWD));
    let planner = ExecutionPlanner::new(&environment, dot_paths, &xdg, &platform);
    let selection = JobSelection::Only(BTreeSet::from([
        JobSelector::Action(selector_id("setup-editor")),
        JobSelector::Link(selector_id("missing")),
    ]));

    let error = planner
        .plan(&manifest, &selection)
        .expect_err("an unknown selector should fail the complete selection");

    assert!(matches!(
        error,
        ExecutionPlanError::Selection(JobSelectionError::Unknown(
            JobSelector::Link(ref id)
        ))
            if id.as_str() == "missing"
    ));
}

#[test]
fn selected_provider_package_reports_a_missing_provider_before_promotion() {
    let input = fixture::read("dry-run/invalid-unknown-provider-before-args.toml");
    let config: Config = toml::from_str(&input).expect("test config should deserialize");
    let platform = platform();
    let target = selector_id("machine");
    let manifest = EffectiveManifest::select_for_execution(&config, &platform, Some(&target), None)
        .expect("test manifest should select");
    let environment = environment();
    let xdg = XdgPaths::detect();
    let planner = ExecutionPlanner::new(
        &environment,
        DotPaths::new(
            Path::new(TEST_CONFIG_DIR),
            Path::new(TEST_CONFIG_DIR),
            Path::new(TEST_CWD),
        ),
        &xdg,
        &platform,
    );

    let error = planner
        .plan(
            &manifest,
            &JobSelection::only(JobSelector::Package(selector_id("invalid-args"))),
        )
        .expect_err("the provider closure should fail before provider args promotion");

    assert!(matches!(
        error,
        ExecutionPlanError::Selection(JobSelectionError::MissingProvider {
            package,
            provider,
        }) if package.as_str() == "invalid-args" && provider.as_str() == "missing"
    ));
}

#[test]
fn selected_interpolation_failure_discards_a_valid_planned_prefix() {
    let path = fixture::path("selection/valid-selected-runtime-isolation.toml");
    let config = Config::parse(&fixture::read(
        "selection/valid-selected-runtime-isolation.toml",
    ))
    .expect("the complete config should validate statically");
    let platform = platform();
    let target = selector_id("machine");
    let manifest = EffectiveManifest::select_for_execution(&config, &platform, Some(&target), None)
        .expect("test manifest should select");
    let environment = environment();
    let xdg = XdgPaths::detect();
    let config_dir = path.parent().expect("fixture path should have a parent");
    let dot_paths = DotPaths::new(config_dir, config_dir, Path::new(TEST_CWD));
    let planner = ExecutionPlanner::new(&environment, dot_paths, &xdg, &platform);

    let selection = JobSelection::Only(BTreeSet::from([
        JobSelector::Package(selector_id("editor")),
        JobSelector::Action(selector_id("setup-editor")),
    ]));
    let error = planner
        .plan(&manifest, &selection)
        .expect_err("the selected runtime value should fail atomically");

    assert!(
        matches!(
            &error,
            ExecutionPlanError::Planning(PlanningError::Interpolation { context, .. })
                if context == "selected job `action:setup-editor` field `exec.program`"
        ),
        "unexpected error: {error}"
    );
    assert_eq!(
        error.to_string(),
        "failed to resolve selected job `action:setup-editor` field `exec.program`: environment variable `DOT_INTENTIONALLY_MISSING` is not defined"
    );
}

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use dot::interpolation::{DotPaths, ExecutionEnvironment, InterpolationError, XdgPaths};
use dot::job::{JobKind, JobSelection, JobSelector};
use dot::manifest::EffectiveManifest;
use dot::native::dry_run;
use dot::native::plan::{
    ExecutionPlanError, ExecutionPlanner, PlannedActionKind, PlannedProviderInstall, PlanningError,
};
use dot::native::{ConfigFile, ConfigLocation, NativeRuntime};
use dot::platform::PlatformInfo;
use dot::report::{
    ActionInfo, ItemStatus, PackageSource, ProviderPackageSource, ReportCommand, ReportStatus,
    ReportSubject,
};
use dot::schema::{
    Config, FetchContentConflict, LinkConflict, LinkMissingParent, ResolvedString,
    SelectorIdentifier,
};
use dot::selection::{ExecutionSelection, ProfileSelection, ScopeSelection};
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

fn platform() -> PlatformInfo {
    PlatformInfo {
        os: "linux".into(),
        arch: "x86_64".into(),
        distro: Some("test".into()),
        distro_families: BTreeSet::new(),
        environments: BTreeSet::from(["native".into()]),
    }
}

fn selector_id(value: &str) -> SelectorIdentifier {
    SelectorIdentifier::new(value).expect("test selector identifier should be valid")
}

fn environment() -> ExecutionEnvironment {
    ExecutionEnvironment::from_variables([
        ("HOME", TEST_HOME),
        ("ROOT", "/opt"),
        ("RUNNER", "bash"),
    ])
}

fn dot_paths() -> DotPaths<'static> {
    DotPaths::new(
        Path::new(TEST_CONFIG),
        Path::new(TEST_CONFIG_DIR),
        Path::new(TEST_CONFIG),
        Path::new(TEST_CONFIG_DIR),
        Path::new(TEST_CWD),
    )
}

fn config_path(relative: &str) -> PathBuf {
    Path::new(TEST_CONFIG_DIR).join(relative)
}

fn config_template_path(relative: &str) -> String {
    format!("{TEST_CONFIG_DIR}/{relative}")
}

fn gitconfig_target() -> PathBuf {
    PathBuf::from(format!("{TEST_HOME}/.gitconfig"))
}

fn select_fixture(name: &str) -> EffectiveManifest {
    select_named_fixture(name, "machine", None)
}

fn select_named_fixture(name: &str, target: &str, profile: Option<&str>) -> EffectiveManifest {
    let input = fixture::read(name);
    let config: Config = toml::from_str(&input).expect("test config should deserialize");
    let target = selector_id(target);
    let profile = profile.map(selector_id);
    EffectiveManifest::select_for_execution(&config, &platform(), Some(&target), profile.as_ref())
        .expect("test manifest should select")
}

fn fetch_content_fixture(target: &Path) -> String {
    let target = format!("{:?}", target.to_string_lossy());
    fixture::read("dry-run/valid-fetch-content-template.toml").replace("__TARGET__", &target)
}

#[test]
fn execution_plan_exposes_one_ordered_typed_job_sequence() {
    let manifest = select_fixture("dry-run/valid-human-readable-plan.toml");
    let environment = environment();
    let xdg = XdgPaths::detect();
    let platform = platform();
    let plan = ExecutionPlanner::new(&environment, dot_paths(), &xdg, &platform)
        .plan(&manifest, &JobSelection::All)
        .expect("execution should plan");

    let ids = plan
        .jobs()
        .iter()
        .map(|job| (job.id().kind(), job.id().name().to_owned()))
        .collect::<Vec<_>>();

    assert_eq!(
        ids,
        [
            (JobKind::Provider, "system".into()),
            (JobKind::Package, "manual".into()),
            (JobKind::Package, "alpha".into()),
            (JobKind::Action, "configure".into()),
            (JobKind::Link, "gitconfig".into()),
        ]
    );
}

#[test]
fn report_projects_only_the_jobs_selected_before_runtime_resolution() {
    let path = fixture::path("selection/valid-selected-runtime-isolation.toml");
    let config =
        ConfigFile::load(ConfigLocation::Path(path)).expect("the complete config should validate");
    let runtime = NativeRuntime::detect();
    let platform = platform();
    let selection = ExecutionSelection {
        scope: ScopeSelection {
            target: Some(selector_id("machine")),
            profile: ProfileSelection::Root,
        },
        jobs: JobSelection::only(JobSelector::Link(
            "nvim".try_into().expect("test selector should be valid"),
        )),
    };

    let report = dot::native::dry_run(&config, &runtime, &platform, &selection)
        .expect("unselected runtime values must remain unresolved");

    assert_eq!(report.items.len(), 1);
    assert_eq!(report.items[0].id, "nvim");
    assert!(matches!(report.items[0].subject, ReportSubject::Link(_)));
}

#[test]
fn selected_fetch_content_action_is_planned_and_projected_without_io() {
    let workspace = tempfile::tempdir().expect("temporary workspace should be created");
    let fetch_target = workspace.path().join("missing-parent/config.toml");
    let input = fetch_content_fixture(&fetch_target);
    let config: Config = toml::from_str(&input).expect("test config should deserialize");
    let target = selector_id("machine");
    let platform = platform();
    let manifest = EffectiveManifest::select_for_execution(&config, &platform, Some(&target), None)
        .expect("test manifest should select");
    let environment = environment();
    let xdg = XdgPaths::detect();
    let planner = ExecutionPlanner::new(&environment, dot_paths(), &xdg, &platform);

    let plan = planner
        .plan(
            &manifest,
            &JobSelection::only(JobSelector::Action(selector_id("remote-config"))),
        )
        .expect("selected fetch action should plan");

    let actions = plan.actions().collect::<Vec<_>>();
    assert_eq!(actions.len(), 1);
    let PlannedActionKind::FetchContent(fetch) = actions[0].kind() else {
        panic!("selected fetch action should preserve its planned kind");
    };
    assert_eq!(fetch.source().as_str(), "https://192.0.2.1/unreachable");
    assert_eq!(fetch.target(), fetch_target);
    assert_eq!(fetch.on_conflict(), FetchContentConflict::Replace);

    let report = dry_run::build_report(Path::new(TEST_CONFIG), &plan);

    assert_eq!(report.status, ReportStatus::Planned);
    assert_eq!(report.items.len(), 1);
    assert_eq!(report.items[0].status, ItemStatus::Planned);
    assert!(matches!(
        &report.items[0].subject,
        ReportSubject::Action(action)
            if matches!(
                &action.action,
                ActionInfo::FetchContent { source, target: actual_target, on_conflict: FetchContentConflict::Replace }
                    if source == "https://192.0.2.1/unreachable" && actual_target == &fetch_target
            )
    ));
    assert!(
        !fetch_target
            .parent()
            .expect("target should have a parent")
            .exists(),
        "dry-run must not inspect deeply enough to create the target parent"
    );
}

#[test]
fn fetch_source_locator_policy_accepts_https_without_confusing_path_data_for_userinfo() {
    let input = r#"
[targets.machine]
platform = { os = "linux" }

[targets.machine.actions.uppercase]
source = "HTTPS://example.com/config.toml"
target = "configs/uppercase.toml"

[targets.machine.actions.ipv6]
source = "https://[2001:db8::1]/config.toml"
target = "configs/ipv6.toml"

[targets.machine.actions.path-at]
source = "https://example.com/@scope/config.toml"
target = "configs/path-at.toml"
"#;
    let config: Config = toml::from_str(input).expect("test config should deserialize");
    let target = selector_id("machine");
    let platform = platform();
    let manifest = EffectiveManifest::select_for_execution(&config, &platform, Some(&target), None)
        .expect("test manifest should select");
    let environment = environment();
    let xdg = XdgPaths::detect();

    let plan = ExecutionPlanner::new(&environment, dot_paths(), &xdg, &platform)
        .plan(&manifest, &JobSelection::All)
        .expect("valid HTTPS locator forms should plan");
    let sources = plan
        .actions()
        .map(|action| {
            let PlannedActionKind::FetchContent(fetch) = action.kind() else {
                panic!("fixture actions should all be fetch actions");
            };
            (action.id(), fetch.source().as_str())
        })
        .collect::<BTreeMap<_, _>>();

    assert_eq!(sources["uppercase"], "https://example.com/config.toml");
    assert_eq!(sources["ipv6"], "https://[2001:db8::1]/config.toml");
    assert_eq!(sources["path-at"], "https://example.com/@scope/config.toml");
}

#[test]
fn fetch_runtime_errors_are_atomic_for_selected_actions_and_deferred_for_unselected_actions() {
    let input = r#"
[targets.machine]
platform = { os = "linux" }

[targets.machine.actions.broken]
source = "http://example.com/insecure"
target = "https://example.com/not-a-local-target"

[targets.machine.actions.empty-userinfo]
source = "https://@example.com/config.toml"
target = "configs/empty-userinfo.toml"

[targets.machine.actions.empty-password-userinfo]
source = "https://:@example.com/config.toml"
target = "configs/empty-password-userinfo.toml"

[targets.machine.actions.invalid-url]
source = ":not-a-url"
target = "configs/url.toml"

[targets.machine.actions.noncanonical-https]
source = "https:///config.toml"
target = "configs/noncanonical.toml"

[targets.machine.actions.secret-userinfo]
source = "https://account:topsecret@example.com/config.toml"
target = "configs/secret.toml"

[targets.machine.actions.invalid-target]
source = "https://example.com/config.toml"
target = "https://account:topsecret@example.com/not-a-local-target"

[targets.machine.actions.missing-env]
source = "${env:DOT_MISSING_FETCH_SOURCE}"
target = "configs/missing.toml"

[targets.machine.actions.missing-target-env]
source = "https://example.com/config.toml"
target = "${env:DOT_MISSING_FETCH_TARGET}"

[targets.machine.actions.valid]
source = "https://example.com/config.toml"
target = "configs/app.toml"
"#;
    let config: Config = toml::from_str(input).expect("test config should deserialize");
    let target = selector_id("machine");
    let platform = platform();
    let manifest = EffectiveManifest::select_for_execution(&config, &platform, Some(&target), None)
        .expect("test manifest should select");
    let environment = environment();
    let xdg = XdgPaths::detect();
    let planner = ExecutionPlanner::new(&environment, dot_paths(), &xdg, &platform);

    let plan = planner
        .plan(
            &manifest,
            &JobSelection::only(JobSelector::Action(selector_id("valid"))),
        )
        .expect("unselected invalid fetch fields should remain deferred");
    assert_eq!(
        plan.actions().map(|action| action.id()).collect::<Vec<_>>(),
        ["valid"]
    );

    let error = planner
        .plan(
            &manifest,
            &JobSelection::only(JobSelector::Action(selector_id("broken"))),
        )
        .expect_err("selected invalid fetch source should fail planning atomically");
    assert!(matches!(
        error,
        ExecutionPlanError::Planning(PlanningError::UnsupportedFetchContentSource { action })
            if action == "broken"
    ));

    for action in ["empty-userinfo", "empty-password-userinfo"] {
        let error = planner
            .plan(
                &manifest,
                &JobSelection::only(JobSelector::Action(selector_id(action))),
            )
            .expect_err("selected empty source userinfo should fail planning");
        assert!(matches!(
            error,
            ExecutionPlanError::Planning(
                PlanningError::AuthenticatedFetchContentSource { action: actual }
            ) if actual == action
        ));
    }

    let error = planner
        .plan(
            &manifest,
            &JobSelection::only(JobSelector::Action(selector_id("invalid-url"))),
        )
        .expect_err("selected invalid fetch URL should fail planning");
    assert!(matches!(
        error,
        ExecutionPlanError::Planning(PlanningError::InvalidFetchContentSourceUrl { action, .. })
            if action == "invalid-url"
    ));

    let error = planner
        .plan(
            &manifest,
            &JobSelection::only(JobSelector::Action(selector_id("noncanonical-https"))),
        )
        .expect_err("selected noncanonical HTTPS source should fail planning");
    assert!(matches!(
        error,
        ExecutionPlanError::Planning(PlanningError::UnsupportedFetchContentSource { action })
            if action == "noncanonical-https"
    ));

    let error = planner
        .plan(
            &manifest,
            &JobSelection::only(JobSelector::Action(selector_id("secret-userinfo"))),
        )
        .expect_err("selected authenticated source should fail planning");
    let message = error.to_string();
    assert!(matches!(
        error,
        ExecutionPlanError::Planning(PlanningError::AuthenticatedFetchContentSource { action })
            if action == "secret-userinfo"
    ));
    assert!(!message.contains("account"));
    assert!(!message.contains("topsecret"));

    let error = planner
        .plan(
            &manifest,
            &JobSelection::only(JobSelector::Action(selector_id("invalid-target"))),
        )
        .expect_err("selected URL target should fail planning");
    let message = error.to_string();
    assert!(matches!(
        error,
        ExecutionPlanError::Planning(PlanningError::UnsupportedFetchContentTarget { action })
            if action == "invalid-target"
    ));
    assert!(!message.contains("account"));
    assert!(!message.contains("topsecret"));

    let error = planner
        .plan(
            &manifest,
            &JobSelection::only(JobSelector::Action(selector_id("missing-env"))),
        )
        .expect_err("selected unresolved fetch expression should fail planning");
    assert!(matches!(
        error,
        ExecutionPlanError::Planning(PlanningError::Interpolation { context, .. })
            if context == "selected job `action:missing-env` field `source`"
    ));

    let error = planner
        .plan(
            &manifest,
            &JobSelection::only(JobSelector::Action(selector_id("missing-target-env"))),
        )
        .expect_err("selected unresolved fetch target should fail planning");
    assert!(matches!(
        error,
        ExecutionPlanError::Planning(PlanningError::Interpolation { context, .. })
            if context == "selected job `action:missing-target-env` field `target`"
    ));
}

#[cfg(unix)]
#[test]
fn fetch_targets_keep_entry_real_and_absolute_path_semantics_through_execution_planning() {
    use std::os::unix::fs::symlink;

    let workspace = tempfile::tempdir().expect("temporary workspace should be created");
    let entry_dir = workspace.path().join("entry");
    let real_dir = workspace.path().join("real");
    std::fs::create_dir_all(&entry_dir).expect("entry directory should be created");
    std::fs::create_dir_all(&real_dir).expect("real directory should be created");
    let real_config = real_dir.join("dot.toml");
    std::fs::write(&real_config, "").expect("real config should be written");
    let entry_config = entry_dir.join("dot.toml");
    symlink(&real_config, &entry_config).expect("config entry symlink should be created");
    let absolute = workspace.path().join("absolute/config.toml");
    let input = format!(
        r#"
[targets.machine]
platform = {{ os = "linux" }}

[targets.machine.actions.absolute]
source = "https://example.com/absolute"
target = {:?}

[targets.machine.actions.entry-relative]
source = "https://example.com/entry"
target = "configs/entry.toml"

[targets.machine.actions.real-explicit]
source = "https://example.com/real"
target = "${{dot:real_config_dir}}/configs/real.toml"
"#,
        absolute.to_string_lossy()
    );
    let config: Config = toml::from_str(&input).expect("test config should deserialize");
    let target = selector_id("machine");
    let platform = platform();
    let manifest = EffectiveManifest::select_for_execution(&config, &platform, Some(&target), None)
        .expect("test manifest should select");
    let environment = environment();
    let xdg = XdgPaths::detect();
    let dot_paths = DotPaths::new(
        &entry_config,
        &entry_dir,
        &real_config,
        &real_dir,
        workspace.path(),
    );

    let plan = ExecutionPlanner::new(&environment, dot_paths, &xdg, &platform)
        .plan(&manifest, &JobSelection::All)
        .expect("all fetch target forms should plan");
    assert!(plan.actions().all(|action| {
        matches!(
            action.kind(),
            PlannedActionKind::FetchContent(fetch)
                if fetch.on_conflict() == FetchContentConflict::Error
        )
    }));
    let targets = plan
        .actions()
        .map(|action| {
            let PlannedActionKind::FetchContent(fetch) = action.kind() else {
                panic!("fixture actions should all be fetch actions");
            };
            (action.id(), fetch.target().to_owned())
        })
        .collect::<BTreeMap<_, _>>();

    assert_eq!(targets["absolute"], absolute);
    assert_eq!(
        targets["entry-relative"],
        entry_dir.join("configs/entry.toml")
    );
    assert_eq!(targets["real-explicit"], real_dir.join("configs/real.toml"));
}

#[test]
fn plans_only_selected_effective_records_and_defers_unused_runtime_values() {
    let manifest = select_named_fixture(
        "dry-run/valid-deferred-expression-errors.toml",
        "selected",
        Some("chosen"),
    );
    let environment = environment();
    let xdg = XdgPaths::detect();
    let platform = platform();
    let planner = ExecutionPlanner::new(&environment, dot_paths(), &xdg, &platform);

    let plan = planner
        .plan(&manifest, &JobSelection::All)
        .expect("deferred runtime values must not affect the selected plan");

    assert_eq!(plan.target(), "selected");
    assert_eq!(plan.profile(), Some("chosen"));
    let providers = plan.providers().collect::<Vec<_>>();
    assert_eq!(
        providers
            .iter()
            .map(|provider| provider.id())
            .collect::<Vec<_>>(),
        ["unused-broken", "shared"]
    );
    assert_eq!(providers[0].probe().program.value(), "unused-probe");
    assert_eq!(providers[1].probe().program.value(), "selected-probe");
    assert!(providers[1].activate().is_none());
    assert!(providers[1].ensure().is_empty());
    let provider_installs = plan.provider_installs().collect::<Vec<_>>();
    assert_eq!(
        provider_installs
            .iter()
            .map(|install| install.id())
            .collect::<Vec<_>>(),
        ["shared-package"]
    );
    assert_eq!(
        provider_installs[0].install().program.value(),
        "selected-install"
    );
    assert!(plan.manual_packages().next().is_none());
    let actions = plan.actions().collect::<Vec<_>>();
    assert_eq!(
        actions.iter().map(|action| action.id()).collect::<Vec<_>>(),
        ["shared-action"]
    );
    let PlannedActionKind::Command(action) = actions[0].kind() else {
        panic!("shared-action should be a command action");
    };
    assert_eq!(action.exec.program.value(), "selected-action");
    assert!(action.check.is_none());
    let links = plan.links().collect::<Vec<_>>();
    assert_eq!(
        links.iter().map(|link| link.id()).collect::<Vec<_>>(),
        ["shared-link"]
    );
    assert_eq!(links[0].source(), config_path("selected-source"));
    assert_eq!(
        links[0].target(),
        Path::new(&format!("{TEST_HOME}/selected-target"))
    );
}

#[test]
fn rejects_selected_expression_errors_at_their_existing_consumers() {
    #[derive(Clone, Copy, Debug)]
    enum ExpectedError {
        UnclosedResolver,
        UnknownResolver,
        ResolverUnavailable,
        ListResolverMustOccupyArgument,
    }

    let cases = [
        (
            "malformed",
            "selected job `action:malformed-action` field `exec.program`",
            ExpectedError::UnclosedResolver,
        ),
        (
            "unknown",
            "selected job `action:unknown-action` field `exec.program`",
            ExpectedError::UnknownResolver,
        ),
        (
            "unavailable",
            "selected job `action:unavailable-action` field `exec.program`",
            ExpectedError::ResolverUnavailable,
        ),
        (
            "wrong-output",
            "selected job `package:wrong-output-package` field `provider.install.args`",
            ExpectedError::ListResolverMustOccupyArgument,
        ),
    ];

    for (profile, expected_context, expected_error) in cases {
        let manifest = select_named_fixture(
            "dry-run/invalid-selected-expression-errors.toml",
            "machine",
            Some(profile),
        );
        let environment = environment();
        let xdg = XdgPaths::detect();
        let platform = platform();
        let planner = ExecutionPlanner::new(&environment, dot_paths(), &xdg, &platform);

        let error = planner
            .plan(&manifest, &JobSelection::All)
            .expect_err("the selected expression error must fail planning");
        let ExecutionPlanError::Planning(PlanningError::Interpolation { context, source }) = error
        else {
            panic!("unexpected planning error for profile `{profile}`: {error}");
        };

        assert_eq!(context, expected_context, "profile `{profile}`");
        assert!(
            match expected_error {
                ExpectedError::UnclosedResolver => {
                    matches!(source, InterpolationError::UnclosedResolver { offset: 0 })
                }
                ExpectedError::UnknownResolver => matches!(
                    source,
                    InterpolationError::UnknownResolver { ref name } if name == "mystery"
                ),
                ExpectedError::ResolverUnavailable => matches!(
                    source,
                    InterpolationError::ResolverUnavailable { ref resolver }
                        if resolver == "package"
                ),
                ExpectedError::ListResolverMustOccupyArgument => matches!(
                    source,
                    InterpolationError::ListResolverMustOccupyArgument { ref resolver }
                        if resolver == "package"
                ),
            },
            "unexpected interpolation error for profile `{profile}`: {source}"
        );
    }
}

#[test]
fn selected_provider_package_runtime_errors_identify_the_exact_install_field() {
    let path = fixture::path("selection/valid-selected-provider-runtime-errors.toml");
    let loaded = dot::native::ConfigFile::load(dot::native::ConfigLocation::Path(path.clone()))
        .expect("the complete config should validate");
    let platform = platform();
    let target = selector_id("machine");
    let manifest =
        EffectiveManifest::select_for_execution(loaded.config(), &platform, Some(&target), None)
            .expect("test manifest should select");
    let environment = environment();
    let xdg = XdgPaths::detect();
    let config_dir = path.parent().expect("fixture path should have a parent");
    let planner = ExecutionPlanner::new(
        &environment,
        DotPaths::new(&path, config_dir, &path, config_dir, Path::new(TEST_CWD)),
        &xdg,
        &platform,
    );

    for (package, field, missing_name) in [
        (
            "program-package",
            "provider.install.program",
            "DOT_INTENTIONALLY_MISSING_PROGRAM",
        ),
        (
            "argument-package",
            "provider.install.args[2]",
            "DOT_INTENTIONALLY_MISSING_ARGUMENT",
        ),
        (
            "cwd-package",
            "provider.install.cwd",
            "DOT_INTENTIONALLY_MISSING_CWD",
        ),
        (
            "env-package",
            "provider.install.env",
            "DOT_INTENTIONALLY_MISSING_ENV",
        ),
    ] {
        let selector = JobSelector::Package(
            package
                .try_into()
                .expect("test package selector should be valid"),
        );
        let error = planner
            .plan(&manifest, &JobSelection::only(selector))
            .expect_err("the selected provider install should fail resolution");
        let ExecutionPlanError::Planning(PlanningError::Interpolation { context, source }) = error
        else {
            panic!("unexpected planning error for package `{package}`: {error}");
        };

        assert_eq!(
            context,
            format!("selected job `package:{package}` field `{field}`")
        );
        assert!(matches!(
            source,
            InterpolationError::MissingEnvironmentVariable { name }
                if name == missing_name
        ));
    }
}

#[test]
fn rejects_an_unknown_provider_before_invalid_literal_provider_args() {
    let manifest = select_fixture("dry-run/invalid-unknown-provider-before-args.toml");
    let environment = environment();
    let xdg = XdgPaths::detect();
    let platform = platform();
    let planner = ExecutionPlanner::new(&environment, dot_paths(), &xdg, &platform);

    let error = planner
        .plan(&manifest, &JobSelection::All)
        .expect_err("provider lookup must precede provider_args validation");

    assert!(matches!(
        error,
        ExecutionPlanError::Planning(PlanningError::UnknownProvider { package, provider })
            if package == "invalid-args" && provider == "missing"
    ));
}

#[test]
fn plans_provider_install_units_independently_and_resolves_their_environment() {
    let manifest = select_fixture("dry-run/valid-provider-install-units.toml");
    let environment = environment();
    let xdg = XdgPaths::detect();
    let platform = platform();
    let planner = ExecutionPlanner::new(&environment, dot_paths(), &xdg, &platform);

    let plan = planner
        .plan(&manifest, &JobSelection::All)
        .expect("execution should plan");

    let providers = plan.providers().collect::<Vec<_>>();
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0].id(), "brew");
    assert_eq!(
        providers[0].probe().program.value(),
        "/opt/homebrew/bin/brew"
    );
    assert_eq!(providers[0].ensure().len(), 1);

    let provider_installs = plan.provider_installs().collect::<Vec<_>>();
    assert_eq!(provider_installs.len(), 3);

    let alpha = provider_installs[0];
    assert!(matches!(alpha, PlannedProviderInstall::Single(_)));
    assert_eq!(alpha.id(), "alpha");
    assert_eq!(alpha.provider(), "brew");
    assert_eq!(alpha.provider_args(), &[] as &[String]);
    assert_eq!(alpha.names().collect::<Vec<_>>(), ["alpha"]);
    assert_eq!(
        alpha
            .install()
            .args
            .iter()
            .map(ResolvedString::value)
            .collect::<Vec<_>>(),
        vec!["before", "middle", "alpha", "after"]
    );

    let beta = provider_installs[1];
    assert!(matches!(beta, PlannedProviderInstall::Single(_)));
    assert_eq!(beta.id(), "beta");
    assert_eq!(beta.names().collect::<Vec<_>>(), ["beta"]);
    assert_eq!(
        beta.install()
            .args
            .iter()
            .map(ResolvedString::value)
            .collect::<Vec<_>>(),
        vec!["before", "middle", "beta", "after"]
    );

    let fonts = provider_installs[2];
    assert!(matches!(fonts, PlannedProviderInstall::Batch(_)));
    assert_eq!(fonts.id(), "fonts");
    assert_eq!(fonts.provider_args(), &[String::from("--cask")]);
    assert_eq!(fonts.names().collect::<Vec<_>>(), ["font-one", "font-two"]);
    assert_eq!(
        fonts
            .install()
            .args
            .iter()
            .map(ResolvedString::value)
            .collect::<Vec<_>>(),
        vec![
            "before", "--cask", "middle", "font-one", "font-two", "after"
        ]
    );
}

#[test]
fn projects_one_dry_run_item_per_provider_install_unit() {
    let manifest = select_fixture("dry-run/valid-provider-install-units.toml");
    let environment = environment();
    let xdg = XdgPaths::detect();
    let platform = platform();
    let planner = ExecutionPlanner::new(&environment, dot_paths(), &xdg, &platform);
    let plan = planner
        .plan(&manifest, &JobSelection::All)
        .expect("execution should plan");
    let report = dry_run::build_report(Path::new(TEST_CONFIG), &plan);

    assert_eq!(report.items.len(), 4);
    assert_eq!(report.items[1].id, "alpha");
    assert_eq!(report.items[2].id, "beta");
    assert_eq!(report.items[3].id, "fonts");
    assert!(matches!(
        &report.items[1].subject,
        ReportSubject::Package(package)
            if matches!(
                &package.source,
                PackageSource::Provider(ProviderPackageSource::Single { provider, .. })
                    if provider == "brew"
            )
    ));
    assert!(matches!(
        &report.items[3].subject,
        ReportSubject::Package(package)
            if matches!(
                &package.source,
                PackageSource::Provider(ProviderPackageSource::Batch { names, .. })
                    if names == &["font-one", "font-two"]
            )
    ));
}

#[test]
fn rejects_an_empty_provider_package_batch() {
    let manifest = select_fixture("dry-run/invalid-empty-package-batch.toml");
    let environment = environment();
    let xdg = XdgPaths::detect();
    let platform = platform();
    let planner = ExecutionPlanner::new(&environment, dot_paths(), &xdg, &platform);

    let error = planner
        .plan(&manifest, &JobSelection::All)
        .expect_err("an empty batch must fail");

    assert!(matches!(
        error,
        ExecutionPlanError::Planning(PlanningError::EmptyPackageBatch { package })
            if package == "empty-tools"
    ));
}

#[test]
fn rejects_a_duplicate_name_inside_one_provider_package_batch() {
    let manifest = select_fixture("dry-run/invalid-duplicate-package-batch-name.toml");
    let environment = environment();
    let xdg = XdgPaths::detect();
    let platform = platform();
    let planner = ExecutionPlanner::new(&environment, dot_paths(), &xdg, &platform);

    let error = planner
        .plan(&manifest, &JobSelection::All)
        .expect_err("a duplicate batch name must fail");

    assert!(matches!(
        error,
        ExecutionPlanError::Planning(PlanningError::DuplicatePackageBatchName { package, name })
            if package == "duplicate-tools" && name == "ripgrep"
    ));
}

#[test]
fn nonempty_provider_args_require_exactly_one_install_list_resolver() {
    let cases = [
        ("invalid-provider-args-resolver.toml", 0),
        ("invalid-provider-args-resolver-twice.toml", 2),
        ("invalid-provider-args-resolver-escaped.toml", 0),
    ];

    for (fixture_name, expected_count) in cases {
        let manifest = select_fixture(&format!("dry-run/{fixture_name}"));
        let environment = environment();
        let xdg = XdgPaths::detect();
        let platform = platform();
        let planner = ExecutionPlanner::new(&environment, dot_paths(), &xdg, &platform);

        let error = planner
            .plan(&manifest, &JobSelection::All)
            .expect_err("nonempty provider args must not be silently discarded");

        assert!(
            matches!(
                error,
                ExecutionPlanError::Planning(PlanningError::ProviderArgsResolverCount {
                    ref package,
                    ref provider,
                    actual,
                })
                    if package == "app"
                        && provider == "brew"
                        && actual == expected_count
            ),
            "unexpected error for {fixture_name}: {error}"
        );
    }
}

#[test]
fn resolves_manual_packages_actions_and_links_without_inspection() {
    let manifest = select_fixture("dry-run/valid-manual-actions-links.toml");
    let environment = environment();
    let xdg = XdgPaths::detect();
    let platform = platform();
    let planner = ExecutionPlanner::new(&environment, dot_paths(), &xdg, &platform);

    let plan = planner
        .plan(&manifest, &JobSelection::All)
        .expect("execution should plan");

    let manual_packages = plan.manual_packages().collect::<Vec<_>>();
    assert_eq!(manual_packages.len(), 1);
    assert_eq!(manual_packages[0].id(), "manual-tool");
    assert_eq!(
        manual_packages[0]
            .install()
            .check
            .as_ref()
            .unwrap()
            .program
            .value(),
        "/opt/bin/manual-tool"
    );
    assert_eq!(manual_packages[0].install().exec.program.value(), "bash");

    let actions = plan.actions().collect::<Vec<_>>();
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].id(), "configure");
    let PlannedActionKind::Command(action) = actions[0].kind() else {
        panic!("configure should be a command action");
    };
    assert_eq!(
        action.exec.args[0].value(),
        config_template_path("scripts/configure.sh")
    );

    let links = plan.links().collect::<Vec<_>>();
    assert_eq!(links.len(), 1);
    let link = links[0];
    assert_eq!(link.id(), "gitconfig");
    assert_eq!(link.source(), config_path("home/.gitconfig"));
    assert_eq!(link.target(), gitconfig_target());
    assert_eq!(link.on_conflict(), LinkConflict::Error);
    assert_eq!(link.on_missing_parent(), LinkMissingParent::Skip);
}

#[test]
fn projects_a_resolved_plan_to_one_report_item_per_logical_object() {
    let manifest = select_fixture("dry-run/valid-human-readable-plan.toml");
    let environment = environment();
    let xdg = XdgPaths::detect();
    let platform = platform();
    let planner = ExecutionPlanner::new(&environment, dot_paths(), &xdg, &platform);

    let plan = planner
        .plan(&manifest, &JobSelection::All)
        .expect("execution should plan");
    let report = dry_run::build_report(Path::new(TEST_CONFIG), &plan);

    assert_eq!(report.command, ReportCommand::DryRun);
    assert_eq!(report.status, ReportStatus::Planned);
    assert_eq!(report.context.target, "machine");
    assert_eq!(report.context.profile, None);
    assert_eq!(report.context.platform, platform);
    assert_eq!(report.items.len(), 5);
    assert!(
        report
            .items
            .iter()
            .all(|item| item.status == ItemStatus::Planned)
    );
    assert!(matches!(
        &report.items[0].subject,
        ReportSubject::Provider(_)
    ));
    assert!(matches!(
        &report.items[1].subject,
        ReportSubject::Package(package)
            if matches!(&package.source, PackageSource::Manual { .. })
    ));
    assert_eq!(report.items[1].id, "manual");
    assert!(matches!(
        &report.items[2].subject,
        ReportSubject::Package(package)
            if matches!(
                &package.source,
                PackageSource::Provider(ProviderPackageSource::Single { provider, .. })
                    if provider == "system"
            )
    ));
    assert_eq!(report.items[2].id, "alpha");
    assert!(matches!(&report.items[3].subject, ReportSubject::Action(_)));
    assert!(matches!(&report.items[4].subject, ReportSubject::Link(_)));
}

#[test]
fn rejects_a_package_that_references_an_unknown_effective_provider() {
    let manifest = select_fixture("dry-run/invalid-unknown-provider.toml");
    let environment = environment();
    let xdg = XdgPaths::detect();
    let platform = platform();
    let planner = ExecutionPlanner::new(&environment, dot_paths(), &xdg, &platform);

    let error = planner
        .plan(&manifest, &JobSelection::All)
        .expect_err("unknown providers must fail planning");

    assert!(matches!(
        error,
        ExecutionPlanError::Planning(PlanningError::UnknownProvider { package, provider })
            if package == "alpha" && provider == "missing"
    ));
}

#[test]
fn rejects_a_link_target_that_is_not_absolute_after_interpolation() {
    let manifest = select_fixture("dry-run/invalid-relative-link-target.toml");
    let environment = environment();
    let xdg = XdgPaths::detect();
    let platform = platform();
    let planner = ExecutionPlanner::new(&environment, dot_paths(), &xdg, &platform);

    let error = planner
        .plan(&manifest, &JobSelection::All)
        .expect_err("relative link targets must fail planning");

    assert!(matches!(
        error,
        ExecutionPlanError::Planning(PlanningError::RelativeLinkTarget { link, target })
            if link == "invalid" && target == Path::new("relative/target")
    ));
}

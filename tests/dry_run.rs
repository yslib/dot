mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use dot::action::ExecutionEnvironment;
use dot::dry_run;
use dot::interpolation::{DotPaths, InterpolationError, XdgPaths};
use dot::manifest::EffectiveManifest;
use dot::plan::{ExecutionPlanner, PlannedProviderInstall, PlanningError};
use dot::platform::PlatformInfo;
use dot::report::{
    ItemStatus, PackageSource, ProviderPackageSource, ReportCommand, ReportStatus, ReportSubject,
};
use dot::schema::{
    Config, EnvironmentName, LinkConflict, LinkMissingParent, ResolvedEnvironmentPatch,
    ResolvedString,
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
    environment
}

fn dot_paths() -> DotPaths<'static> {
    DotPaths::new(
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
    EffectiveManifest::select(&config, &platform(), Some(target), profile)
        .expect("test manifest should select")
}

#[test]
fn plans_only_selected_effective_records_and_defers_unused_provider_installs() {
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
        .plan(&manifest)
        .expect("deferred source expression errors must not affect the selected plan");

    assert_eq!(plan.target(), "selected");
    assert_eq!(plan.profile(), Some("chosen"));
    assert_eq!(
        plan.providers()
            .iter()
            .map(|provider| provider.id())
            .collect::<Vec<_>>(),
        ["shared", "unused-broken"]
    );
    assert_eq!(
        plan.providers()[0].probe().program.value(),
        "selected-probe"
    );
    assert!(plan.providers()[0].activate().is_none());
    assert!(plan.providers()[0].ensure().is_empty());
    assert_eq!(plan.providers()[1].probe().program.value(), "unused-probe");
    assert_eq!(
        plan.provider_installs()
            .iter()
            .map(PlannedProviderInstall::id)
            .collect::<Vec<_>>(),
        ["shared-package"]
    );
    assert_eq!(
        plan.provider_installs()[0].install().program.value(),
        "selected-install"
    );
    assert!(plan.manual_packages().is_empty());
    assert_eq!(
        plan.actions()
            .iter()
            .map(|action| action.id())
            .collect::<Vec<_>>(),
        ["shared-action"]
    );
    assert_eq!(
        plan.actions()[0].action().exec.program.value(),
        "selected-action"
    );
    assert!(plan.actions()[0].action().check.is_none());
    assert_eq!(
        plan.links()
            .iter()
            .map(|link| link.id())
            .collect::<Vec<_>>(),
        ["shared-link"]
    );
    assert_eq!(plan.links()[0].source(), config_path("selected-source"));
    assert_eq!(
        plan.links()[0].target(),
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
            "action `malformed-action`",
            ExpectedError::UnclosedResolver,
        ),
        (
            "unknown",
            "action `unknown-action`",
            ExpectedError::UnknownResolver,
        ),
        (
            "unavailable",
            "action `unavailable-action`",
            ExpectedError::ResolverUnavailable,
        ),
        (
            "wrong-output",
            "provider `catalog` install unit `wrong-output-package`",
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
            .plan(&manifest)
            .expect_err("the selected expression error must fail planning");
        let PlanningError::Interpolation { context, source } = error else {
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
fn rejects_an_unknown_provider_before_invalid_literal_provider_args() {
    let manifest = select_fixture("dry-run/invalid-unknown-provider-before-args.toml");
    let environment = environment();
    let xdg = XdgPaths::detect();
    let platform = platform();
    let planner = ExecutionPlanner::new(&environment, dot_paths(), &xdg, &platform);

    let error = planner
        .plan(&manifest)
        .expect_err("provider lookup must precede provider_args validation");

    assert!(matches!(
        error,
        PlanningError::UnknownProvider { package, provider }
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

    let plan = planner.plan(&manifest).expect("execution should plan");

    assert_eq!(plan.providers().len(), 1);
    assert_eq!(plan.providers()[0].id(), "brew");
    assert_eq!(
        plan.providers()[0].probe().program.value(),
        "/opt/homebrew/bin/brew"
    );
    assert_eq!(plan.providers()[0].ensure().len(), 1);

    assert_eq!(plan.provider_installs().len(), 3);

    let alpha = &plan.provider_installs()[0];
    assert!(matches!(alpha, PlannedProviderInstall::Single(_)));
    assert_eq!(alpha.id(), "alpha");
    assert_eq!(alpha.provider(), "brew");
    assert_eq!(alpha.provider_args(), &[] as &[String]);
    assert_eq!(alpha.names(), &[String::from("alpha")]);
    assert_eq!(
        alpha
            .install()
            .args
            .iter()
            .map(ResolvedString::value)
            .collect::<Vec<_>>(),
        vec!["before", "middle", "alpha", "after"]
    );

    let beta = &plan.provider_installs()[1];
    assert!(matches!(beta, PlannedProviderInstall::Single(_)));
    assert_eq!(beta.id(), "beta");
    assert_eq!(beta.names(), &[String::from("beta")]);
    assert_eq!(
        beta.install()
            .args
            .iter()
            .map(ResolvedString::value)
            .collect::<Vec<_>>(),
        vec!["before", "middle", "beta", "after"]
    );

    let fonts = &plan.provider_installs()[2];
    assert!(matches!(fonts, PlannedProviderInstall::Batch(_)));
    assert_eq!(fonts.id(), "fonts");
    assert_eq!(fonts.provider_args(), &[String::from("--cask")]);
    assert_eq!(
        fonts.names(),
        &[String::from("font-one"), "font-two".into()]
    );
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
    let plan = planner.plan(&manifest).expect("execution should plan");

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
        .plan(&manifest)
        .expect_err("an empty batch must fail");

    assert!(matches!(
        error,
        PlanningError::EmptyPackageBatch { package } if package == "empty-tools"
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
        .plan(&manifest)
        .expect_err("a duplicate batch name must fail");

    assert!(matches!(
        error,
        PlanningError::DuplicatePackageBatchName { package, name }
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
            .plan(&manifest)
            .expect_err("nonempty provider args must not be silently discarded");

        assert!(
            matches!(
                error,
                PlanningError::ProviderArgsResolverCount {
                    ref provider,
                    actual,
                }
                    if provider == "brew" && actual == expected_count
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

    let plan = planner.plan(&manifest).expect("execution should plan");

    assert_eq!(plan.manual_packages().len(), 1);
    assert_eq!(plan.manual_packages()[0].id(), "manual-tool");
    assert_eq!(
        plan.manual_packages()[0]
            .install()
            .check
            .as_ref()
            .unwrap()
            .program
            .value(),
        "/opt/bin/manual-tool"
    );
    assert_eq!(
        plan.manual_packages()[0].install().exec.program.value(),
        "bash"
    );

    assert_eq!(plan.actions().len(), 1);
    assert_eq!(plan.actions()[0].id(), "configure");
    assert_eq!(
        plan.actions()[0].action().exec.args[0].value(),
        config_template_path("scripts/configure.sh")
    );

    assert_eq!(plan.links().len(), 1);
    let link = &plan.links()[0];
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

    let plan = planner.plan(&manifest).expect("execution should plan");
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
            if matches!(
                &package.source,
                PackageSource::Provider(ProviderPackageSource::Single { provider, .. })
                    if provider == "system"
            )
    ));
    assert_eq!(report.items[1].id, "alpha");
    assert!(matches!(
        &report.items[2].subject,
        ReportSubject::Package(package)
            if matches!(&package.source, PackageSource::Manual { .. })
    ));
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
        .plan(&manifest)
        .expect_err("unknown providers must fail planning");

    assert!(matches!(
        error,
        PlanningError::UnknownProvider { package, provider }
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
        .plan(&manifest)
        .expect_err("relative link targets must fail planning");

    assert!(matches!(
        error,
        PlanningError::RelativeLinkTarget { link, target }
            if link == "invalid" && target == Path::new("relative/target")
    ));
}

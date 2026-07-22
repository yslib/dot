mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use dot::action::ExecutionEnvironment;
use dot::dry_run;
use dot::interpolation::{DotPaths, XdgPaths};
use dot::manifest::EffectiveManifest;
use dot::plan::{ExecutionPlanner, PlannedProviderInstall, PlanningError};
use dot::platform::PlatformInfo;
use dot::report::{ItemStatus, PackageSource, ReportCommand, ReportStatus, ReportSubject};
use dot::schema::{
    Config, EnvironmentName, EnvironmentPatch, LinkConflict, LinkMissingParent, ScalarTemplate,
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
        .apply_patch(&EnvironmentPatch {
            path_prepend: None,
            path_append: None,
            variables: BTreeMap::from([
                (
                    EnvironmentName::new("HOME").expect("test name should be valid"),
                    ScalarTemplate::from(TEST_HOME),
                ),
                (
                    EnvironmentName::new("ROOT").expect("test name should be valid"),
                    ScalarTemplate::from("/opt"),
                ),
                (
                    EnvironmentName::new("RUNNER").expect("test name should be valid"),
                    ScalarTemplate::from("bash"),
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
    let input = fixture::read(name);
    let config: Config = toml::from_str(&input).expect("test config should deserialize");
    EffectiveManifest::select(&config, &platform(), Some("machine"), None)
        .expect("test manifest should select")
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
        plan.providers()[0].probe().program.as_str(),
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
            .map(ScalarTemplate::as_str)
            .collect::<Vec<_>>(),
        vec!["install", "alpha"]
    );

    let beta = &plan.provider_installs()[1];
    assert!(matches!(beta, PlannedProviderInstall::Single(_)));
    assert_eq!(beta.id(), "beta");
    assert_eq!(beta.names(), &[String::from("beta")]);
    assert_eq!(
        beta.install()
            .args
            .iter()
            .map(ScalarTemplate::as_str)
            .collect::<Vec<_>>(),
        vec!["install", "beta"]
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
            .map(ScalarTemplate::as_str)
            .collect::<Vec<_>>(),
        vec!["install", "--cask", "font-one", "font-two"]
    );
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
fn nonempty_provider_args_require_one_install_list_resolver() {
    let manifest = select_fixture("dry-run/invalid-provider-args-resolver.toml");
    let environment = environment();
    let xdg = XdgPaths::detect();
    let platform = platform();
    let planner = ExecutionPlanner::new(&environment, dot_paths(), &xdg, &platform);

    let error = planner
        .plan(&manifest)
        .expect_err("nonempty provider args must not be silently discarded");

    assert!(
        error
            .to_string()
            .contains("exactly one `${package:provider_args}`")
    );
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
            .as_str(),
        "/opt/bin/manual-tool"
    );
    assert_eq!(
        plan.manual_packages()[0].install().exec.program.as_str(),
        "bash"
    );

    assert_eq!(plan.actions().len(), 1);
    assert_eq!(plan.actions()[0].id(), "configure");
    assert_eq!(
        plan.actions()[0].action().exec.args[0].as_str(),
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
                PackageSource::Provider { provider, .. } if provider == "system"
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

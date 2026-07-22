mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use dot::action::ExecutionEnvironment;
use dot::dry_run;
use dot::interpolation::{DotPaths, XdgPaths};
use dot::manifest::EffectiveManifest;
use dot::plan::{ExecutionPlanner, PlanningError};
use dot::platform::PlatformInfo;
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
fn groups_provider_packages_and_resolves_each_batch_environment() {
    let manifest = select_fixture("dry-run/valid-provider-batches.toml");
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

    assert_eq!(plan.provider_batches().len(), 2);
    let default = &plan.provider_batches()[0];
    assert_eq!(default.provider(), "brew");
    assert_eq!(default.provider_args(), &[] as &[String]);
    assert_eq!(default.packages(), &[String::from("alpha"), "beta".into()]);
    assert_eq!(
        default
            .install()
            .args
            .iter()
            .map(ScalarTemplate::as_str)
            .collect::<Vec<_>>(),
        vec!["install", "alpha", "beta"]
    );

    let cask = &plan.provider_batches()[1];
    assert_eq!(cask.provider_args(), &[String::from("--cask")]);
    assert_eq!(
        cask.packages(),
        &[String::from("font-one"), "font-two".into()]
    );
    assert_eq!(
        cask.install()
            .args
            .iter()
            .map(ScalarTemplate::as_str)
            .collect::<Vec<_>>(),
        vec!["install", "--cask", "font-one", "font-two"]
    );
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
fn renders_a_resolved_human_readable_execution_plan() {
    let manifest = select_fixture("dry-run/valid-human-readable-plan.toml");
    let environment = environment();
    let xdg = XdgPaths::detect();
    let platform = platform();
    let planner = ExecutionPlanner::new(&environment, dot_paths(), &xdg, &platform);

    let plan = planner.plan(&manifest).expect("execution should plan");
    let rendered = dry_run::display(&plan).to_string();

    assert!(rendered.contains("target: machine"));
    assert!(rendered.contains("profile: <root>"));
    assert!(rendered.contains("platform: linux/x86_64"));
    assert!(rendered.contains("providers:\n  system"));
    assert!(rendered.contains("path_prepend: [\"/opt/bin\"]"));
    assert!(rendered.contains("provider packages:\n  system"));
    assert!(rendered.contains("packages: [\"alpha\"]"));
    assert!(rendered.contains("manual packages:\n  manual"));
    assert!(rendered.contains("actions:\n  configure"));
    assert!(rendered.contains("links:\n  gitconfig:"));
    let link = &plan.links()[0];
    assert!(
        rendered.contains(&format!("{:?}", link.source().display().to_string())),
        "{rendered}"
    );
    assert!(
        rendered.contains(&format!("{:?}", link.target().display().to_string())),
        "{rendered}"
    );
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

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
        Path::new("/repo/dot.toml"),
        Path::new("/repo"),
        Path::new("/work"),
    )
}

fn gitconfig_target() -> PathBuf {
    PathBuf::from(format!("{TEST_HOME}/.gitconfig"))
}

fn select(input: &str) -> EffectiveManifest {
    let config: Config = toml::from_str(input).expect("test config should deserialize");
    EffectiveManifest::select(&config, &platform(), Some("machine"), None)
        .expect("test manifest should select")
}

#[test]
fn groups_provider_packages_and_resolves_each_batch_environment() {
    let manifest = select(
        r#"
        [targets.machine]
        platform = { os = "linux" }

        [targets.machine.providers.brew]
        activate = { variables = { BREW = "${env:ROOT}/homebrew/bin/brew" } }
        probe = { program = "${env:BREW}", args = ["--version"] }
        ensure = { program = "bootstrap-brew" }
        install = { program = "${env:BREW}", args = ["install", "${package:provider_args}", "${package:names}"] }

        [targets.machine.packages]
        alpha = { provider = "brew" }
        beta = { provider = "brew" }
        font-one = { provider = "brew", provider_args = ["--cask"] }
        font-two = { provider = "brew", provider_args = ["--cask"] }
        "#,
    );
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
    let manifest = select(
        r#"
        [targets.machine]
        platform = { os = "linux" }

        [targets.machine.providers.brew]
        probe = { program = "brew", args = ["--version"] }
        install = { program = "brew", args = ["install", "${package:names}"] }

        [targets.machine.packages]
        app = { provider = "brew", provider_args = ["--cask"] }
        "#,
    );
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
    let manifest = select(
        r#"
        [targets.machine]
        platform = { os = "linux" }

        [targets.machine.packages.manual-tool]
        install = {
          check = { program = "${env:ROOT}/bin/manual-tool", args = ["--version"] },
          exec = { program = "${env:RUNNER}", args = ["${dot:config_dir}/scripts/install-tool.sh"], cwd = "${dot:config_dir}" }
        }

        [targets.machine.actions.configure]
        check = { program = "${env:ROOT}/bin/config-check" }
        exec = { program = "${env:RUNNER}", args = ["${dot:config_dir}/scripts/configure.sh"] }

        [targets.machine.links.gitconfig]
        source = "home/.gitconfig"
        target = "${env:HOME}/.gitconfig"
        on_conflict = "error"
        on_missing_parent = "skip"
        "#,
    );
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
        "/repo/scripts/configure.sh"
    );

    assert_eq!(plan.links().len(), 1);
    let link = &plan.links()[0];
    assert_eq!(link.id(), "gitconfig");
    assert_eq!(link.source(), Path::new("/repo/home/.gitconfig"));
    assert_eq!(link.target(), gitconfig_target());
    assert_eq!(link.on_conflict(), LinkConflict::Error);
    assert_eq!(link.on_missing_parent(), LinkMissingParent::Skip);
}

#[test]
fn renders_a_resolved_human_readable_execution_plan() {
    let manifest = select(
        r#"
        [targets.machine]
        platform = { os = "linux" }

        [targets.machine.providers.system]
        activate = { path_prepend = ["${env:ROOT}/bin"] }
        probe = { program = "tool", args = ["--version"] }
        install = { program = "tool", args = ["install", "${package:names}"] }

        [targets.machine.packages]
        alpha = { provider = "system" }
        manual = { install = { exec = { program = "${env:RUNNER}", args = ["install.sh"] } } }

        [targets.machine.actions.configure]
        exec = { program = "${env:RUNNER}", args = ["configure.sh"] }

        [targets.machine.links.gitconfig]
        source = "home/.gitconfig"
        target = "${env:HOME}/.gitconfig"
        "#,
    );
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
    assert!(rendered.contains("/repo/home/.gitconfig"));
    assert!(rendered.contains(&gitconfig_target().display().to_string()));
}

#[test]
fn rejects_a_package_that_references_an_unknown_effective_provider() {
    let manifest = select(
        r#"
        [targets.machine]
        platform = { os = "linux" }

        [targets.machine.packages]
        alpha = { provider = "missing" }
        "#,
    );
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
    let manifest = select(
        r#"
        [targets.machine]
        platform = { os = "linux" }

        [targets.machine.links.invalid]
        source = "source"
        target = "relative/target"
        "#,
    );
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

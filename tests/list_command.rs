//! End-to-end list command behavior.

mod support;

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use dot::job::JobSelector;
use support::fixture;

static NEXT_EXACT_WORKSPACE: AtomicU64 = AtomicU64::new(0);

fn dot(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_dot"))
        .args(args)
        .arg("--config")
        .arg(fixture::path("list/valid-catalog.toml"))
        .output()
        .expect("dot should start")
}

fn success_stdout(args: &[&str]) -> String {
    let output = dot(args);
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("list output should be UTF-8")
}

struct ExactCatalogWorkspace {
    directory: PathBuf,
    manifest: PathBuf,
}

impl ExactCatalogWorkspace {
    fn new() -> Self {
        let sequence = NEXT_EXACT_WORKSPACE.fetch_add(1, Ordering::Relaxed);
        let directory = env::temp_dir().join(format!(
            "dot-list-command-exact-{}-{sequence}",
            process::id()
        ));
        fs::create_dir(&directory).expect("exact-selection workspace should be created");
        fs::write(directory.join("selected-source.txt"), "selected")
            .expect("selected link source should be written");
        fs::write(directory.join("unselected-source.txt"), "unselected")
            .expect("unselected link source should be written");
        let manifest = directory.join("dot.toml");
        let program = format!(
            "{:?}",
            env::current_exe()
                .expect("test executable should have a path")
                .to_string_lossy()
        );
        let contents = fixture::read("selection/valid-exact-command-template.toml")
            .replace("__OS__", env::consts::OS)
            .replace("__PROGRAM__", &program);
        fs::write(&manifest, contents).expect("exact-selection manifest should be written");
        Self {
            directory,
            manifest,
        }
    }
}

impl Drop for ExactCatalogWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn report_job_rows(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .filter_map(|line| {
            let cells = line
                .trim_matches('│')
                .split('┆')
                .map(str::trim)
                .collect::<Vec<_>>();
            let [kind, id, ..] = cells.as_slice() else {
                return None;
            };
            matches!(*kind, "provider" | "package" | "action" | "link")
                .then(|| format!("{kind}:{id}"))
        })
        .collect()
}

fn expected_exact_rows(selector: &JobSelector) -> Vec<String> {
    let mut rows = Vec::new();
    if let JobSelector::Package(package) = selector {
        match package.as_str() {
            "selected" | "same-provider-other" => {
                rows.push("provider:selected-provider".to_owned());
            }
            "other-provider-package" => {
                rows.push("provider:unrelated-provider".to_owned());
            }
            "manual-package" => {}
            unknown => panic!("unexpected package selector in exact fixture: {unknown}"),
        }
    }
    rows.push(selector.to_string());
    rows
}

#[test]
fn listed_job_selectors_round_trip_through_exact_dry_run() {
    let workspace = ExactCatalogWorkspace::new();
    let list = Command::new(env!("CARGO_BIN_EXE_dot"))
        .arg("--config")
        .arg(&workspace.manifest)
        .args(["list", "jobs"])
        .env_remove("DOT_INTENTIONALLY_MISSING")
        .output()
        .expect("dot list jobs should start");
    assert!(
        list.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&list.stdout),
        String::from_utf8_lossy(&list.stderr)
    );
    let list_stdout = String::from_utf8(list.stdout).expect("job list should be UTF-8");
    let fields = list_stdout
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            line.split_once('\t')
                .expect("every job record should contain multiple TSV fields")
                .0
        })
        .collect::<Vec<_>>();
    let unique_fields = fields.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(
        unique_fields,
        BTreeSet::from([
            "package:manual-package",
            "package:other-provider-package",
            "package:same-provider-other",
            "package:selected",
            "action:missing-runtime",
            "action:runnable-action",
            "link:selected-link",
            "link:unselected-link",
        ]),
        "{list_stdout}"
    );
    assert_eq!(
        fields.len(),
        unique_fields.len(),
        "job selectors must be unique:\n{list_stdout}"
    );

    for field in fields {
        assert!(
            !field.starts_with("provider:"),
            "providers must not appear as selectable TSV records: {field}"
        );
        let selector = field
            .parse::<JobSelector>()
            .unwrap_or_else(|error| panic!("listed selector `{field}` should parse: {error}"));
        assert_eq!(
            selector.to_string(),
            field,
            "the first field should use canonical selector syntax"
        );

        let output = Command::new(env!("CARGO_BIN_EXE_dot"))
            .arg("--config")
            .arg(&workspace.manifest)
            .args(["dry-run", "--job", field])
            .env(
                "DOT_INTENTIONALLY_MISSING",
                env::current_exe().expect("test executable should have a path"),
            )
            .output()
            .expect("dot dry-run should start");
        let stdout = String::from_utf8(output.stdout).expect("dry-run report should be UTF-8");
        assert!(
            output.status.success(),
            "selector: {field}\nstdout:\n{stdout}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            report_job_rows(&stdout),
            expected_exact_rows(&selector),
            "selector: {field}\n{stdout}"
        );
    }
}

#[test]
fn list_targets_filters_compatible_targets_by_default() {
    assert_eq!(
        success_stdout(&["list", "targets"]),
        "current\tcompatible\tlinux,macos,windows\t\t\t\t\n"
    );
}

#[test]
fn list_targets_all_reports_fixed_columns_labels_and_btree_order() {
    assert_eq!(
        success_stdout(&["list", "targets", "--all"]),
        concat!(
            "current\tcompatible\tlinux,macos,windows\t\t\t\t\n",
            "never\tincompatible\tnever-os\tx86_64,aarch64\tnever-distro\t",
            "never-family,other-family\tnever-environment\n",
        )
    );
}

#[test]
fn list_profiles_allows_explicit_incompatible_target_and_uses_preorder() {
    assert_eq!(
        success_stdout(&["list", "profiles", "--target", "never"]),
        concat!(
            "@root\t<root>\t0\n",
            "desktop\tdesktop\t1\n",
            "laptop\tdesktop/laptop\t2\n",
            "server\tserver\t1\n",
        )
    );
}

#[test]
fn list_jobs_inspects_unresolved_effective_jobs_without_provider_rows() {
    assert_eq!(
        success_stdout(&["list", "jobs", "--target", "never"]),
        concat!(
            "package:alpha\tpackage\talpha\tsystem\talpha\n",
            "package:bundle\tpackage\tbundle\tsystem\tbat,fd,fzf\n",
            "package:manual\tpackage\tmanual\tmanual\tmanual\\\\runner\\tphase\n",
            "action:configure\taction\tconfigure\texec\tconfigure\\nrunner\n",
            "action:remote-config\taction\tremote-config\tfetch\t",
            "https://example.com/config.toml -> configs/app.toml\n",
            "link:config\tlink\tconfig\tbuiltin\thome\\\\config -> ",
            "${env:HOME}/.config/tool\n",
        )
    );
}

#[test]
fn list_jobs_merges_a_nested_profile_in_canonical_job_order() {
    assert_eq!(
        success_stdout(&["list", "jobs", "--target", "never", "--profile", "laptop",]),
        concat!(
            "package:alpha\tpackage\talpha\tsystem\talpha\n",
            "package:bundle\tpackage\tbundle\tsystem\tbat,fd,fzf\n",
            "package:desktop\tpackage\tdesktop\tsystem\tdesktop\n",
            "package:manual\tpackage\tmanual\tmanual\tmanual\\\\runner\\tphase\n",
            "action:configure\taction\tconfigure\texec\tconfigure\\nrunner\n",
            "action:laptop\taction\tlaptop\texec\tlaptop-runner\n",
            "action:remote-config\taction\tremote-config\tfetch\t",
            "https://example.com/config.toml -> configs/app.toml\n",
            "link:config\tlink\tconfig\tbuiltin\thome\\\\config -> ",
            "${env:HOME}/.config/tool\n",
        )
    );
}

#[test]
fn list_jobs_preserves_an_unresolved_fetch_source_with_a_missing_environment_value() {
    let output = Command::new(env!("CARGO_BIN_EXE_dot"))
        .args(["list", "jobs", "--target", "never", "--profile", "server"])
        .arg("--config")
        .arg(fixture::path("list/valid-catalog.toml"))
        .env_remove("DOT_INTENTIONALLY_MISSING")
        .output()
        .expect("dot list jobs should start");

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("list output should be UTF-8");
    assert!(
        stdout.contains(concat!(
            "action:remote-config\taction\tremote-config\tfetch\t",
            "${env:DOT_INTENTIONALLY_MISSING} -> configs/server.toml\n"
        )),
        "{stdout}"
    );
}

#[test]
fn explicit_root_matches_an_absent_profile() {
    assert_eq!(
        success_stdout(&["list", "jobs", "--target", "never", "--profile", "@root",]),
        success_stdout(&["list", "jobs", "--target", "never"])
    );
}

#[test]
fn list_errors_leave_stdout_empty() {
    for args in [
        vec!["list", "jobs", "--target", "unknown"],
        vec!["list", "jobs", "--target", "never", "--profile", "unknown"],
    ] {
        let output = dot(&args);
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty(), "{:?}", output.stdout);
        assert!(!output.stderr.is_empty());
    }

    let output = Command::new(env!("CARGO_BIN_EXE_dot"))
        .args(["list", "targets", "--config"])
        .arg(fixture::path("config/invalid-syntax.toml"))
        .output()
        .expect("dot should start");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty(), "{:?}", output.stdout);
    assert!(!output.stderr.is_empty());
}

#[cfg(feature = "dev-platform-override")]
const NEVER_PLATFORM: &str = r#"{ os = "never-os", arch = "x86_64", distro = "never-distro", distro_family = "never-family", environment = "never-environment" }"#;

#[cfg(feature = "dev-platform-override")]
#[test]
fn list_targets_warns_that_override_affects_labels_and_filtering() {
    let output = dot(&["list", "targets", "--platform", NEVER_PLATFORM]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "{stderr}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        concat!(
            "never\tcompatible\tnever-os\tx86_64,aarch64\tnever-distro\t",
            "never-family,other-family\tnever-environment\n",
        )
    );
    assert!(
        stderr.contains("compatibility labels and default filtering"),
        "{stderr}"
    );
}

#[cfg(feature = "dev-platform-override")]
#[test]
fn profile_and_job_lists_warn_that_override_affects_only_target_inference() {
    for leaf in ["profiles", "jobs"] {
        let output = dot(&["list", leaf, "--platform", NEVER_PLATFORM]);
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(output.status.success(), "{stderr}");
        assert!(!output.stdout.is_empty());
        assert!(
            stderr.contains("--platform affects only omitted-target inference"),
            "{stderr}"
        );
    }
}

mod support;

use std::process::Command;

use support::fixture;

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
            "link:config\tlink\tconfig\tbuiltin\thome\\\\config -> ",
            "${env:HOME}/.config/tool\n",
        )
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

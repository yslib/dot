use std::collections::BTreeSet;

use dot::inspect::Inspector;
use dot::output::{TsvRecord, TsvRenderer};
use dot::platform::PlatformInfo;
use dot::schema::SelectorIdentifier;
use dot::selection::{ProfileSelection, ScopeSelection};

const SOURCE: &str = include_str!("fixtures/list/valid-catalog.toml");

fn platform() -> PlatformInfo {
    PlatformInfo {
        os: "linux".to_owned(),
        arch: "x86_64".to_owned(),
        distro: Some("test-linux".to_owned()),
        distro_families: BTreeSet::from(["test-family".to_owned()]),
        environments: BTreeSet::from(["native".to_owned()]),
    }
}

fn render<R: TsvRecord>(records: &[R]) -> String {
    TsvRenderer
        .render_to_string(records)
        .expect("records should render")
}

#[test]
fn target_tsv_includes_declared_targets_and_compatibility_labels() {
    let records = {
        let config = dot::schema::Config::parse(SOURCE).expect("configuration should parse");
        Inspector::new(&config, &platform()).targets(true)
    };
    let output = render(&records);

    assert_eq!(
        output,
        concat!(
            "current\tcompatible\tlinux,macos,windows\t\t\t\t\n",
            "never\tincompatible\tnever-os\tx86_64,aarch64\tnever-distro\t",
            "never-family,other-family\tnever-environment\n",
        )
    );
}

#[test]
fn incompatible_target_profile_tsv_includes_root_and_nested_profiles() {
    let records = {
        let config = dot::schema::Config::parse(SOURCE).expect("configuration should parse");
        let target = SelectorIdentifier::new("never").expect("target should parse");
        Inspector::new(&config, &platform())
            .profiles(Some(&target))
            .expect("profiles should list")
    };
    let output = render(&records);

    assert_eq!(
        output,
        concat!(
            "@root\t<root>\t0\n",
            "desktop\tdesktop\t1\n",
            "laptop\tdesktop/laptop\t2\n",
            "server\tserver\t1\n",
        )
    );
}

#[test]
fn root_and_named_profile_jobs_include_effective_records_and_details() {
    let config = dot::schema::Config::parse(SOURCE).expect("configuration should parse");
    let target = SelectorIdentifier::new("never").expect("target should parse");
    let root_scope = ScopeSelection {
        target: Some(target.clone()),
        profile: ProfileSelection::Root,
    };
    let server_scope = ScopeSelection {
        target: Some(target),
        profile: ProfileSelection::Named(
            SelectorIdentifier::new("server").expect("profile should parse"),
        ),
    };
    let (root_records, server_records) = {
        let platform = platform();
        let inspector = Inspector::new(&config, &platform);
        (
            inspector.jobs(&root_scope).expect("root jobs should list"),
            inspector
                .jobs(&server_scope)
                .expect("named-profile jobs should list"),
        )
    };
    let root = render(&root_records);
    let server = render(&server_records);

    assert_eq!(
        root,
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
    assert_eq!(
        server,
        concat!(
            "package:alpha\tpackage\talpha\tsystem\talpha\n",
            "package:bundle\tpackage\tbundle\tsystem\tbat,fd,fzf\n",
            "package:manual\tpackage\tmanual\tmanual\tmanual\\\\runner\\tphase\n",
            "action:configure\taction\tconfigure\texec\tconfigure\\nrunner\n",
            "action:remote-config\taction\tremote-config\tfetch\t",
            "${env:DOT_INTENTIONALLY_MISSING} -> configs/server.toml\n",
            "link:config\tlink\tconfig\tbuiltin\thome\\\\config -> ",
            "${env:HOME}/.config/tool\n",
            "link:server\tlink\tserver\tbuiltin\tserver-source -> /server-target\n",
        )
    );
}

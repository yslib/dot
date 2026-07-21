use std::collections::BTreeMap;
use std::env;
use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process;

use dot::action::{ExecutionEnvironment, ExecutionError, IoMode, PreparedCommand, ProcessExecutor};
use dot::schema::{EnvironmentName, EnvironmentPatch, ExecAction, OneOrMany, ScalarTemplate};

fn patch(
    prepend: Option<OneOrMany<ScalarTemplate>>,
    append: Option<OneOrMany<ScalarTemplate>>,
    variables: &[(&str, &str)],
) -> EnvironmentPatch {
    EnvironmentPatch {
        path_prepend: prepend,
        path_append: append,
        variables: variables
            .iter()
            .map(|(name, value)| {
                (
                    EnvironmentName::new(*name).expect("test environment name should be valid"),
                    ScalarTemplate::from(*value),
                )
            })
            .collect::<BTreeMap<_, _>>(),
    }
}

fn helper_action(mode: &str, cwd: Option<&Path>) -> ExecAction {
    ExecAction {
        kind: None,
        program: env::current_exe()
            .expect("test executable should have a path")
            .into_os_string()
            .into_string()
            .expect("test executable path should be Unicode")
            .into(),
        args: vec![
            "--exact".into(),
            "helper_process".into(),
            "--nocapture".into(),
        ],
        cwd: cwd.map(|path| path.to_string_lossy().into_owned().into()),
        env: Some(patch(None, None, &[("DOT_ACTION_TEST_HELPER", mode)])),
    }
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[test]
fn environment_patch_overrides_variables_and_wraps_the_effective_path() {
    let original = PathBuf::from("original-bin");
    let original_path = env::join_paths([&original])
        .expect("test PATH should be valid")
        .to_string_lossy()
        .into_owned();
    let mut environment = ExecutionEnvironment::empty();
    environment
        .apply_patch(&patch(
            None,
            None,
            &[("PATH", &original_path), ("DOT_VALUE", "base")],
        ))
        .expect("base patch should apply");

    environment
        .apply_patch(&patch(
            Some(OneOrMany::Many(vec![
                "first-bin".into(),
                "second-bin".into(),
            ])),
            Some(OneOrMany::One("last-bin".into())),
            &[("DOT_VALUE", "action")],
        ))
        .expect("action patch should apply");

    assert_eq!(environment.get("DOT_VALUE"), Some(OsStr::new("action")));
    let paths =
        env::split_paths(environment.get("PATH").expect("PATH should exist")).collect::<Vec<_>>();
    assert_eq!(
        paths,
        vec![
            PathBuf::from("first-bin"),
            PathBuf::from("second-bin"),
            original,
            PathBuf::from("last-bin"),
        ]
    );
}

#[test]
fn prepared_command_copies_process_fields_and_layers_action_environment() {
    let mut base = ExecutionEnvironment::empty();
    base.apply_patch(&patch(
        None,
        None,
        &[("BASE_ONLY", "present"), ("SHARED", "base")],
    ))
    .expect("base environment should be valid");
    let action = ExecAction {
        kind: None,
        program: "example-program".into(),
        args: vec!["first".into(), "two words".into(), "".into()],
        cwd: Some("working-directory".into()),
        env: Some(patch(None, None, &[("SHARED", "action")])),
    };

    let command = PreparedCommand::from_exec_action(&action, &base)
        .expect("command preparation should succeed");

    assert_eq!(command.program(), OsStr::new("example-program"));
    assert_eq!(
        command.args(),
        &[
            OsString::from("first"),
            OsString::from("two words"),
            OsString::from(""),
        ]
    );
    assert_eq!(command.cwd(), Some(Path::new("working-directory")));
    assert_eq!(
        command.environment().get("BASE_ONLY"),
        Some(OsStr::new("present"))
    );
    assert_eq!(
        command.environment().get("SHARED"),
        Some(OsStr::new("action"))
    );
    assert_eq!(base.get("SHARED"), Some(OsStr::new("base")));
}

#[test]
fn capture_returns_raw_output_and_preserves_a_nonzero_exit_status() {
    let command = PreparedCommand::from_exec_action(
        &helper_action("nonzero-output", None),
        &ExecutionEnvironment::empty(),
    )
    .expect("helper command should prepare");

    let result = ProcessExecutor::new()
        .execute(&command, IoMode::Capture)
        .expect("a nonzero exit is still an execution result");

    assert!(!result.success());
    assert_eq!(result.code(), Some(23));
    assert!(contains_bytes(
        result.stdout().expect("stdout should be captured"),
        &[b'o', b'u', b't', 0xff]
    ));
    assert!(contains_bytes(
        result.stderr().expect("stderr should be captured"),
        b"error-output"
    ));
}

#[test]
fn capture_uses_the_prepared_environment_and_working_directory() {
    let directory = env::temp_dir();
    let canonical_directory = directory
        .canonicalize()
        .expect("temporary directory should be canonicalizable");
    let mut action = helper_action("context", Some(&directory));
    action
        .env
        .as_mut()
        .expect("helper has an environment patch")
        .variables
        .insert(
            EnvironmentName::new("DOT_CONTEXT_VALUE").expect("test name should be valid"),
            "from-command".into(),
        );
    let command = PreparedCommand::from_exec_action(&action, &ExecutionEnvironment::empty())
        .expect("helper command should prepare");

    let result = ProcessExecutor::new()
        .execute(&command, IoMode::Capture)
        .expect("helper should execute");
    let stdout = String::from_utf8_lossy(result.stdout().expect("stdout should be captured"));

    assert!(stdout.contains("value=from-command"));
    assert!(stdout.contains(&format!("cwd={}", canonical_directory.display())));
}

#[test]
fn capture_connects_stdin_to_null() {
    let command = PreparedCommand::from_exec_action(
        &helper_action("read-stdin", None),
        &ExecutionEnvironment::empty(),
    )
    .expect("helper command should prepare");

    let result = ProcessExecutor::new()
        .execute(&command, IoMode::Capture)
        .expect("helper should execute");

    assert!(contains_bytes(
        result.stdout().expect("stdout should be captured"),
        b"stdin-bytes=0"
    ));
}

#[test]
fn inherit_returns_only_the_exit_status() {
    let command = PreparedCommand::from_exec_action(
        &helper_action("success", None),
        &ExecutionEnvironment::empty(),
    )
    .expect("helper command should prepare");

    let result = ProcessExecutor::new()
        .execute(&command, IoMode::Inherit)
        .expect("helper should execute");

    assert!(result.success());
    assert_eq!(result.code(), Some(0));
    assert_eq!(result.stdout(), None);
    assert_eq!(result.stderr(), None);
}

#[test]
fn reports_a_program_that_cannot_be_started() {
    let action = ExecAction {
        kind: None,
        program: "dot-program-that-must-not-exist-4d02a925".into(),
        args: Vec::new(),
        cwd: None,
        env: None,
    };
    let command = PreparedCommand::from_exec_action(&action, &ExecutionEnvironment::capture())
        .expect("command should prepare even when its program is missing");

    let error = ProcessExecutor::new()
        .execute(&command, IoMode::Capture)
        .expect_err("missing program should fail to start");

    assert!(matches!(error, ExecutionError::Spawn { .. }));
    assert!(error.to_string().contains(action.program.as_str()));
}

#[test]
fn helper_process() {
    let Ok(mode) = env::var("DOT_ACTION_TEST_HELPER") else {
        return;
    };

    match mode.as_str() {
        "nonzero-output" => {
            std::io::stdout()
                .write_all(&[b'o', b'u', b't', 0xff])
                .expect("helper should write stdout");
            std::io::stderr()
                .write_all(b"error-output")
                .expect("helper should write stderr");
            process::exit(23);
        }
        "context" => {
            let cwd = env::current_dir().expect("helper should read cwd");
            let value = env::var("DOT_CONTEXT_VALUE").expect("helper should read environment");
            println!("cwd={}", cwd.display());
            println!("value={value}");
        }
        "read-stdin" => {
            let mut input = Vec::new();
            std::io::stdin()
                .read_to_end(&mut input)
                .expect("helper should read stdin");
            println!("stdin-bytes={}", input.len());
        }
        "success" => {}
        unknown => panic!("unknown helper mode: {unknown}"),
    }
}

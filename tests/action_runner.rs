use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};

use dot::action::ExecutionEnvironment;
use dot::action_runner::{ActionOutcome, ActionRunner, ActionStage};
use dot::schema::{
    EnvironmentName, ResolvedAction, ResolvedEnvironmentPatch, ResolvedExecAction, ResolvedString,
};

static NEXT_STATE: AtomicU64 = AtomicU64::new(0);

struct TempState {
    directory: PathBuf,
}

impl TempState {
    fn new() -> Self {
        let sequence = NEXT_STATE.fetch_add(1, Ordering::Relaxed);
        let directory =
            env::temp_dir().join(format!("dot-action-runner-{}-{sequence}", process::id()));
        fs::create_dir(&directory).expect("temporary state directory should be created");
        Self { directory }
    }

    fn marker(&self) -> PathBuf {
        self.directory.join("satisfied")
    }

    fn events(&self) -> PathBuf {
        self.directory.join("events")
    }

    fn recorded_events(&self) -> Vec<String> {
        fs::read_to_string(self.events())
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

impl Drop for TempState {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn helper_action(mode: &str, state: &TempState) -> ResolvedExecAction {
    let variables = [
        ("DOT_ACTION_RUNNER_MODE", mode.to_owned()),
        (
            "DOT_ACTION_RUNNER_MARKER",
            state.marker().to_string_lossy().into_owned(),
        ),
        (
            "DOT_ACTION_RUNNER_EVENTS",
            state.events().to_string_lossy().into_owned(),
        ),
    ]
    .into_iter()
    .map(|(name, value)| {
        (
            EnvironmentName::new(name).expect("test environment name should be valid"),
            ResolvedString::from(value),
        )
    })
    .collect::<BTreeMap<_, _>>();

    ResolvedExecAction {
        kind: None,
        program: env::current_exe()
            .expect("test executable should have a path")
            .to_string_lossy()
            .into_owned()
            .into(),
        args: vec![
            "--exact".into(),
            "helper_process".into(),
            "--nocapture".into(),
        ],
        cwd: None,
        env: Some(ResolvedEnvironmentPatch {
            path_prepend: None,
            path_append: None,
            variables,
        }),
    }
}

fn action(check: Option<ResolvedExecAction>, exec: ResolvedExecAction) -> ResolvedAction {
    ResolvedAction { check, exec }
}

#[test]
fn executes_an_action_without_a_check() {
    let state = TempState::new();
    let action = action(None, helper_action("exec-record", &state));

    let outcome = ActionRunner::new(&ExecutionEnvironment::empty())
        .run(&action)
        .expect("the action should execute");

    let ActionOutcome::Executed {
        initial_check,
        exec,
        post_check,
    } = outcome
    else {
        panic!("an unchecked action should execute");
    };
    assert!(initial_check.is_none());
    assert!(exec.success());
    assert!(exec.stdout().is_none());
    assert!(post_check.is_none());
    assert_eq!(state.recorded_events(), ["exec"]);
}

#[test]
fn skips_exec_when_the_initial_check_is_satisfied() {
    let state = TempState::new();
    fs::write(state.marker(), "ready").expect("marker should be written");
    let action = action(
        Some(helper_action("check-state", &state)),
        helper_action("exec-record", &state),
    );

    let outcome = ActionRunner::new(&ExecutionEnvironment::empty())
        .run(&action)
        .expect("a satisfied action should succeed");

    let ActionOutcome::AlreadySatisfied { check } = outcome else {
        panic!("a ready check should skip exec");
    };
    assert_eq!(check.code(), Some(0));
    assert!(check.stdout().is_some());
    assert_eq!(state.recorded_events(), ["check"]);
}

#[test]
fn executes_and_checks_again_when_the_initial_check_returns_one() {
    let state = TempState::new();
    let action = action(
        Some(helper_action("check-state", &state)),
        helper_action("exec-satisfy", &state),
    );

    let outcome = ActionRunner::new(&ExecutionEnvironment::empty())
        .run(&action)
        .expect("exec should satisfy the action");

    let ActionOutcome::Executed {
        initial_check,
        exec,
        post_check,
    } = outcome
    else {
        panic!("an unsatisfied action should execute");
    };
    assert_eq!(initial_check.and_then(|result| result.code()), Some(1));
    assert!(exec.success());
    assert_eq!(post_check.and_then(|result| result.code()), Some(0));
    assert_eq!(state.recorded_events(), ["check", "exec", "check"]);
}

#[test]
fn rejects_an_initial_check_exit_other_than_zero_or_one() {
    let state = TempState::new();
    let action = action(
        Some(helper_action("check-error", &state)),
        helper_action("exec-record", &state),
    );

    let error = ActionRunner::new(&ExecutionEnvironment::empty())
        .run(&action)
        .expect_err("an invalid check exit should fail");

    assert_eq!(error.stage(), ActionStage::InitialCheck);
    assert_eq!(
        error.exit_result().and_then(|result| result.code()),
        Some(23)
    );
    assert_eq!(state.recorded_events(), ["check"]);
}

#[test]
fn stops_before_post_check_when_exec_fails() {
    let state = TempState::new();
    let action = action(
        Some(helper_action("check-state", &state)),
        helper_action("exec-fail", &state),
    );

    let error = ActionRunner::new(&ExecutionEnvironment::empty())
        .run(&action)
        .expect_err("a failed exec should fail the action");

    assert_eq!(error.stage(), ActionStage::Exec);
    assert_eq!(
        error.exit_result().and_then(|result| result.code()),
        Some(17)
    );
    assert_eq!(state.recorded_events(), ["check", "exec"]);
}

#[test]
fn fails_when_post_check_is_not_satisfied() {
    let state = TempState::new();
    let action = action(
        Some(helper_action("check-state", &state)),
        helper_action("exec-record", &state),
    );

    let error = ActionRunner::new(&ExecutionEnvironment::empty())
        .run(&action)
        .expect_err("post-check must verify the desired state");

    assert_eq!(error.stage(), ActionStage::PostCheck);
    assert_eq!(
        error.exit_result().and_then(|result| result.code()),
        Some(1)
    );
    assert_eq!(state.recorded_events(), ["check", "exec", "check"]);
}

#[test]
fn helper_process() {
    let Ok(mode) = env::var("DOT_ACTION_RUNNER_MODE") else {
        return;
    };
    let marker = PathBuf::from(
        env::var_os("DOT_ACTION_RUNNER_MARKER").expect("helper marker path should be set"),
    );
    let events = PathBuf::from(
        env::var_os("DOT_ACTION_RUNNER_EVENTS").expect("helper event path should be set"),
    );

    match mode.as_str() {
        "check-state" => {
            record(&events, "check");
            println!("checked={}", marker.exists());
            if !marker.exists() {
                process::exit(1);
            }
        }
        "check-error" => {
            record(&events, "check");
            process::exit(23);
        }
        "exec-record" => record(&events, "exec"),
        "exec-satisfy" => {
            record(&events, "exec");
            fs::write(marker, "ready").expect("helper should write marker");
        }
        "exec-fail" => {
            record(&events, "exec");
            process::exit(17);
        }
        unknown => panic!("unknown helper mode: {unknown}"),
    }
}

fn record(path: &Path, event: &str) {
    use std::io::Write as _;

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("helper event file should open");
    writeln!(file, "{event}").expect("helper event should be recorded");
}

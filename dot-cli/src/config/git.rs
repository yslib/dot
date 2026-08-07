use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output};

const LOCATION_ENVIRONMENT: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_COMMON_DIR",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_SHALLOW_FILE",
    "GIT_CEILING_DIRECTORIES",
    "GIT_DISCOVERY_ACROSS_FILESYSTEM",
    "GIT_NAMESPACE",
];

pub(crate) fn prepare(repository: &str, worktree: &Path) -> Result<(), GitError> {
    match fs::symlink_metadata(worktree) {
        Ok(metadata) => validate_entry(repository, worktree, &metadata),
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            clone_repository(repository, worktree)?;
            let metadata = fs::symlink_metadata(worktree).map_err(|source| GitError::Inspect {
                worktree: worktree.to_path_buf(),
                source,
            })?;
            validate_entry(repository, worktree, &metadata)
        }
        Err(source) => Err(GitError::Inspect {
            worktree: worktree.to_path_buf(),
            source,
        }),
    }
}

fn clone_repository(repository: &str, worktree: &Path) -> Result<(), GitError> {
    let status = git_command()
        .arg("clone")
        .arg("--origin")
        .arg("origin")
        .arg("--")
        .arg(repository)
        .arg(worktree)
        .status()
        .map_err(|source| GitError::CloneLaunch {
            worktree: worktree.to_path_buf(),
            source,
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(GitError::CloneFailed {
            worktree: worktree.to_path_buf(),
            status,
        })
    }
}

fn validate_entry(
    repository: &str,
    worktree: &Path,
    metadata: &fs::Metadata,
) -> Result<(), GitError> {
    if !metadata.file_type().is_dir() {
        return Err(GitError::NotDirectory {
            worktree: worktree.to_path_buf(),
        });
    }

    let inside = checked_output(
        worktree,
        "determine whether the entry is a Git worktree",
        &["rev-parse", "--is-inside-work-tree"],
    )?;
    if strip_line_ending(&inside.stdout) != b"true" {
        return Err(GitError::NotWorktree {
            worktree: worktree.to_path_buf(),
            actual: render_bytes(strip_line_ending(&inside.stdout)),
        });
    }

    let prefix = checked_output(
        worktree,
        "determine the Git worktree root",
        &["rev-parse", "--show-prefix"],
    )?;
    let prefix = strip_line_ending(&prefix.stdout);
    if !prefix.is_empty() {
        return Err(GitError::NotWorktreeRoot {
            worktree: worktree.to_path_buf(),
            prefix: render_bytes(prefix),
        });
    }

    validate_origin(repository, worktree)
}

fn validate_origin(repository: &str, worktree: &Path) -> Result<(), GitError> {
    let output = git_output(
        worktree,
        "read remote.origin.url",
        &[
            "config",
            "--local",
            "--null",
            "--get-all",
            "remote.origin.url",
        ],
    )?;
    if !output.status.success() {
        if output.status.code() == Some(1) && output.stdout.is_empty() {
            return Err(GitError::MissingOrigin {
                worktree: worktree.to_path_buf(),
            });
        }
        return Err(command_failed(worktree, "read remote.origin.url", output));
    }

    let origin = parse_origin(&output.stdout).map_err(|error| match error {
        OriginOutputError::Missing => GitError::MissingOrigin {
            worktree: worktree.to_path_buf(),
        },
        OriginOutputError::Repeated => GitError::RepeatedOrigin {
            worktree: worktree.to_path_buf(),
        },
        OriginOutputError::Malformed => GitError::MalformedOrigin {
            worktree: worktree.to_path_buf(),
        },
    })?;

    if origin != repository.as_bytes() {
        // Git may store a relative local repository as an absolute origin. This
        // comparison is intentionally literal; dot does not normalize Git sources.
        return Err(GitError::OriginMismatch {
            worktree: worktree.to_path_buf(),
            actual: render_bytes(origin),
            expected: render_bytes(repository.as_bytes()),
        });
    }
    Ok(())
}

fn checked_output(
    worktree: &Path,
    operation: &'static str,
    args: &[&str],
) -> Result<Output, GitError> {
    let output = git_output(worktree, operation, args)?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(command_failed(worktree, operation, output))
    }
}

fn git_output(worktree: &Path, operation: &'static str, args: &[&str]) -> Result<Output, GitError> {
    git_command()
        .arg("-C")
        .arg(worktree)
        .args(args)
        .output()
        .map_err(|source| GitError::CommandLaunch {
            operation,
            worktree: worktree.to_path_buf(),
            source,
        })
}

fn command_failed(worktree: &Path, operation: &'static str, output: Output) -> GitError {
    GitError::CommandFailed {
        operation,
        worktree: worktree.to_path_buf(),
        status: output.status,
        stderr: render_bytes(&output.stderr),
    }
}

fn git_command() -> Command {
    let mut command = Command::new("git");
    for variable in LOCATION_ENVIRONMENT {
        command.env_remove(variable);
    }
    command
}

fn strip_line_ending(bytes: &[u8]) -> &[u8] {
    let bytes = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    bytes.strip_suffix(b"\r").unwrap_or(bytes)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OriginOutputError {
    Missing,
    Repeated,
    Malformed,
}

fn parse_origin(stdout: &[u8]) -> Result<&[u8], OriginOutputError> {
    if stdout.is_empty() {
        return Err(OriginOutputError::Missing);
    }
    let Some(origin) = stdout.strip_suffix(b"\0") else {
        return Err(OriginOutputError::Malformed);
    };
    if origin.contains(&b'\0') {
        return Err(OriginOutputError::Repeated);
    }
    Ok(origin)
}

fn render_bytes(bytes: &[u8]) -> String {
    let mut rendered = String::new();
    for byte in bytes {
        match byte {
            b'`' => rendered.push_str(r"\`"),
            b'\\' => rendered.push_str(r"\\"),
            b'\n' => rendered.push_str(r"\n"),
            b'\r' => rendered.push_str(r"\r"),
            b'\t' => rendered.push_str(r"\t"),
            b' '..=b'~' => rendered.push(char::from(*byte)),
            byte => write!(rendered, "\\x{byte:02x}").expect("writing to a String cannot fail"),
        }
    }
    rendered
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum GitError {
    #[error("failed to inspect Git worktree entry `{}`: {source}", .worktree.display())]
    Inspect {
        worktree: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("Git worktree entry `{}` is not a directory", .worktree.display())]
    NotDirectory { worktree: PathBuf },
    #[error("failed to launch Git clone for worktree `{}`: {source}", .worktree.display())]
    CloneLaunch {
        worktree: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("Git clone for worktree `{}` exited with {status}", .worktree.display())]
    CloneFailed {
        worktree: PathBuf,
        status: ExitStatus,
    },
    #[error("failed to launch Git to {operation} in `{}`: {source}", .worktree.display())]
    CommandLaunch {
        operation: &'static str,
        worktree: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "Git failed to {operation} in `{}` with {status}; stderr: `{stderr}`",
        .worktree.display()
    )]
    CommandFailed {
        operation: &'static str,
        worktree: PathBuf,
        status: ExitStatus,
        stderr: String,
    },
    #[error(
        "Git entry `{}` is not a worktree (`rev-parse --is-inside-work-tree` returned `{actual}`)",
        .worktree.display()
    )]
    NotWorktree { worktree: PathBuf, actual: String },
    #[error(
        "Git entry `{}` is not the worktree root (`rev-parse --show-prefix` returned `{prefix}`)",
        .worktree.display()
    )]
    NotWorktreeRoot { worktree: PathBuf, prefix: String },
    #[error("Git worktree `{}` has no remote.origin.url", .worktree.display())]
    MissingOrigin { worktree: PathBuf },
    #[error("Git worktree `{}` has more than one remote.origin.url", .worktree.display())]
    RepeatedOrigin { worktree: PathBuf },
    #[error(
        "Git worktree `{}` returned a malformed remote.origin.url record",
        .worktree.display()
    )]
    MalformedOrigin { worktree: PathBuf },
    #[error(
        "Git worktree `{}` origin does not match --git\nactual: `{actual}`\nexpected: `{expected}`",
        .worktree.display()
    )]
    OriginMismatch {
        worktree: PathBuf,
        actual: String,
        expected: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_exactly_one_nul_terminated_origin_record() {
        enum Expected<'a> {
            Origin(&'a [u8]),
            Missing,
            Repeated,
            Malformed,
        }

        let cases = [
            (&b""[..], Expected::Missing),
            (&b"first\0second\0"[..], Expected::Repeated),
            (&b"missing terminator"[..], Expected::Malformed),
            (
                &b"https://example.com/dot.git\0"[..],
                Expected::Origin(b"https://example.com/dot.git"),
            ),
            (&b"\xff\0"[..], Expected::Origin(b"\xff")),
        ];

        for (stdout, expected) in cases {
            let actual = parse_origin(stdout);
            match expected {
                Expected::Origin(origin) => assert_eq!(
                    actual.expect("one terminated record should parse"),
                    origin,
                    "stdout: {stdout:?}"
                ),
                Expected::Missing => assert!(
                    matches!(actual, Err(OriginOutputError::Missing)),
                    "stdout: {stdout:?}"
                ),
                Expected::Repeated => assert!(
                    matches!(actual, Err(OriginOutputError::Repeated)),
                    "stdout: {stdout:?}"
                ),
                Expected::Malformed => assert!(
                    matches!(actual, Err(OriginOutputError::Malformed)),
                    "stdout: {stdout:?}"
                ),
            }
        }
    }

    #[test]
    fn renders_origin_bytes_losslessly_for_single_line_diagnostics() {
        let cases = [
            (&b"`"[..], r"\`"),
            (&b"\\"[..], r"\\"),
            (&b"line\n\x01"[..], r"line\n\x01"),
            ("é".as_bytes(), r"\xc3\xa9"),
        ];

        for (input, expected) in cases {
            assert_eq!(render_bytes(input), expected, "input: {input:?}");
        }
    }
}

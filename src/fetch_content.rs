use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use tempfile::NamedTempFile;
use url::Url;

use crate::plan::PlannedFetchContentAction;
use crate::schema::FetchContentConflict;

const MAX_REDIRECTS: u32 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FetchContentOutcome {
    Created,
    Replaced,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FetchContentStage {
    Preflight,
    Prepare,
    Transfer,
    Commit,
}

impl fmt::Display for FetchContentStage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Preflight => "preflight",
            Self::Prepare => "prepare",
            Self::Transfer => "transfer",
            Self::Commit => "commit",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EntryKind {
    Missing,
    RegularFile,
    Symlink,
    Directory,
    Special,
}

impl fmt::Display for EntryKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Missing => "missing entry",
            Self::RegularFile => "regular file",
            Self::Symlink => "symbolic link",
            Self::Directory => "directory",
            Self::Special => "special filesystem entry",
        })
    }
}

/// An error produced while materializing fetched content at a local target.
///
/// Target-kind safety assumes the target path is not concurrently mutated by another writer.
/// The commit-time inspection narrows accidental races, but it does not atomically bind a later
/// deletion to the inspected filesystem object. With concurrent mutation, an action may fail or
/// affect the competing directory entry; no cross-platform identity or rollback guarantee is
/// provided.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct FetchContentError {
    details: Box<FetchContentErrorDetails>,
}

#[derive(Debug, thiserror::Error)]
enum FetchContentErrorDetails {
    #[error(
        "fetch preflight failed for target `{}`: existing {entry} conflicts with error policy",
        target.display()
    )]
    Conflict { target: PathBuf, entry: EntryKind },
    #[error(
        "fetch preflight failed for target `{}`: {entry} cannot be materialized",
        target.display()
    )]
    Unsupported { target: PathBuf, entry: EntryKind },
    #[error(
        "fetch commit failed for target `{}`: observed {entry} at commit check; refusing that commit path",
        target.display()
    )]
    Concurrent { target: PathBuf, entry: EntryKind },
    #[error(
        "fetch prepare failed for target `{}`: target has no parent directory",
        target.display()
    )]
    InvalidTarget { target: PathBuf },
    #[error(
        "fetch {stage} failed for target `{}`: failed to {operation}: {source}",
        target.display()
    )]
    Io {
        stage: FetchContentStage,
        target: PathBuf,
        operation: &'static str,
        #[source]
        source: io::Error,
    },
    #[error(
        "fetch transfer failed for target `{}`: failed to fetch `{source_url}`: {source}",
        target.display()
    )]
    Transfer {
        source_url: Url,
        target: PathBuf,
        #[source]
        source: FetchTransportError,
    },
}

impl FetchContentError {
    #[cfg(test)]
    fn stage(&self) -> FetchContentStage {
        match &*self.details {
            FetchContentErrorDetails::Conflict { .. }
            | FetchContentErrorDetails::Unsupported { .. } => FetchContentStage::Preflight,
            FetchContentErrorDetails::Concurrent { .. } => FetchContentStage::Commit,
            FetchContentErrorDetails::InvalidTarget { .. } => FetchContentStage::Prepare,
            FetchContentErrorDetails::Io { stage, .. } => *stage,
            FetchContentErrorDetails::Transfer { .. } => FetchContentStage::Transfer,
        }
    }

    fn from_details(details: FetchContentErrorDetails) -> Self {
        Self {
            details: Box::new(details),
        }
    }

    fn conflict(action: &PlannedFetchContentAction, entry: EntryKind) -> Self {
        Self::from_details(FetchContentErrorDetails::Conflict {
            target: action.target().to_owned(),
            entry,
        })
    }

    fn unsupported(action: &PlannedFetchContentAction, entry: EntryKind) -> Self {
        Self::from_details(FetchContentErrorDetails::Unsupported {
            target: action.target().to_owned(),
            entry,
        })
    }

    fn concurrent(action: &PlannedFetchContentAction, entry: EntryKind) -> Self {
        Self::from_details(FetchContentErrorDetails::Concurrent {
            target: action.target().to_owned(),
            entry,
        })
    }

    fn invalid_target(action: &PlannedFetchContentAction) -> Self {
        Self::from_details(FetchContentErrorDetails::InvalidTarget {
            target: action.target().to_owned(),
        })
    }

    fn transfer(action: &PlannedFetchContentAction, source: FetchTransportError) -> Self {
        Self::from_details(FetchContentErrorDetails::Transfer {
            source_url: action.source().clone(),
            target: action.target().to_owned(),
            source,
        })
    }

    fn io(
        action: &PlannedFetchContentAction,
        stage: FetchContentStage,
        operation: &'static str,
        source: io::Error,
    ) -> Self {
        Self::from_details(FetchContentErrorDetails::Io {
            stage,
            target: action.target().to_owned(),
            operation,
            source,
        })
    }

    #[cfg(test)]
    pub(crate) fn for_test(
        action: &PlannedFetchContentAction,
        failure: TestFetchContentFailure,
    ) -> Self {
        match failure {
            TestFetchContentFailure::Conflict => Self::conflict(action, EntryKind::RegularFile),
            TestFetchContentFailure::Directory => Self::unsupported(action, EntryKind::Directory),
            TestFetchContentFailure::Special => Self::unsupported(action, EntryKind::Special),
            TestFetchContentFailure::Prepare(source) => Self::io(
                action,
                FetchContentStage::Prepare,
                "prepare test target",
                source,
            ),
            TestFetchContentFailure::Transport(source) => {
                Self::transfer(action, FetchTransportError::TransportIo(source))
            }
            TestFetchContentFailure::HttpStatus(status) => {
                Self::transfer(action, FetchTransportError::http_status(status))
            }
            TestFetchContentFailure::RedirectLimit => {
                Self::transfer(action, FetchTransportError::TooManyRedirects)
            }
            TestFetchContentFailure::Commit(source) => Self::io(
                action,
                FetchContentStage::Commit,
                "commit test target",
                source,
            ),
        }
    }
}

#[cfg(test)]
pub(crate) enum TestFetchContentFailure {
    Conflict,
    Directory,
    Special,
    Prepare(io::Error),
    Transport(io::Error),
    HttpStatus(u16),
    RedirectLimit,
    Commit(io::Error),
}

pub(crate) trait FetchTransport {
    fn fetch(&self, source: &Url, output: &mut dyn Write) -> Result<(), FetchTransportError>;
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum FetchTransportError {
    #[error("HTTPS-only policy rejected the request or an insecure redirect downgrade")]
    RequireHttpsOnly,
    #[error("redirect limit of {} was exhausted", MAX_REDIRECTS)]
    TooManyRedirects,
    #[error("HTTP response status {0} is not successful")]
    HttpStatus(u16),
    #[error("HTTPS transport failed: {0}")]
    TransportIo(#[source] io::Error),
    #[error("HTTPS transport failed: {0}")]
    TransportUreq(#[source] ureq::Error),
    #[error("failed to copy the HTTPS response body: {0}")]
    BodyCopy(#[source] io::Error),
}

impl FetchTransportError {
    fn io(source: io::Error) -> Self {
        Self::BodyCopy(source)
    }

    fn from_ureq(source: ureq::Error) -> Self {
        match source {
            ureq::Error::RequireHttpsOnly(_) => Self::RequireHttpsOnly,
            ureq::Error::TooManyRedirects => Self::TooManyRedirects,
            ureq::Error::StatusCode(status) => Self::HttpStatus(status),
            ureq::Error::Io(source) => Self::TransportIo(source),
            source => Self::TransportUreq(source),
        }
    }

    const fn http_status(status: u16) -> Self {
        Self::HttpStatus(status)
    }
}

/// Materializes fetched content through one injected transport dispatch.
///
/// Callers must cooperatively exclude concurrent writers to the target path. The final target-kind
/// inspection narrows accidental races but is not an atomic compare-and-swap with removal or
/// persistence. Concurrent mutation may make the action fail or affect the competing directory
/// entry, and there is no cross-platform rollback or filesystem-object identity guarantee.
pub(crate) struct FetchContentRunner<'a> {
    transport: &'a dyn FetchTransport,
}

impl<'a> FetchContentRunner<'a> {
    pub(crate) const fn new(transport: &'a dyn FetchTransport) -> Self {
        Self { transport }
    }

    /// Executes one staged materialization under the runner's cooperative-concurrency contract.
    pub(crate) fn run(
        &self,
        action: &PlannedFetchContentAction,
    ) -> Result<FetchContentOutcome, FetchContentError> {
        let initial = classify(action, FetchContentStage::Preflight)?;
        let outcome = match initial {
            EntryKind::Missing => FetchContentOutcome::Created,
            EntryKind::RegularFile | EntryKind::Symlink => {
                if action.on_conflict() == FetchContentConflict::Error {
                    return Err(FetchContentError::conflict(action, initial));
                }
                FetchContentOutcome::Replaced
            }
            EntryKind::Directory | EntryKind::Special => {
                return Err(FetchContentError::unsupported(action, initial));
            }
        };

        let parent = action
            .target()
            .parent()
            .ok_or_else(|| FetchContentError::invalid_target(action))?;
        fs::create_dir_all(parent).map_err(|source| {
            FetchContentError::io(
                action,
                FetchContentStage::Prepare,
                "create target parent directories",
                source,
            )
        })?;
        let mut staged = NamedTempFile::new_in(parent).map_err(|source| {
            FetchContentError::io(
                action,
                FetchContentStage::Prepare,
                "create staging file",
                source,
            )
        })?;

        self.transport
            .fetch(action.source(), staged.as_file_mut())
            .map_err(|source| FetchContentError::transfer(action, source))?;
        staged.as_file_mut().flush().map_err(|source| {
            FetchContentError::io(
                action,
                FetchContentStage::Transfer,
                "flush staged content",
                source,
            )
        })?;

        let current = classify(action, FetchContentStage::Commit)?;
        // This path-based inspection and the following removal are not atomic. Another writer can
        // replace the entry between them; this guard narrows accidental races but is not a CAS.
        // Correct target-kind behavior therefore assumes cooperative exclusion of other writers.
        match initial {
            EntryKind::Missing if current != EntryKind::Missing => {
                return Err(FetchContentError::concurrent(action, current));
            }
            EntryKind::Missing => {}
            EntryKind::RegularFile | EntryKind::Symlink => match current {
                EntryKind::Missing => {}
                EntryKind::RegularFile => fs::remove_file(action.target()).map_err(|source| {
                    FetchContentError::io(
                        action,
                        FetchContentStage::Commit,
                        "remove current regular-file target",
                        source,
                    )
                })?,
                EntryKind::Symlink => remove_native_symlink(action.target()).map_err(|source| {
                    FetchContentError::io(
                        action,
                        FetchContentStage::Commit,
                        "remove current symbolic-link target",
                        source,
                    )
                })?,
                EntryKind::Directory | EntryKind::Special => {
                    return Err(FetchContentError::concurrent(action, current));
                }
            },
            EntryKind::Directory | EntryKind::Special => unreachable!("preflight rejects these"),
        }

        staged.persist_noclobber(action.target()).map_err(|error| {
            FetchContentError::io(
                action,
                FetchContentStage::Commit,
                "persist staged content without overwriting the target",
                error.error,
            )
        })?;
        Ok(outcome)
    }
}

fn classify(
    action: &PlannedFetchContentAction,
    stage: FetchContentStage,
) -> Result<EntryKind, FetchContentError> {
    match fs::symlink_metadata(action.target()) {
        Err(source)
            if matches!(
                source.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            Ok(EntryKind::Missing)
        }
        Err(source) => Err(FetchContentError::io(
            action,
            stage,
            "inspect fetch target",
            source,
        )),
        Ok(metadata) => {
            let file_type = metadata.file_type();
            if file_type.is_symlink() {
                Ok(EntryKind::Symlink)
            } else if file_type.is_file() {
                Ok(EntryKind::RegularFile)
            } else if file_type.is_dir() {
                Ok(EntryKind::Directory)
            } else {
                Ok(EntryKind::Special)
            }
        }
    }
}

#[cfg(unix)]
fn remove_native_symlink(target: &Path) -> io::Result<()> {
    fs::remove_file(target)
}

#[cfg(windows)]
fn remove_native_symlink(target: &Path) -> io::Result<()> {
    use std::os::windows::fs::FileTypeExt;

    let file_type = fs::symlink_metadata(target)?.file_type();
    if file_type.is_symlink_dir() {
        fs::remove_dir(target)
    } else {
        fs::remove_file(target)
    }
}

pub(crate) struct UreqHttpsTransport {
    agent: ureq::Agent,
}

impl UreqHttpsTransport {
    pub(crate) fn new() -> Self {
        let agent = ureq::Agent::config_builder()
            .https_only(true)
            .max_redirects(MAX_REDIRECTS)
            .max_redirects_will_error(true)
            .http_status_as_error(true)
            .build()
            .into();
        Self { agent }
    }
}

impl Default for UreqHttpsTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl FetchTransport for UreqHttpsTransport {
    fn fetch(&self, source: &Url, output: &mut dyn Write) -> Result<(), FetchTransportError> {
        let mut response = self
            .agent
            .get(source.as_str())
            .call()
            .map_err(FetchTransportError::from_ureq)?;
        validate_final_status(response.status().as_u16())?;
        io::copy(&mut response.body_mut().as_reader(), output).map_err(FetchTransportError::io)?;
        Ok(())
    }
}

fn validate_final_status(status: u16) -> Result<(), FetchTransportError> {
    if (200..=299).contains(&status) {
        Ok(())
    } else {
        Err(FetchTransportError::http_status(status))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::error::Error as _;
    use std::fs;
    use std::io::{self, Write};
    use std::path::{Path, PathBuf};

    use tempfile::tempdir;
    use url::Url;

    use super::*;
    use crate::plan::PlannedFetchContentAction;
    use crate::schema::FetchContentConflict;

    enum FakeResult {
        Bytes(Vec<u8>),
        Error(io::ErrorKind),
    }

    struct FakeTransport {
        calls: Cell<usize>,
        result: FakeResult,
        mutation: RefCell<Option<Box<dyn FnMut()>>>,
    }

    impl FakeTransport {
        fn bytes(bytes: impl Into<Vec<u8>>) -> Self {
            Self {
                calls: Cell::new(0),
                result: FakeResult::Bytes(bytes.into()),
                mutation: RefCell::new(None),
            }
        }

        fn error(kind: io::ErrorKind) -> Self {
            Self {
                calls: Cell::new(0),
                result: FakeResult::Error(kind),
                mutation: RefCell::new(None),
            }
        }

        fn with_mutation(mut self, mutation: impl FnMut() + 'static) -> Self {
            self.mutation = RefCell::new(Some(Box::new(mutation)));
            self
        }
    }

    impl FetchTransport for FakeTransport {
        fn fetch(&self, _source: &Url, output: &mut dyn Write) -> Result<(), FetchTransportError> {
            self.calls.set(self.calls.get() + 1);
            match &self.result {
                FakeResult::Bytes(bytes) => {
                    output.write_all(bytes).map_err(FetchTransportError::io)?;
                }
                FakeResult::Error(kind) => {
                    return Err(FetchTransportError::io(io::Error::new(
                        *kind,
                        "configured fake transport failure",
                    )));
                }
            }
            if let Some(mut mutation) = self.mutation.borrow_mut().take() {
                mutation();
            }
            Ok(())
        }
    }

    fn action(target: &Path, on_conflict: FetchContentConflict) -> PlannedFetchContentAction {
        PlannedFetchContentAction::new(
            Url::parse("https://example.com/content").expect("URL should be valid"),
            target.to_owned(),
            on_conflict,
        )
    }

    fn run(
        target: &Path,
        on_conflict: FetchContentConflict,
        transport: &FakeTransport,
    ) -> Result<FetchContentOutcome, FetchContentError> {
        FetchContentRunner::new(transport).run(&action(target, on_conflict))
    }

    fn assert_error_shape(
        error: &FetchContentError,
        stage: FetchContentStage,
        target: &Path,
        reason: &str,
    ) {
        let message = error.to_string();
        assert!(
            message.starts_with(&format!(
                "fetch {stage} failed for target `{}`:",
                target.display()
            )),
            "unexpected error shape: {message}"
        );
        assert!(
            message.contains(reason),
            "error `{message}` did not contain reason `{reason}`"
        );
    }

    fn directory_entries(path: &Path) -> Vec<PathBuf> {
        let mut entries = fs::read_dir(path)
            .expect("directory should be readable")
            .map(|entry| entry.expect("entry should be readable").path())
            .collect::<Vec<_>>();
        entries.sort();
        entries
    }

    #[cfg(unix)]
    fn symlink_file(source: &Path, target: &Path) {
        std::os::unix::fs::symlink(source, target).expect("file symlink should be created");
    }

    #[cfg(windows)]
    fn symlink_file(source: &Path, target: &Path) {
        std::os::windows::fs::symlink_file(source, target).expect("file symlink should be created");
    }

    #[cfg(unix)]
    fn symlink_dir(source: &Path, target: &Path) {
        std::os::unix::fs::symlink(source, target).expect("directory symlink should be created");
    }

    #[cfg(windows)]
    fn symlink_dir(source: &Path, target: &Path) {
        std::os::windows::fs::symlink_dir(source, target)
            .expect("directory symlink should be created");
    }

    #[cfg(unix)]
    struct PermissionGuard {
        path: PathBuf,
        original: fs::Permissions,
    }

    #[cfg(unix)]
    impl PermissionGuard {
        fn set(path: &Path, mode: u32) -> Self {
            use std::os::unix::fs::PermissionsExt;

            let original = fs::metadata(path)
                .expect("guarded directory should be inspectable")
                .permissions();
            fs::set_permissions(path, fs::Permissions::from_mode(mode))
                .expect("guarded directory permissions should be changed");
            Self {
                path: path.to_owned(),
                original,
            }
        }
    }

    #[cfg(unix)]
    impl Drop for PermissionGuard {
        fn drop(&mut self) {
            let _ = fs::set_permissions(&self.path, self.original.clone());
        }
    }

    #[cfg(unix)]
    fn effective_uid() -> u32 {
        unsafe extern "C" {
            fn geteuid() -> u32;
        }

        // SAFETY: geteuid takes no arguments and has no memory-safety preconditions.
        unsafe { geteuid() }
    }

    #[cfg(unix)]
    fn drop_staging_helper_privileges() {
        unsafe extern "C" {
            fn setuid(uid: u32) -> i32;
        }

        assert_eq!(effective_uid(), 0, "staging helper should start as root");
        // SAFETY: UID 65534 is passed by value and setuid has no pointer arguments.
        let result = unsafe { setuid(65_534) };
        assert_eq!(result, 0, "staging helper should drop root privileges");
        assert_ne!(effective_uid(), 0, "staging helper must not remain root");
    }

    #[test]
    fn missing_target_creates_missing_parents_and_returns_created() {
        let directory = tempdir().expect("temporary directory should be created");
        let target = directory.path().join("missing/parents/content");
        let transport = FakeTransport::bytes(b"downloaded".to_vec());

        let outcome = run(&target, FetchContentConflict::Error, &transport)
            .expect("missing target should be created");

        assert_eq!(outcome, FetchContentOutcome::Created);
        assert_eq!(
            fs::read(&target).expect("target should be readable"),
            b"downloaded"
        );
        assert_eq!(transport.calls.get(), 1);
    }

    #[test]
    fn eligible_action_always_fetches_once_even_when_bytes_are_equal() {
        let directory = tempdir().expect("temporary directory should be created");
        let target = directory.path().join("content");
        fs::write(&target, b"same").expect("existing target should be written");
        let transport = FakeTransport::bytes(b"same".to_vec());

        let outcome = run(&target, FetchContentConflict::Replace, &transport)
            .expect("existing target should be refreshed");

        assert_eq!(outcome, FetchContentOutcome::Replaced);
        assert_eq!(transport.calls.get(), 1);
        assert_eq!(
            fs::read(target).expect("target should be readable"),
            b"same"
        );
    }

    #[test]
    fn error_policy_rejects_existing_regular_file_before_transport() {
        let directory = tempdir().expect("temporary directory should be created");
        let target = directory.path().join("content");
        fs::write(&target, b"old").expect("existing target should be written");
        let transport = FakeTransport::bytes(b"new".to_vec());

        let error = run(&target, FetchContentConflict::Error, &transport)
            .expect_err("existing regular file should conflict");

        assert_eq!(error.stage(), FetchContentStage::Preflight);
        assert!(matches!(
            &*error.details,
            FetchContentErrorDetails::Conflict {
                entry: EntryKind::RegularFile,
                ..
            }
        ));
        assert_error_shape(
            &error,
            FetchContentStage::Preflight,
            &target,
            "existing regular file conflicts with error policy",
        );
        assert!(error.source().is_none());
        assert_eq!(transport.calls.get(), 0);
        assert_eq!(fs::read(target).expect("target should be readable"), b"old");
    }

    #[test]
    fn error_policy_rejects_existing_symlink_before_transport() {
        let directory = tempdir().expect("temporary directory should be created");
        let destination = directory.path().join("destination");
        let target = directory.path().join("content");
        fs::write(&destination, b"destination").expect("destination should be written");
        symlink_file(&destination, &target);
        let transport = FakeTransport::bytes(b"new".to_vec());

        let error = run(&target, FetchContentConflict::Error, &transport)
            .expect_err("existing symlink should conflict");

        assert_eq!(error.stage(), FetchContentStage::Preflight);
        assert!(matches!(
            &*error.details,
            FetchContentErrorDetails::Conflict {
                entry: EntryKind::Symlink,
                ..
            }
        ));
        assert_eq!(transport.calls.get(), 0);
        assert!(
            fs::symlink_metadata(target)
                .expect("target should exist")
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn replace_replaces_regular_file() {
        let directory = tempdir().expect("temporary directory should be created");
        let target = directory.path().join("content");
        fs::write(&target, b"old").expect("existing target should be written");
        let transport = FakeTransport::bytes(b"new".to_vec());

        assert_eq!(
            run(&target, FetchContentConflict::Replace, &transport)
                .expect("regular file should be replaced"),
            FetchContentOutcome::Replaced
        );
        assert_eq!(fs::read(target).expect("target should be readable"), b"new");
    }

    #[test]
    fn replace_replaces_symlink_without_modifying_destination() {
        let directory = tempdir().expect("temporary directory should be created");
        let destination = directory.path().join("destination");
        let target = directory.path().join("content");
        fs::write(&destination, b"destination").expect("destination should be written");
        symlink_file(&destination, &target);
        let transport = FakeTransport::bytes(b"new".to_vec());

        run(&target, FetchContentConflict::Replace, &transport)
            .expect("symlink should be replaced");

        assert!(
            !fs::symlink_metadata(&target)
                .expect("target should exist")
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(target).expect("target should be readable"), b"new");
        assert_eq!(
            fs::read(destination).expect("destination should be readable"),
            b"destination"
        );
    }

    #[test]
    fn replace_replaces_symlink_pointing_to_directory() {
        let directory = tempdir().expect("temporary directory should be created");
        let destination = directory.path().join("destination");
        let target = directory.path().join("content");
        fs::create_dir(&destination).expect("destination directory should be created");
        fs::write(destination.join("sentinel"), b"safe").expect("sentinel should be written");
        symlink_dir(&destination, &target);
        let transport = FakeTransport::bytes(b"new".to_vec());

        run(&target, FetchContentConflict::Replace, &transport)
            .expect("directory symlink should be replaced");

        assert_eq!(fs::read(target).expect("target should be readable"), b"new");
        assert_eq!(
            fs::read(destination.join("sentinel")).expect("sentinel should be readable"),
            b"safe"
        );
    }

    #[test]
    fn replacing_hard_link_changes_only_target_entry() {
        let directory = tempdir().expect("temporary directory should be created");
        let sibling = directory.path().join("sibling");
        let target = directory.path().join("content");
        fs::write(&sibling, b"old").expect("sibling should be written");
        fs::hard_link(&sibling, &target).expect("hard link should be created");
        let transport = FakeTransport::bytes(b"new".to_vec());

        run(&target, FetchContentConflict::Replace, &transport)
            .expect("hard-linked target should be replaced");

        assert_eq!(fs::read(target).expect("target should be readable"), b"new");
        assert_eq!(
            fs::read(sibling).expect("sibling should be readable"),
            b"old"
        );
    }

    #[test]
    fn both_policies_reject_real_directory_before_transport() {
        for policy in [FetchContentConflict::Error, FetchContentConflict::Replace] {
            let directory = tempdir().expect("temporary directory should be created");
            let target = directory.path().join("content");
            fs::create_dir(&target).expect("target directory should be created");
            let transport = FakeTransport::bytes(b"new".to_vec());

            let error =
                run(&target, policy, &transport).expect_err("real directory should be rejected");

            assert_eq!(error.stage(), FetchContentStage::Preflight);
            assert!(matches!(
                &*error.details,
                FetchContentErrorDetails::Unsupported {
                    entry: EntryKind::Directory,
                    ..
                }
            ));
            assert_error_shape(
                &error,
                FetchContentStage::Preflight,
                &target,
                "directory cannot be materialized",
            );
            assert!(error.source().is_none());
            assert_eq!(transport.calls.get(), 0);
            assert!(target.is_dir());
        }
    }

    #[cfg(unix)]
    #[test]
    fn both_policies_reject_special_entry_before_transport() {
        use std::os::unix::net::UnixListener;

        for policy in [FetchContentConflict::Error, FetchContentConflict::Replace] {
            let directory = tempdir().expect("temporary directory should be created");
            let target = directory.path().join("content.sock");
            let listener = UnixListener::bind(&target).expect("Unix socket should be created");
            let transport = FakeTransport::bytes(b"new".to_vec());

            let error =
                run(&target, policy, &transport).expect_err("special entry should be rejected");

            assert_eq!(error.stage(), FetchContentStage::Preflight);
            assert!(matches!(
                &*error.details,
                FetchContentErrorDetails::Unsupported {
                    entry: EntryKind::Special,
                    ..
                }
            ));
            assert_error_shape(
                &error,
                FetchContentStage::Preflight,
                &target,
                "special filesystem entry cannot be materialized",
            );
            assert!(error.source().is_none());
            assert_eq!(transport.calls.get(), 0);
            assert!(fs::symlink_metadata(&target).is_ok());
            drop(listener);
        }
    }

    #[test]
    fn transport_failure_leaves_existing_replace_target_unchanged_and_cleans_staging() {
        let directory = tempdir().expect("temporary directory should be created");
        let target = directory.path().join("content");
        fs::write(&target, b"old").expect("existing target should be written");
        let before = directory_entries(directory.path());
        let transport = FakeTransport::error(io::ErrorKind::ConnectionReset);

        let error = run(&target, FetchContentConflict::Replace, &transport)
            .expect_err("transport failure should be returned");

        assert_eq!(error.stage(), FetchContentStage::Transfer);
        assert_error_shape(
            &error,
            FetchContentStage::Transfer,
            &target,
            "failed to copy the HTTPS response body: configured fake transport failure",
        );
        assert!(
            error
                .to_string()
                .contains("failed to fetch `https://example.com/content`")
        );
        assert_eq!(transport.calls.get(), 1);
        assert_eq!(fs::read(target).expect("target should be readable"), b"old");
        assert_eq!(directory_entries(directory.path()), before);
        assert!(error.source().is_some());
    }

    #[test]
    fn unusable_parent_fails_before_transport() {
        let directory = tempdir().expect("temporary directory should be created");
        let parent = directory.path().join("not-a-directory");
        fs::write(&parent, b"file").expect("blocking parent should be written");
        let target = parent.join("content");
        let transport = FakeTransport::bytes(b"new".to_vec());

        let error = run(&target, FetchContentConflict::Error, &transport)
            .expect_err("unusable parent should fail");

        assert_eq!(error.stage(), FetchContentStage::Prepare);
        assert_error_shape(
            &error,
            FetchContentStage::Prepare,
            &target,
            "failed to create target parent directories",
        );
        assert_eq!(transport.calls.get(), 0);
        assert!(
            error
                .source()
                .is_some_and(|source| source.is::<io::Error>())
        );
    }

    #[test]
    fn invalid_target_error_has_prepare_shape_and_no_source() {
        let target = PathBuf::from("invalid-target");
        let action = action(&target, FetchContentConflict::Error);
        let error = FetchContentError::invalid_target(&action);

        assert_error_shape(
            &error,
            FetchContentStage::Prepare,
            &target,
            "target has no parent directory",
        );
        assert!(error.source().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn staging_file_creation_failure_preserves_existing_target_before_transport() {
        use std::process::Command;

        const HELPER_ENV: &str = "DOT_FETCH_STAGING_FAILURE_TARGET";
        const HELPER_SENTINEL_ENV: &str = "DOT_FETCH_STAGING_FAILURE_HELPER";
        const HELPER_SENTINEL: &str = "fetch-content-private-staging-helper-v1";
        const HELPER_TEST: &str = "fetch_content::tests::staging_file_creation_failure_preserves_existing_target_before_transport";

        if std::env::var_os(HELPER_SENTINEL_ENV).as_deref()
            == Some(std::ffi::OsStr::new(HELPER_SENTINEL))
        {
            let target = std::env::var_os(HELPER_ENV)
                .map(PathBuf::from)
                .expect("staging failure helper target should be provided");
            drop_staging_helper_privileges();
            assert_staging_creation_failure(&target);
            return;
        }

        let directory = tempfile::tempdir_in("/tmp")
            .expect("world-traversable temporary root should be created");
        let parent = directory.path().join("target-parent");
        let target = parent.join("content");
        fs::create_dir(&parent).expect("target parent should be created");
        fs::write(&target, b"old").expect("existing target should be written");
        let _outer_permissions = PermissionGuard::set(directory.path(), 0o755);
        let _target_permissions = PermissionGuard::set(&target, 0o644);
        let _parent_permissions = PermissionGuard::set(&parent, 0o555);

        match NamedTempFile::new_in(&parent) {
            Err(_) => assert_staging_creation_failure(&target),
            Ok(probe) => {
                drop(probe);
                assert_eq!(
                    effective_uid(),
                    0,
                    "only root may bypass read/execute-only directory permissions"
                );
                let status = Command::new(
                    std::env::current_exe().expect("current test executable should be available"),
                )
                .arg("--exact")
                .arg(HELPER_TEST)
                .arg("--nocapture")
                .env(HELPER_ENV, &target)
                .env(HELPER_SENTINEL_ENV, HELPER_SENTINEL)
                .current_dir("/")
                .status()
                .expect("unprivileged staging failure helper should start");
                assert!(
                    status.success(),
                    "unprivileged staging failure helper failed"
                );
            }
        }

        assert_eq!(directory_entries(&parent), vec![target]);
    }

    #[cfg(unix)]
    fn assert_staging_creation_failure(target: &Path) {
        let transport = FakeTransport::bytes(b"new".to_vec());

        let error = run(target, FetchContentConflict::Replace, &transport)
            .expect_err("staging file creation should fail");

        assert_eq!(error.stage(), FetchContentStage::Prepare);
        assert!(matches!(
            &*error.details,
            FetchContentErrorDetails::Io {
                operation: "create staging file",
                ..
            }
        ));
        assert_eq!(transport.calls.get(), 0);
        assert_eq!(
            fs::read(target).expect("existing target should remain readable"),
            b"old"
        );
        assert_eq!(
            directory_entries(target.parent().expect("target should have a parent")),
            vec![target.to_owned()]
        );
    }

    #[test]
    fn target_appearing_after_missing_preflight_is_not_overwritten() {
        let directory = tempdir().expect("temporary directory should be created");
        let target = directory.path().join("content");
        let mutation_target = target.clone();
        let transport = FakeTransport::bytes(b"downloaded".to_vec()).with_mutation(move || {
            fs::write(&mutation_target, b"concurrent")
                .expect("concurrent target should be written");
        });

        let error = run(&target, FetchContentConflict::Replace, &transport)
            .expect_err("concurrent target should prevent commit");

        assert_eq!(error.stage(), FetchContentStage::Commit);
        assert_eq!(
            fs::read(target).expect("target should be readable"),
            b"concurrent"
        );
        assert_eq!(directory_entries(directory.path()).len(), 1);
    }

    #[test]
    fn replaceable_directory_symlink_changed_to_real_directory_is_not_removed() {
        let directory = tempdir().expect("temporary directory should be created");
        let destination = directory.path().join("destination");
        let target = directory.path().join("content");
        fs::create_dir(&destination).expect("destination directory should be created");
        symlink_dir(&destination, &target);
        let mutation_target = target.clone();
        let transport = FakeTransport::bytes(b"downloaded".to_vec()).with_mutation(move || {
            remove_symlink_for_test(&mutation_target);
            fs::create_dir(&mutation_target).expect("concurrent directory should be created");
            fs::write(mutation_target.join("sentinel"), b"safe")
                .expect("sentinel should be written");
        });

        let error = run(&target, FetchContentConflict::Replace, &transport)
            .expect_err("concurrent real directory should prevent commit");

        assert_eq!(error.stage(), FetchContentStage::Commit);
        assert!(matches!(
            &*error.details,
            FetchContentErrorDetails::Concurrent {
                entry: EntryKind::Directory,
                ..
            }
        ));
        assert_eq!(
            error.to_string(),
            format!(
                "fetch commit failed for target `{}`: observed directory at commit check; refusing that commit path",
                target.display()
            )
        );
        assert!(error.source().is_none());
        assert_eq!(
            fs::read(target.join("sentinel")).expect("sentinel should be readable"),
            b"safe"
        );
        assert_eq!(directory_entries(directory.path()).len(), 2);
    }

    #[test]
    fn commit_failure_cleans_transient_staging_file() {
        let directory = tempdir().expect("temporary directory should be created");
        let target = directory.path().join("content");
        let mutation_target = target.clone();
        let transport = FakeTransport::bytes(b"downloaded".to_vec()).with_mutation(move || {
            fs::write(&mutation_target, b"concurrent")
                .expect("concurrent target should be written");
        });

        run(&target, FetchContentConflict::Error, &transport)
            .expect_err("concurrent target should prevent commit");

        assert_eq!(directory_entries(directory.path()), vec![target]);
    }

    #[test]
    fn error_sources_are_preserved_only_for_sourceful_errors() {
        let directory = tempdir().expect("temporary directory should be created");
        let target = directory.path().join("content");
        fs::write(&target, b"old").expect("existing target should be written");
        let conflict = run(
            &target,
            FetchContentConflict::Error,
            &FakeTransport::bytes(b"new".to_vec()),
        )
        .expect_err("existing target should conflict");
        assert!(conflict.source().is_none());

        fs::remove_file(&target).expect("target should be removed");
        let transfer = run(
            &target,
            FetchContentConflict::Error,
            &FakeTransport::error(io::ErrorKind::BrokenPipe),
        )
        .expect_err("transfer should fail");
        let transport_source = transfer
            .source()
            .expect("transfer should retain its source");
        assert!(transport_source.is::<FetchTransportError>());
        assert!(
            transport_source
                .source()
                .is_some_and(|source| source.is::<io::Error>())
        );

        let ureq_transfer = FetchContentError::transfer(
            &action(&target, FetchContentConflict::Error),
            FetchTransportError::from_ureq(ureq::Error::ConnectionFailed),
        );
        let transport_source = ureq_transfer
            .source()
            .expect("transfer should retain its transport source");
        assert!(transport_source.is::<FetchTransportError>());
        assert!(
            transport_source
                .source()
                .is_some_and(|source| source.is::<ureq::Error>())
        );
    }

    #[test]
    fn production_agent_uses_strict_https_redirect_and_status_policy() {
        let transport = UreqHttpsTransport::new();
        let config = transport.agent.config();

        assert!(config.https_only());
        assert_eq!(config.max_redirects(), MAX_REDIRECTS);
        assert!(config.max_redirects_will_error());
        assert!(config.http_status_as_error());
    }

    #[test]
    fn maps_https_redirect_status_and_generic_transport_errors() {
        let require_https = FetchTransportError::from_ureq(ureq::Error::RequireHttpsOnly(
            "http://example.com".to_owned(),
        ));
        assert!(matches!(
            &require_https,
            FetchTransportError::RequireHttpsOnly
        ));
        assert!(require_https.source().is_none());

        let redirects = FetchTransportError::from_ureq(ureq::Error::TooManyRedirects);
        assert!(matches!(&redirects, FetchTransportError::TooManyRedirects));
        assert!(redirects.source().is_none());

        for status in [400, 500] {
            let error = FetchTransportError::from_ureq(ureq::Error::StatusCode(status));
            assert!(matches!(
                &error,
                FetchTransportError::HttpStatus(actual) if *actual == status
            ));
            assert!(error.source().is_none());
        }

        let error = FetchTransportError::from_ureq(ureq::Error::Io(io::Error::new(
            io::ErrorKind::ConnectionAborted,
            "network stopped",
        )));
        assert!(matches!(&error, FetchTransportError::TransportIo(_)));
        assert!(
            error
                .source()
                .is_some_and(|source| source.is::<io::Error>())
        );

        let error = FetchTransportError::from_ureq(ureq::Error::ConnectionFailed);
        assert!(matches!(&error, FetchTransportError::TransportUreq(_)));
        assert!(
            error
                .source()
                .is_some_and(|source| source.is::<ureq::Error>())
        );
    }

    #[test]
    fn validates_representative_final_http_statuses() {
        for status in [100, 199, 300, 302, 399, 400, 404, 500, 599] {
            let error = validate_final_status(status).expect_err("non-2xx should fail");
            assert!(matches!(
                error,
                FetchTransportError::HttpStatus(actual) if actual == status
            ));
        }
        for status in [200, 201, 204, 299] {
            validate_final_status(status).expect("2xx should succeed");
        }
    }

    #[test]
    fn https_downgrade_and_redirect_exhaustion_are_distinct_clear_failures() {
        let downgrade = FetchTransportError::from_ureq(ureq::Error::RequireHttpsOnly(
            "http://example.com".to_owned(),
        ));
        let redirects = FetchTransportError::from_ureq(ureq::Error::TooManyRedirects);

        assert!(matches!(&downgrade, FetchTransportError::RequireHttpsOnly));
        assert!(matches!(&redirects, FetchTransportError::TooManyRedirects));
        assert!(downgrade.to_string().contains("HTTPS"));
        assert!(redirects.to_string().contains("redirect"));
    }

    #[cfg(unix)]
    fn remove_symlink_for_test(path: &Path) {
        fs::remove_file(path).expect("symlink should be removed");
    }

    #[cfg(windows)]
    fn remove_symlink_for_test(path: &Path) {
        use std::os::windows::fs::FileTypeExt;

        let file_type = fs::symlink_metadata(path)
            .expect("symlink should be inspectable")
            .file_type();
        if file_type.is_symlink_dir() {
            fs::remove_dir(path).expect("directory symlink should be removed");
        } else {
            fs::remove_file(path).expect("file symlink should be removed");
        }
    }
}

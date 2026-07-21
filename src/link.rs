use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::plan::PlannedLink;
use crate::schema::{LinkConflict, LinkMissingParent};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkOutcome {
    Satisfied,
    Created,
    Replaced,
    SkippedMissingParent,
}

#[derive(Debug)]
pub struct LinkResult {
    id: String,
    outcome: Result<LinkOutcome, LinkError>,
}

impl LinkResult {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn outcome(&self) -> Result<LinkOutcome, &LinkError> {
        self.outcome.as_ref().copied()
    }
}

#[derive(Debug)]
pub struct LinkReport {
    results: Vec<LinkResult>,
}

impl LinkReport {
    pub fn results(&self) -> &[LinkResult] {
        &self.results
    }

    pub fn all_succeeded(&self) -> bool {
        self.results.iter().all(|result| result.outcome.is_ok())
    }
}

pub fn reconcile(links: &[PlannedLink]) -> Result<LinkReport, LinkPhaseError> {
    let targets = links
        .iter()
        .map(|link| {
            normalize_link_target(link.target())
                .map_err(|error| LinkError::io("normalize target", link.target(), error))
        })
        .collect::<Vec<_>>();

    for (index, candidate) in targets.iter().enumerate() {
        let Ok(candidate) = candidate else {
            continue;
        };
        let duplicates = targets
            .iter()
            .enumerate()
            .filter_map(|(other_index, other)| {
                let other = other.as_ref().ok()?;
                paths_equivalent(candidate, other)
                    .then(|| (other_index, links[other_index].id().to_owned()))
            })
            .collect::<Vec<_>>();
        if duplicates.len() > 1 && duplicates[0].0 == index {
            return Err(LinkPhaseError::DuplicateTarget {
                target: candidate.clone(),
                links: duplicates.into_iter().map(|(_, id)| id).collect(),
            });
        }
    }

    let prepared = links
        .iter()
        .zip(targets)
        .map(|(link, target)| target.and_then(|target| prepare_link(link, target)))
        .collect::<Vec<_>>();
    let results = links
        .iter()
        .zip(prepared)
        .map(|(link, prepared)| LinkResult {
            id: link.id().to_owned(),
            outcome: prepared.and_then(reconcile_one),
        })
        .collect();
    Ok(LinkReport { results })
}

fn prepare_link(link: &PlannedLink, target: PathBuf) -> Result<PreparedLink<'_>, LinkError> {
    let metadata = fs::metadata(link.source())
        .map_err(|source| LinkError::io("inspect source", link.source(), source))?;
    let kind = if metadata.is_file() {
        SourceKind::File
    } else if metadata.is_dir() {
        SourceKind::Directory
    } else {
        return Err(LinkError::UnsupportedSourceType {
            source: link.source().to_owned(),
        });
    };
    let source = fs::canonicalize(link.source())
        .map_err(|error| LinkError::io("canonicalize source", link.source(), error))?;

    Ok(PreparedLink {
        definition: link,
        source,
        target,
        kind,
    })
}

fn reconcile_one(link: PreparedLink<'_>) -> Result<LinkOutcome, LinkError> {
    let parent = link
        .target
        .parent()
        .ok_or_else(|| LinkError::InvalidTarget {
            target: link.target.clone(),
        })?;
    match fs::metadata(parent) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            return Err(LinkError::ParentNotDirectory {
                parent: parent.to_owned(),
            });
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if link.definition.on_missing_parent() == LinkMissingParent::Skip {
                return Ok(LinkOutcome::SkippedMissingParent);
            }
            fs::create_dir_all(parent)
                .map_err(|error| LinkError::io("create target parent", parent, error))?;
        }
        Err(error) => return Err(LinkError::io("inspect target parent", parent, error)),
    }

    match fs::symlink_metadata(&link.target) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(LinkError::io("inspect target", &link.target, error)),
        Ok(metadata) if metadata.file_type().is_symlink() => {
            let destination = resolve_link_destination(&link.target)?;
            if paths_equivalent(&destination, &link.source) {
                return Ok(LinkOutcome::Satisfied);
            }
            if link.definition.on_conflict() == LinkConflict::Error {
                return Err(LinkError::Conflict {
                    target: link.target,
                    destination,
                });
            }
            remove_native_symlink(&link.target, &metadata.file_type())
                .map_err(|error| LinkError::io("remove symbolic link", &link.target, error))?;
            create_native_symlink(&link.source, &link.target, link.kind)
                .map_err(|error| LinkError::io("create replacement link", &link.target, error))?;
            verify_link(&link)?;
            return Ok(LinkOutcome::Replaced);
        }
        Ok(_) => {
            return Err(LinkError::ExistingNonLink {
                target: link.target,
            });
        }
    }

    create_native_symlink(&link.source, &link.target, link.kind)
        .map_err(|error| LinkError::io("create symbolic link", &link.target, error))?;
    verify_link(&link)?;
    Ok(LinkOutcome::Created)
}

struct PreparedLink<'a> {
    definition: &'a PlannedLink,
    source: PathBuf,
    target: PathBuf,
    kind: SourceKind,
}

fn normalize_link_target(target: &Path) -> io::Result<PathBuf> {
    let target = lexically_normalize(target);
    let parent = target
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "link target has no parent"))?;
    let name = target.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "link target has no final component",
        )
    })?;
    let mut normalized = normalize_existing_or_future(parent)?;
    normalized.push(name);
    Ok(lexically_normalize(&normalized))
}

fn resolve_link_destination(target: &Path) -> Result<PathBuf, LinkError> {
    let destination = fs::read_link(target)
        .map_err(|error| LinkError::io("read symbolic link", target, error))?;
    let destination = if destination.is_absolute() {
        destination
    } else {
        let parent = target.parent().ok_or_else(|| LinkError::InvalidTarget {
            target: target.to_owned(),
        })?;
        parent.join(destination)
    };
    normalize_existing_or_future(&destination)
        .map_err(|error| LinkError::io("normalize symbolic-link destination", &destination, error))
}

fn verify_link(link: &PreparedLink<'_>) -> Result<(), LinkError> {
    let metadata = fs::symlink_metadata(&link.target)
        .map_err(|error| LinkError::io("verify symbolic link", &link.target, error))?;
    if !metadata.file_type().is_symlink() {
        return Err(LinkError::VerificationMismatch {
            target: link.target.clone(),
            expected: link.source.clone(),
            actual: None,
        });
    }
    let actual = resolve_link_destination(&link.target)?;
    if !paths_equivalent(&actual, &link.source) {
        return Err(LinkError::VerificationMismatch {
            target: link.target.clone(),
            expected: link.source.clone(),
            actual: Some(actual),
        });
    }
    Ok(())
}

fn normalize_existing_or_future(path: &Path) -> io::Result<PathBuf> {
    let mut current = path;
    let mut missing = Vec::<OsString>::new();

    loop {
        match fs::canonicalize(current) {
            Ok(mut existing) => {
                for component in missing.iter().rev() {
                    existing.push(component);
                }
                return Ok(lexically_normalize(&existing));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let Some(component) = current.file_name() else {
                    return Err(error);
                };
                missing.push(component.to_owned());
                let Some(parent) = current.parent() else {
                    return Err(error);
                };
                current = parent;
            }
            Err(error) => return Err(error),
        }
    }
}

fn lexically_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    normalized
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SourceKind {
    File,
    Directory,
}

#[cfg(unix)]
fn create_native_symlink(source: &Path, target: &Path, _: SourceKind) -> io::Result<()> {
    std::os::unix::fs::symlink(source, target)
}

#[cfg(windows)]
fn create_native_symlink(source: &Path, target: &Path, kind: SourceKind) -> io::Result<()> {
    match kind {
        SourceKind::File => std::os::windows::fs::symlink_file(source, target),
        SourceKind::Directory => std::os::windows::fs::symlink_dir(source, target),
    }
}

#[cfg(unix)]
fn remove_native_symlink(target: &Path, _: &fs::FileType) -> io::Result<()> {
    fs::remove_file(target)
}

#[cfg(windows)]
fn remove_native_symlink(target: &Path, file_type: &fs::FileType) -> io::Result<()> {
    use std::os::windows::fs::FileTypeExt;

    if file_type.is_symlink_dir() {
        fs::remove_dir(target)
    } else {
        fs::remove_file(target)
    }
}

#[cfg(not(windows))]
fn paths_equivalent(left: &Path, right: &Path) -> bool {
    left == right
}

#[cfg(windows)]
fn paths_equivalent(left: &Path, right: &Path) -> bool {
    left.to_string_lossy()
        .eq_ignore_ascii_case(&right.to_string_lossy())
}

#[derive(Debug)]
pub enum LinkError {
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    UnsupportedSourceType {
        source: PathBuf,
    },
    ExistingNonLink {
        target: PathBuf,
    },
    Conflict {
        target: PathBuf,
        destination: PathBuf,
    },
    InvalidTarget {
        target: PathBuf,
    },
    ParentNotDirectory {
        parent: PathBuf,
    },
    VerificationMismatch {
        target: PathBuf,
        expected: PathBuf,
        actual: Option<PathBuf>,
    },
}

impl LinkError {
    fn io(operation: &'static str, path: &Path, source: io::Error) -> Self {
        Self::Io {
            operation,
            path: path.to_owned(),
            source,
        }
    }
}

impl fmt::Display for LinkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "failed to {operation} `{}`: {source}",
                path.display()
            ),
            Self::UnsupportedSourceType { source } => write!(
                formatter,
                "link source `{}` is not a file or directory",
                source.display()
            ),
            Self::ExistingNonLink { target } => write!(
                formatter,
                "link target `{}` exists and is not a symbolic link",
                target.display()
            ),
            Self::Conflict {
                target,
                destination,
            } => write!(
                formatter,
                "link target `{}` points to `{}`",
                target.display(),
                destination.display()
            ),
            Self::InvalidTarget { target } => {
                write!(
                    formatter,
                    "link target `{}` has no parent",
                    target.display()
                )
            }
            Self::ParentNotDirectory { parent } => write!(
                formatter,
                "link target parent `{}` is not a directory",
                parent.display()
            ),
            Self::VerificationMismatch {
                target,
                expected,
                actual,
            } => write!(
                formatter,
                "link target `{}` did not resolve to `{}` after creation; actual destination: {}",
                target.display(),
                expected.display(),
                actual
                    .as_deref()
                    .map(Path::display)
                    .map(|path| path.to_string())
                    .unwrap_or_else(|| "<not a symbolic link>".to_owned())
            ),
        }
    }
}

impl Error for LinkError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::UnsupportedSourceType { .. }
            | Self::ExistingNonLink { .. }
            | Self::Conflict { .. }
            | Self::InvalidTarget { .. }
            | Self::ParentNotDirectory { .. }
            | Self::VerificationMismatch { .. } => None,
        }
    }
}

#[derive(Debug)]
pub enum LinkPhaseError {
    DuplicateTarget { target: PathBuf, links: Vec<String> },
}

impl fmt::Display for LinkPhaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateTarget { target, links } => write!(
                formatter,
                "links {links:?} resolve to the same target `{}`",
                target.display()
            ),
        }
    }
}

impl Error for LinkPhaseError {}

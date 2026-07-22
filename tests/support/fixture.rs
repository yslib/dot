use std::fs;
use std::path::{Component, Path, PathBuf};

pub fn path(relative: impl AsRef<Path>) -> PathBuf {
    let relative = relative.as_ref();
    assert!(
        !relative
            .components()
            .any(|component| matches!(component, Component::Prefix(_) | Component::RootDir)),
        "fixture path must be relative: {}",
        relative.display()
    );
    assert!(
        !relative
            .components()
            .any(|component| component == Component::ParentDir),
        "fixture path must not contain `..`: {}",
        relative.display()
    );

    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(relative)
}

pub fn read(relative: impl AsRef<Path>) -> String {
    let path = path(relative);
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read fixture `{}`: {error}", path.display()))
}

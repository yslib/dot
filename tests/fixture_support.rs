mod support;

use std::path::Path;

use support::fixture;

#[test]
fn resolves_and_reads_a_fixture_below_the_fixture_root() {
    let path = fixture::path("support/readable.txt");

    assert!(path.is_absolute());
    assert!(path.ends_with(Path::new("tests/fixtures/support/readable.txt")));
    assert_eq!(fixture::read("support/readable.txt"), "fixture contents\n");
}

#[test]
#[should_panic(expected = "fixture path must be relative")]
fn rejects_an_absolute_fixture_path() {
    fixture::path("/outside.txt");
}

#[test]
#[should_panic(expected = "fixture path must not contain `..`")]
fn rejects_fixture_parent_traversal() {
    fixture::path("schema/../../outside.toml");
}

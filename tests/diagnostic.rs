use std::io;

use dot::diagnostic::{Operation, lookup};

#[test]
fn maps_windows_symlink_privilege_error_to_a_structured_hint() {
    let error = io::Error::from_raw_os_error(1314);

    let hint = lookup("windows", Operation::CreateSymbolicLink, &error)
        .expect("Windows error 1314 should have a symlink-specific hint");

    assert_eq!(hint.code, "windows.symlink.privilege-required");
    assert_eq!(hint.summary, "symbolic-link creation requires permission");
    assert_eq!(
        hint.suggestion,
        "enable Windows Developer Mode or run dot from an elevated shell"
    );
}

#[test]
fn requires_the_platform_operation_and_raw_code_to_match() {
    let privilege_error = io::Error::from_raw_os_error(1314);
    let other_error = io::Error::from_raw_os_error(5);

    assert!(lookup("linux", Operation::CreateSymbolicLink, &privilege_error).is_none());
    assert!(lookup("windows", Operation::StartProcess, &privilege_error).is_none());
    assert!(lookup("windows", Operation::CreateSymbolicLink, &other_error).is_none());
}

#[test]
fn ignores_errors_without_a_raw_os_code() {
    let error = io::Error::other("synthetic failure");

    assert!(
        lookup("windows", Operation::CreateSymbolicLink, &error).is_none(),
        "a synthetic io::Error must not be confused with a native error code"
    );
}

use super::{Mapping, Operation};

pub(super) const MAPPINGS: &[Mapping] = &[Mapping {
    operation: Operation::CreateSymbolicLink,
    raw_code: 1314,
    code: "windows.symlink.privilege-required",
    summary: "symbolic-link creation requires permission",
    suggestion: "enable Windows Developer Mode or run dot from an elevated shell",
}];

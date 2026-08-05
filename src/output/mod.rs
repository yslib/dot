//! Platform-independent output formats.

mod tsv;

pub use tsv::{PreparedTsv, TsvRecord, TsvRenderer};

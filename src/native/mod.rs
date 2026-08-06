//! Native host capabilities and high-level operations.

mod apply;
pub mod command_action;
pub mod diagnostic;
pub mod dry_run;
mod fetch_content;
pub mod job_execution;
pub mod link;
pub mod plan;
pub mod process;
pub mod provider;
pub mod provider_check;
mod runtime;
mod terminal;

pub use apply::{ApplyError, apply};
pub use dry_run::{DryRunError, dry_run};
pub use provider_check::{ProviderCheckError, check_providers};
pub use runtime::NativeRuntime;
pub use terminal::TerminalRenderer;

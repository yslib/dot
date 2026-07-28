use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::PathBuf;

use clap::error::ErrorKind;
use clap::{Args, CommandFactory, Error, Parser, Subcommand};

use crate::app::{Dispatch, ExecutionRequest, Operation, ProfileSelection, ScopeSelection};
use crate::config::ConfigRequest;
use crate::job::{JobSelection, JobSelector};
use crate::schema::SelectorIdentifier;

pub fn parse() -> Dispatch {
    let cli = Cli::parse();
    cli.into_dispatch().unwrap_or_else(|error| error.exit())
}

pub fn try_parse_from<I, T>(args: I) -> Result<Dispatch, Error>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    Cli::try_parse_from(args)?.into_dispatch()
}

#[derive(Debug, Parser)]
#[command(
    name = "dot",
    version,
    about = "Bootstrap a declared development environment",
    arg_required_else_help = true,
    subcommand_required = true
)]
struct Cli {
    /// Path to the TOML manifest; defaults to ./.dot.toml, then the user fallback
    #[arg(short, long, global = true, value_name = "PATH")]
    config: Option<PathBuf>,

    /// Inject PlatformInfo for development-time compatibility selection; host environment, XDG
    /// paths, commands, and filesystem state remain unchanged
    #[cfg(feature = "dev-platform-override")]
    #[arg(
        long,
        global = true,
        value_name = "TOML",
        value_parser = crate::platform::parse_override
    )]
    platform: Option<crate::platform::PlatformInfo>,

    #[command(subcommand)]
    command: Command,
}

impl Cli {
    fn into_dispatch(self) -> Result<Dispatch, Error> {
        #[cfg(feature = "dev-platform-override")]
        let platform_override = self.platform;
        #[cfg(not(feature = "dev-platform-override"))]
        let platform_override = None;

        let operation = match self.command {
            Command::Apply(args) => Operation::Apply(args.into_request(ExecutionCommand::Apply)?),
            Command::DryRun(args) => {
                Operation::DryRun(args.into_request(ExecutionCommand::DryRun)?)
            }
            Command::Check {
                command: CheckCommand::Providers(args),
            } => Operation::CheckProviders(args.into_scope()),
            Command::List {
                command: ListCommand::Targets(args),
            } => Operation::ListTargets { all: args.all },
            Command::List {
                command: ListCommand::Profiles(args),
            } => Operation::ListProfiles {
                target: args.target,
            },
            Command::List {
                command: ListCommand::Jobs(args),
            } => Operation::ListJobs(args.into_scope()),
        };

        Ok(Dispatch {
            config: self
                .config
                .map_or(ConfigRequest::Discover, ConfigRequest::Explicit),
            operation,
            platform_override,
        })
    }
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Apply the selected jobs
    Apply(ExecutionArgs),

    /// Show the selected apply plan without executing it
    DryRun(ExecutionArgs),

    /// Inspect the current environment without repairing it
    Check {
        #[command(subcommand)]
        command: CheckCommand,
    },

    /// Print machine-readable catalog records
    List {
        #[command(subcommand)]
        command: ListCommand,
    },
}

#[derive(Debug, Subcommand)]
enum CheckCommand {
    /// Probe every effective provider
    Providers(ScopeArgs),
}

#[derive(Debug, Subcommand)]
enum ListCommand {
    /// List configured targets
    Targets(ListTargetsArgs),

    /// List declared profiles for one target
    Profiles(ListProfilesArgs),

    /// List effective unresolved jobs
    Jobs(ScopeArgs),
}

#[derive(Debug, Args)]
struct ExecutionArgs {
    #[command(flatten)]
    scope: ScopeArgs,

    /// Select one package, action, or link job; repeat to select more
    #[arg(long, value_name = "KIND:ID")]
    job: Vec<JobSelector>,
}

impl ExecutionArgs {
    fn into_request(self, command: ExecutionCommand) -> Result<ExecutionRequest, Error> {
        let jobs = if self.job.is_empty() {
            JobSelection::All
        } else {
            let mut unique = BTreeSet::new();
            for selector in self.job {
                if !unique.insert(selector.clone()) {
                    return Err(duplicate_job_error(&selector, command));
                }
            }
            JobSelection::Only(unique)
        };

        Ok(ExecutionRequest {
            scope: self.scope.into_scope(),
            jobs,
        })
    }
}

#[derive(Clone, Copy)]
enum ExecutionCommand {
    Apply,
    DryRun,
}

impl ExecutionCommand {
    const fn name(self) -> &'static str {
        match self {
            Self::Apply => "apply",
            Self::DryRun => "dry-run",
        }
    }
}

fn duplicate_job_error(selector: &JobSelector, leaf: ExecutionCommand) -> Error {
    let mut command = Cli::command();
    command.build();
    command
        .find_subcommand_mut(leaf.name())
        .expect("the execution leaf came from the CLI command model")
        .error(
            ErrorKind::ArgumentConflict,
            format!("job selector `{selector}` was supplied more than once"),
        )
}

#[derive(Debug, Args)]
struct ScopeArgs {
    /// Target to select; optional when exactly one target is compatible
    #[arg(short, long, value_name = "TARGET", value_parser = parse_selector)]
    target: Option<SelectorIdentifier>,

    /// Profile node to select, or @root for target-root jobs
    #[arg(short, long, value_name = "PROFILE", value_parser = parse_profile)]
    profile: Option<String>,
}

impl ScopeArgs {
    fn into_scope(self) -> ScopeSelection {
        ScopeSelection {
            target: self.target,
            profile: ProfileSelection::from_cli(self.profile.as_deref())
                .expect("clap validated the profile selector"),
        }
    }
}

#[derive(Debug, Args)]
struct ListTargetsArgs {
    /// Include incompatible targets
    #[arg(long)]
    all: bool,
}

#[derive(Debug, Args)]
struct ListProfilesArgs {
    /// Target to inspect; optional when exactly one target is compatible
    #[arg(short, long, value_name = "TARGET", value_parser = parse_selector)]
    target: Option<SelectorIdentifier>,
}

fn parse_selector(value: &str) -> Result<SelectorIdentifier, String> {
    SelectorIdentifier::new(value).map_err(|error| error.to_string())
}

fn parse_profile(value: &str) -> Result<String, String> {
    ProfileSelection::from_cli(Some(value))
        .map(|_| value.to_owned())
        .map_err(|error| error.to_string())
}

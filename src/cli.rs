use std::ffi::OsString;
use std::path::PathBuf;

use clap::error::ErrorKind;
use clap::{Args, Error, Parser, Subcommand};

use crate::app::{Dispatch, Operation, Selection};

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
    about = "Bootstrap a declared development environment"
)]
struct Cli {
    #[command(flatten)]
    selection: SelectionArgs,

    /// Show the effective apply plan without executing it
    #[arg(long)]
    dry_run: bool,

    /// Inject PlatformInfo for development-time target selection only; commands, environment,
    /// XDG paths, and filesystem state remain on the host
    #[cfg(feature = "dev-platform-override")]
    #[arg(
        long,
        global = true,
        value_name = "TOML",
        value_parser = crate::platform::parse_override
    )]
    platform: Option<crate::platform::PlatformInfo>,

    #[command(subcommand)]
    command: Option<Command>,
}

impl Cli {
    fn into_dispatch(self) -> Result<Dispatch, Error> {
        #[cfg(feature = "dev-platform-override")]
        let platform_override = self.platform;
        #[cfg(not(feature = "dev-platform-override"))]
        let platform_override = None;

        let operation = match self.command {
            None => Operation::Apply {
                dry_run: self.dry_run,
            },
            Some(Command::Check {
                command: CheckCommand::Providers,
            }) if !self.dry_run => Operation::CheckProviders,
            Some(Command::Check { .. }) => {
                return Err(Error::raw(
                    ErrorKind::ArgumentConflict,
                    "--dry-run cannot be used with `check providers`",
                ));
            }
        };

        Ok(Dispatch {
            selection: self.selection.into(),
            operation,
            platform_override,
        })
    }
}

#[derive(Debug, Args)]
struct SelectionArgs {
    /// Path to the TOML manifest
    #[arg(
        short,
        long,
        global = true,
        value_name = "PATH",
        default_value = "dot.toml"
    )]
    config: PathBuf,

    /// Target to select; optional when the manifest contains exactly one target
    #[arg(short, long, global = true, value_name = "TARGET")]
    target: Option<String>,

    /// Unique profile node name to select
    #[arg(
        short,
        long,
        global = true,
        value_name = "PROFILE",
        value_parser = parse_profile_name
    )]
    profile: Option<String>,
}

impl From<SelectionArgs> for Selection {
    fn from(args: SelectionArgs) -> Self {
        Self {
            config: args.config,
            target: args.target,
            profile: args.profile,
        }
    }
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Inspect the current environment without repairing it
    Check {
        #[command(subcommand)]
        command: CheckCommand,
    },
}

#[derive(Debug, Subcommand)]
enum CheckCommand {
    /// Probe every effective provider
    Providers,
}

fn parse_profile_name(value: &str) -> Result<String, String> {
    if value.is_empty() {
        return Err("profile name cannot be empty".into());
    }
    if value.contains('/') {
        return Err("profile must be one node name and cannot contain `/`".into());
    }
    Ok(value.to_owned())
}

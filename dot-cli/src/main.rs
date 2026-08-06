use std::collections::BTreeSet;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::builder::{OsStringValueParser, TypedValueParser};
use clap::error::ErrorKind;
use clap::{Args, CommandFactory, Error, Parser, Subcommand};

use dot_core::inspect::{InspectError, Inspector};
use dot_core::job::{JobSelection, JobSelector};
use dot_core::native::{NativeRuntime, TerminalRenderer, apply, check_providers, dry_run};
use dot_core::output::{TsvRecord, TsvRenderer};
use dot_core::platform::PlatformInfo;
use dot_core::report::{CommandReport, ReportStatus};
use dot_core::schema::SelectorIdentifier;
use dot_core::selection::{ExecutionSelection, ProfileSelection, ScopeSelection};

use config::{ConfigRequest, ConfigSource, load_config};

mod config;
#[cfg(feature = "dev-platform-override")]
mod platform_override;

#[derive(Debug, Parser)]
#[command(
    name = "dot",
    version,
    about = "Bootstrap a declared development environment",
    arg_required_else_help = true,
    subcommand_required = true
)]
struct Cli {
    /// Path or HTTPS URL to the TOML manifest; defaults to ./.dot.toml, then the user fallback
    #[arg(
        short,
        long,
        global = true,
        value_name = "SOURCE",
        value_parser = OsStringValueParser::new().try_map(ConfigSource::from_os_string)
    )]
    config: Option<ConfigSource>,

    /// Git repository containing a root .dot.toml
    #[arg(
        long,
        global = true,
        value_name = "REPOSITORY",
        requires = "git_worktree",
        conflicts_with = "config"
    )]
    git: Option<String>,

    /// Persistent worktree path for --git
    #[arg(long, global = true, value_name = "PATH", requires = "git")]
    git_worktree: Option<PathBuf>,

    /// Inject PlatformInfo for development-time compatibility selection; host environment, XDG
    /// paths, commands, and filesystem state remain unchanged
    #[cfg(feature = "dev-platform-override")]
    #[arg(
        long,
        global = true,
        value_name = "TOML",
        value_parser = platform_override::parse
    )]
    platform: Option<PlatformInfo>,

    #[command(subcommand)]
    command: Command,
}

impl Cli {
    fn run(self) -> ExitCode {
        if let Err(error) = self.command.validate() {
            error.exit();
        }

        #[cfg(feature = "dev-platform-override")]
        let platform_override = self.platform;
        #[cfg(not(feature = "dev-platform-override"))]
        let platform_override: Option<PlatformInfo> = None;

        let request = match (self.config, self.git, self.git_worktree) {
            (Some(source), None, None) => ConfigRequest::Source(source),
            (None, Some(repository), Some(worktree)) => ConfigRequest::Git {
                repository,
                worktree,
            },
            (None, None, None) => ConfigRequest::Discover,
            _ => unreachable!("clap validated the configuration source arguments"),
        };
        let config = match load_config(request) {
            Ok(config) => config,
            Err(error) => return command_error(error),
        };

        if platform_override.is_some() {
            print_platform_warning(&self.command);
        }

        match self.command {
            Command::Apply(args) => {
                let runtime = NativeRuntime::detect();
                match apply(&config, &runtime, &args.into_selection()) {
                    Ok(report) => render_report(&report),
                    Err(error) => command_error(error),
                }
            }
            Command::DryRun(args) => {
                let runtime = NativeRuntime::detect();
                let platform = platform_override.as_ref().unwrap_or(runtime.platform());
                match dry_run(&config, &runtime, platform, &args.into_selection()) {
                    Ok(report) => render_report(&report),
                    Err(error) => command_error(error),
                }
            }
            Command::Check {
                command: CheckCommand::Providers(args),
            } => {
                let runtime = NativeRuntime::detect();
                let platform = platform_override.as_ref().unwrap_or(runtime.platform());
                match check_providers(&config, &runtime, platform, &args.into_scope()) {
                    Ok(report) => render_report(&report),
                    Err(error) => command_error(error),
                }
            }
            Command::List {
                command: ListCommand::Targets(args),
            } => {
                let platform = platform_override.unwrap_or_else(PlatformInfo::detect);
                render_records(Ok(
                    Inspector::new(config.config(), &platform).targets(args.all)
                ))
            }
            Command::List {
                command: ListCommand::Profiles(args),
            } => {
                let platform = platform_override.unwrap_or_else(PlatformInfo::detect);
                render_records(
                    Inspector::new(config.config(), &platform).profiles(args.target.as_ref()),
                )
            }
            Command::List {
                command: ListCommand::Jobs(args),
            } => {
                let platform = platform_override.unwrap_or_else(PlatformInfo::detect);
                render_records(Inspector::new(config.config(), &platform).jobs(&args.into_scope()))
            }
        }
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

impl Command {
    fn validate(&self) -> Result<(), Error> {
        match self {
            Self::Apply(args) => args.validate(ExecutionCommand::Apply),
            Self::DryRun(args) => args.validate(ExecutionCommand::DryRun),
            Self::Check { .. } | Self::List { .. } => Ok(()),
        }
    }
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
    fn validate(&self, command: ExecutionCommand) -> Result<(), Error> {
        let mut unique = BTreeSet::new();
        for selector in &self.job {
            if !unique.insert(selector) {
                return Err(duplicate_job_error(selector, command));
            }
        }
        Ok(())
    }

    fn into_selection(self) -> ExecutionSelection {
        let jobs = if self.job.is_empty() {
            JobSelection::All
        } else {
            JobSelection::Only(self.job.into_iter().collect())
        };
        ExecutionSelection {
            scope: self.scope.into_scope(),
            jobs,
        }
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
            profile: ProfileSelection::parse(self.profile.as_deref())
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
    ProfileSelection::parse(Some(value))
        .map(|_| value.to_owned())
        .map_err(|error| error.to_string())
}

fn print_platform_warning(command: &Command) {
    match command {
        Command::Apply(_) => eprintln!(
            "dot: warning: --platform is ignored by apply; detected host PlatformInfo will be used"
        ),
        Command::DryRun(_) | Command::Check { .. } => eprintln!(
            "dot: warning: --platform affects target compatibility only; host environment, XDG paths, commands, and filesystem state remain unchanged"
        ),
        Command::List {
            command: ListCommand::Targets(_),
        } => eprintln!(
            "dot: warning: --platform affects compatibility labels and default filtering for list targets"
        ),
        Command::List { .. } => eprintln!(
            "dot: warning: --platform affects only omitted-target inference for this list command"
        ),
    }
}

fn command_error(error: impl std::fmt::Display) -> ExitCode {
    eprintln!("dot: {error}");
    ExitCode::FAILURE
}

fn render_report(report: &CommandReport) -> ExitCode {
    let stdout = io::stdout();
    let renderer = TerminalRenderer::new(stdout.is_terminal());
    if let Err(error) = renderer.render(report, &mut stdout.lock()) {
        return command_error(format_args!("failed to write command output: {error}"));
    }
    match report.status {
        ReportStatus::Planned | ReportStatus::Succeeded => ExitCode::SUCCESS,
        ReportStatus::Failed => ExitCode::FAILURE,
    }
}

fn render_records<R: TsvRecord>(records: Result<Vec<R>, InspectError>) -> ExitCode {
    let records = match records {
        Ok(records) => records,
        Err(error) => return command_error(error),
    };
    let output = match TsvRenderer.render_to_string(&records) {
        Ok(output) => output,
        Err(error) => return command_error(format_args!("failed to render list output: {error}")),
    };
    match normalize_list_output(io::stdout().lock().write_all(output.as_bytes())) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => command_error(format_args!("failed to write list output: {error}")),
    }
}

fn normalize_list_output(result: io::Result<()>) -> io::Result<()> {
    match result {
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        result => result,
    }
}

fn main() -> ExitCode {
    Cli::parse().run()
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::ffi::OsString;
    use std::io;
    #[cfg(unix)]
    use std::os::unix::ffi::OsStringExt;
    #[cfg(unix)]
    use std::path::PathBuf;

    #[cfg(unix)]
    use clap::Parser;

    use super::normalize_list_output;
    #[cfg(unix)]
    use super::{Cli, ConfigSource};

    #[cfg(unix)]
    #[test]
    fn config_argument_preserves_non_utf8_native_paths() {
        let source = OsString::from_vec(b"config-\xff.toml".to_vec());
        let parsed = Cli::try_parse_from([
            OsString::from("dot"),
            OsString::from("--config"),
            source.clone(),
            OsString::from("list"),
            OsString::from("targets"),
        ])
        .expect("non-UTF-8 native path should parse");

        assert_eq!(
            parsed.config,
            Some(ConfigSource::Path(PathBuf::from(source)))
        );
    }

    #[test]
    fn list_output_ignores_only_broken_pipe() {
        assert!(
            normalize_list_output(Err(io::Error::new(io::ErrorKind::BrokenPipe, "injected")))
                .is_ok()
        );
        assert_eq!(
            normalize_list_output(Err(io::Error::other("injected")))
                .expect_err("other errors should remain")
                .kind(),
            io::ErrorKind::Other
        );
    }
}

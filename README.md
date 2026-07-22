# dot

`dot` is a small, declarative bootstrap runner for personal development
environments and dotfiles across Linux, macOS, and Windows.

It reads a TOML manifest, selects one explicit target and optional profile,
then coordinates external package providers, manual installation actions,
generic actions, and symbolic links. The manifest stays readable as an
inventory of the environment it describes.

> [yslib/dotfiles](https://github.com/yslib/dotfiles) is the complete application
> example used to develop `dot`. It describes an Arch Linux base/desktop/laptop
> profile tree plus independent macOS and Windows environments.

`dot` is intentionally not a package manager. It does not search repositories,
resolve dependencies, compare versions, implement installers, or keep an
installed-state database. Those responsibilities remain with commands such as
`pacman`, `brew`, `scoop`, `npm`, and `cargo`.

## Installation

Prebuilt binaries are published on the
[GitHub Releases](https://github.com/yslib/dot/releases) page for:

- Linux x86-64, statically linked with musl;
- macOS Apple Silicon;
- Windows x86-64.

Rename the downloaded asset to `dot` (`dot.exe` on Windows), make it executable
where necessary, and place it on `PATH`.

To build from source with stable Rust:

```console
git clone https://github.com/yslib/dot.git
cd dot
cargo build --release
```

The binary is written to `target/release/dot` (`dot.exe` on Windows).

## Quick start

Create `dot.toml` in the directory containing the files you want to manage:

```toml
[targets.workstation]
platform = { os = "linux", distro = ["debian", "ubuntu"] }

[targets.workstation.providers.apt]
probe   = { program = "apt-get", args = ["--version"] }
install = { program = "sudo", args = ["apt-get", "install", "-y", "${package:names}"] }

[targets.workstation.packages]
git     = { provider = "apt" }
ripgrep = { provider = "apt" }

[targets.workstation.links]
nvim = { source = "config/nvim", target = "${xdg:config}/nvim" }
```

Inspect the selected intent without running commands or touching links:

```console
dot --target workstation --dry-run
```

Check whether the selected providers are currently available:

```console
dot --target workstation check providers
```

Apply the environment:

```console
dot --target workstation
```

`dot.toml` is the default configuration path. `--target` may be omitted when
the file declares exactly one target.

## Command line

```text
dot [OPTIONS]
dot [OPTIONS] check providers
```

With no subcommand, `dot` applies the selected environment. Selection options
are global and may appear before or after `check providers`.

| Option | Meaning |
| --- | --- |
| `-c, --config <PATH>` | TOML manifest; defaults to `./dot.toml` |
| `-t, --target <TARGET>` | Target id; optional only when exactly one target exists |
| `-p, --profile <PROFILE>` | One unique profile node name inside the target |
| `--dry-run` | Render the resolved apply plan without executing it |
| `-h, --help` | Print command help |
| `-V, --version` | Print the `dot` version |

`--dry-run` performs configuration loading, target/profile selection, platform
validation, profile merging, interpolation, and planning. It does not execute
provider, package, or action commands and does not inspect or modify link state.
Its output describes intent, not whether the current machine can satisfy it.

`check providers` applies each provider's in-memory environment patch and runs
its probe once. It does not run ensure or install, process packages or actions,
or inspect links. All effective providers are checked even if one is not ready.

Apply uses the same resolved execution plan as dry-run. It ensures providers,
runs provider package batches, applies manual packages and global actions, then
reconciles links. Unrelated work continues after an individual runtime failure.
The final report exits non-zero if any item failed or was blocked.

## Configuration model

A manifest is deliberately small:

```text
targets
└── target
    ├── platform
    ├── providers
    ├── packages
    ├── links
    ├── actions
    └── profiles
        └── profile
            └── profiles ...
```

Targets are independent, complete environment declarations. Profiles are
nested inline and form a tree. Selecting a profile by its unique name merges
only the path from the target root to that node. For `providers`, `packages`,
`links`, and `actions`, a deeper record with the same id replaces the complete
ancestor record; fields are not merged individually.

Unknown fields are rejected. Identifiers must be non-empty and cannot contain
interpolation. Profile names must be unique within a target and should not
contain `/`, because the CLI accepts a node name rather than a path.

### Root

| Field | Required | Meaning |
| --- | --- | --- |
| `targets` | yes | Table mapping globally unique target ids to target records |

```toml
[targets.linux]
platform = { os = "linux" }

[targets.macos]
platform = { os = "macos" }
```

### Target

| Field | Required | Meaning | Default |
| --- | --- | --- | --- |
| `platform` | yes | Compatibility constraint checked against the current machine | none |
| `providers` | no | Provider records keyed by provider id | empty |
| `packages` | no | Provider-backed or manual packages keyed by package id | empty |
| `links` | no | Native symbolic-link intents keyed by link id | empty |
| `actions` | no | Generic idempotent actions keyed by action id | empty |
| `profiles` | no | Nested profile nodes keyed by profile name | empty |

A target is never inferred from platform facts. When multiple targets exist,
the user must select one with `--target`.

### Platform constraint

Each field accepts either one string or a list of strings. Declared fields are
combined with AND; values inside one field are combined with OR.

| Field | Required | Meaning |
| --- | --- | --- |
| `os` | yes | Operating system, normally `linux`, `macos`, or `windows` |
| `arch` | no | Rust target architecture such as `x86_64` or `aarch64` |
| `distro` | no | Linux `ID` from `os-release`, such as `arch` or `ubuntu` |
| `distro_family` | no | Linux `ID_LIKE` family, such as `debian` |
| `environment` | no | Runtime class: `native`, `wsl`, or `container` |

```toml
[targets.server]
platform = { os = "linux", arch = ["x86_64", "aarch64"], distro_family = ["debian", "rhel"], environment = ["native", "container"] }
```

A selected target must match every declared field. A mismatch is an error,
not a reason to silently skip declarations.

### Profile

A profile supports the same optional `providers`, `packages`, `links`,
`actions`, and child `profiles` fields as a target. It cannot redefine
`platform`.

```toml
[targets.arch]
platform = { os = "linux", distro = "arch" }

[targets.arch.providers.pacman]
probe   = { program = "pacman", args = ["--version"] }
install = { program = "sudo", args = ["pacman", "-S", "--needed", "${package:names}"] }

[targets.arch.packages]
git = { provider = "pacman" }

[targets.arch.profiles.desktop.packages]
waybar = { provider = "pacman" }

[targets.arch.profiles.desktop.profiles.laptop.packages]
tlp = { provider = "pacman" }
```

`--profile laptop` selects `arch -> desktop -> laptop`, so the effective package
set contains `git`, `waybar`, and `tlp`. Sibling profiles are alternatives.
There is no `extends`, multiple inheritance, deletion, or cross-target reuse.

### Provider

A provider is an external package-installation capability. Every effective
provider is probed and, when necessary and possible, ensured before any package
batch is installed.

| Field | Required | Meaning |
| --- | --- | --- |
| `probe` | yes | Exec action with no package input; exit 0 means ready |
| `activate` | no | Environment patch applied to this provider's child processes |
| `ensure` | no | One exec action or an ordered list used when the initial probe is not ready |
| `install` | yes | Exec action used once for each non-empty package batch |

```toml
[targets.macos]
platform = { os = "macos" }

[targets.macos.providers.brew]
activate = { path_prepend = ["/opt/homebrew/bin", "/usr/local/bin"] }
probe    = { program = "brew", args = ["--version"] }
ensure   = { program = "bash", args = ["${dot:config_dir}/scripts/install-homebrew.sh"] }
install  = { program = "brew", args = ["install", "${package:provider_args}", "${package:names}"] }
```

If the first probe is ready, ensure is skipped. Otherwise ensure actions run in
declaration order and stop at the first failure. `dot` then reapplies activate
and probes once more. A provider is usable only when that final probe succeeds.

`dot` does not resolve dependencies between providers. An ensure command may
invoke other programs, but their availability is part of that provider's own
bootstrap contract.

On Windows, `dot` does not reload User or Machine `PATH` from the registry after
ensure. Use `activate.path_prepend` to make the provider executable visible to
the current `dot` process.

### Package

A package has exactly one of two forms.

#### Provider package

| Field | Required | Meaning | Default |
| --- | --- | --- | --- |
| `provider` | yes | Id of one effective provider | none |
| `provider_args` | no | Ordered literal argv values used to form a separate batch | empty |

The package table key is the exact package name passed to the provider. This
makes platform-specific names explicit in each target.

```toml
[targets.macos.packages]
ripgrep             = { provider = "brew" }
font-hack-nerd-font = { provider = "brew", provider_args = ["--cask"] }
```

Packages are grouped by provider id and their exact `provider_args` list. If
any package for a provider has non-empty `provider_args`, that provider's
`install.args` must contain `${package:provider_args}` exactly once.

#### Manual package

| Field | Required | Meaning |
| --- | --- | --- |
| `install` | yes | A generic action that installs this one package |

```toml
[targets.server]
platform = { os = "linux" }

[targets.server.packages.ripgrep]
install = { check = { program = "bash", args = ["-c", "command -v rg >/dev/null 2>&1"] }, exec = { program = "bash", args = ["${dot:config_dir}/scripts/install-ripgrep.sh"] } }
```

Manual packages are not batched. Download selection, checksums, extraction, or
other procedural work normally belongs in the external script.

### Environment patch

Environment patches are used by `provider.activate` and `exec.env`.

| Field | Required | Meaning | Default |
| --- | --- | --- | --- |
| `path_prepend` | no | One path or a list inserted before effective `PATH` | none |
| `path_append` | no | One path or a list appended after effective `PATH` | none |
| `variables` | no | Environment variables added or replaced by name | empty |

```toml
activate = { path_prepend = "${xdg:executable}", variables = { CARGO_HOME = "${xdg:data}/cargo" } }
```

Provider activation is applied after the environment captured when `dot`
starts. An exec action's own patch is applied after provider activation, so its
variables override provider variables and its prepended paths appear first.
Global and manual-package actions do not inherit a provider environment.

### Exec action

An exec action describes one process invocation.

| Field | Required | Meaning | Default |
| --- | --- | --- | --- |
| `type` | no | Reserved action kind; the only accepted value is `"exec"` | exec |
| `program` | yes | Executable path or name | none |
| `args` | no | Ordered argv elements | empty |
| `cwd` | no | Child-process working directory | inherited |
| `env` | no | Environment patch applied to this command | none |

```toml
exec = { program = "git", args = ["status", "--short"], cwd = "${dot:config_dir}", env = { variables = { NO_COLOR = "1" } } }
```

Commands are started directly without an implicit shell. Pipes, redirects,
`&&`, shell expansion, and shell quoting have no special meaning. Request shell
behavior explicitly:

```toml
exec = { program = "bash", args = ["-c", 'printf "%s\n" hello'] }
```

### Action

| Field | Required | Meaning |
| --- | --- | --- |
| `check` | no | Exec action that tests whether the desired state already exists |
| `exec` | yes | Exec action that establishes the desired state |

```toml
[targets.linux]
platform = { os = "linux" }

[targets.linux.actions.install-shell-config]
check = { program = "test", args = ["-d", "${xdg:home}/.oh-my-zsh"] }
exec  = { program = "bash", args = ["${dot:config_dir}/scripts/install-oh-my-zsh.sh"] }
```

Check exit codes have a fixed contract:

- 0: already satisfied; skip exec;
- 1: not satisfied; run exec, then run the same check once more;
- any other result: fail the action.

Without check, exec runs on every apply. `dot` persists no action state, so the
configuration author owns check correctness and command idempotency.

### Link

| Field | Required | Meaning | Default |
| --- | --- | --- | --- |
| `source` | yes | Existing file or directory; relative paths use the manifest directory | none |
| `target` | yes | Link target path; must be absolute after interpolation | none |
| `on_conflict` | no | `error` or `replace-link` for an incorrect existing symlink | `replace-link` |
| `on_missing_parent` | no | `create` or `skip` when the target parent is absent | `create` |

```toml
[targets.windows]
platform = { os = "windows" }

[targets.windows.links]
terminal = { source = "config/windows-terminal/settings.json", target = "${env:LOCALAPPDATA}/Packages/Terminal/LocalState/settings.json", on_conflict = "replace-link", on_missing_parent = "skip" }
```

`dot` creates native symbolic links. It never replaces an existing regular file
or directory, even with `replace-link`. It verifies newly created links and
rejects duplicate effective target paths before link mutation begins. Removing
a declaration does not remove a previously created link.

## Built-in interpolation

Interpolated strings use an OmegaConf-like surface:

```text
${resolver:payload}
```

The resolver registry is closed: TOML cannot define new resolvers. Missing
environment variables, unavailable standard paths, unknown resolvers, and
invalid resolver contexts are errors rather than empty strings.

### Scalar resolvers

Scalar resolvers produce one string and may be a complete value or part of a
larger string.

| Resolver | Value |
| --- | --- |
| `${env:NAME}` | `NAME` from the current effective child environment |
| `${dot:config}` | Absolute path of the loaded TOML file |
| `${dot:config_dir}` | Directory containing the TOML file |
| `${dot:cwd}` | Directory from which `dot` was invoked |
| `${xdg:home}` | Current user's home directory |
| `${xdg:config}` | Standard configuration directory |
| `${xdg:config_local}` | Local/non-roaming configuration directory |
| `${xdg:data}` | Standard user data directory |
| `${xdg:data_local}` | Local/non-roaming user data directory |
| `${xdg:cache}` | Standard user cache directory |
| `${xdg:state}` | Standard user state directory, when defined |
| `${xdg:runtime}` | Standard runtime directory, when defined |
| `${xdg:executable}` | User-writable executable directory, when defined |
| `${xdg:documents}` | Current user's Documents directory, when available |

`xdg` names are portable vocabulary backed by the host platform's standard
directories; they are not Linux-only paths.

### Provider package resolvers

These resolvers produce lists and are available only as complete elements of
`provider.install.args`:

| Resolver | Value |
| --- | --- |
| `${package:names}` | Complete package-name batch |
| `${package:provider_args}` | Shared `provider_args` list for that batch |

`provider_args` values themselves are literal and cannot contain resolvers.
To write a literal `${`, escape it as `\${`. Resolved values are passed as data
and are never reinterpreted as shell syntax.

## Reports and side-effect boundaries

Dry-run, apply, and provider check produce the same presentation-independent
report model and render it as a table with one row per logical provider,
package, action, or link. Provider batching remains an internal execution
detail.

Dry-run is side-effect-free with respect to managed state, but provider probes
are arbitrary external commands: `check providers` is diagnostic, not a
side-effect-free simulation. Apply streams interactive child output and prints
its final report afterward.

The current table output is intended for people and is not a stable serialized
interface. v0.0.1 does not provide `--json`.

## Goals

- Make a personal development environment reproducible without hiding the
  commands that establish it.
- Keep each target locally complete, even when independent targets repeat data.
- Model only the small amount of domain knowledge needed for cross-platform
  bootstrap work.
- Keep the manifest readable as an explicit inventory.
- Let procedural edge cases remain ordinary shell or PowerShell scripts.

## Non-goals

`dot` is not a package manager, a general-purpose configuration language, or a
system orchestration platform. It deliberately omits:

- repository search, dependency solving, versions, updates, and uninstall;
- package or link receipts and other persistent managed-state databases;
- automatic target selection from platform facts;
- per-item condition expressions and arbitrary evaluation;
- cross-target inheritance, profile references, and multiple inheritance;
- action dependency graphs and provider dependency resolution;
- built-in download, archive, checksum, build, or service-management logic;
- implicit shell execution;
- link removal, garbage collection, copies, and fallback link strategies.

If installation logic is inherently procedural, invoke a script. If a feature
would require `dot` to understand how a particular package manager or arbitrary
program works, it probably belongs outside `dot`.

## Development platform override

The development-only `dev-platform-override` Cargo feature adds
`--platform <TOML>` for testing target selection:

```console
cargo run --features dev-platform-override -- \
  --dry-run \
  --platform '{ os = "windows", arch = "x86_64" }'
```

Dry-run and `check providers` use the injected PlatformInfo only for target
selection. Commands, environment variables, XDG paths, and filesystem state
still belong to the host. Apply accepts but ignores the override and always
uses detected host facts. `dot` prints a warning whenever this option is used.
Without the feature, `--platform` is not part of the CLI.

## Further reading

- [Configuration schema](docs/SCHEMA.txt) — exact field shapes and string roles.
- [Design](docs/DESIGN.txt) — runtime semantics, execution boundaries, and
  explicit design decisions.
- [yslib/dotfiles](https://github.com/yslib/dotfiles) — complete real-world
  configuration for Linux, macOS, and Windows.

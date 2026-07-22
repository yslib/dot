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
runs each declared provider install unit, applies manual packages and global
actions, then reconciles links. Unrelated work continues after an individual
runtime failure. The final report exits non-zero if any item failed or was
blocked.

## Configuration

Configuration is intentionally finite and explicit. A target is a complete
environment declaration; a selected nested profile inherits the target and only
the profile nodes along the path to the selected node. Deeper keyed records
replace complete earlier records.

A provider declares how to probe and install, with optional activation and
ensure actions. Every provider-backed package is one declared provider install
unit: a Single uses its table key as the name, while a Batch supplies an
explicit non-empty `names` list. `dot` does not infer grouping.

<!-- readme-configuration-example:start -->

```toml
[targets.workstation]
platform = { os = "macos" }

[targets.workstation.providers.brew]
probe   = { program = "brew", args = ["--version"] }
install = { program = "brew", args = ["install", "${package:names}"] }

[targets.workstation.packages.ripgrep]
provider = "brew"

[targets.workstation.packages.cli-tools]
provider = "brew"
names = ["bat", "fd"]
```

<!-- readme-configuration-example:end -->

A manual package contains its own install action. Generic actions describe
other idempotent work, and links map existing sources to native symlink targets.

See the [Configuration Reference](docs/CONFIGURATION.md) for the complete types,
fields, examples, and interpolation rules.

## Reports and side-effect boundaries

Dry-run, apply, and provider check share the same presentation-independent
report model and table rendering, but cover different items:

- **Dry-run and apply:** report each logical provider, one item for each declared
  Single or Batch provider install unit, each manual package, action, and link.
- **`check providers`:** reports only effective providers and their readiness.

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

- [Configuration Reference](docs/CONFIGURATION.md) — user-facing types, fields,
  examples, and interpolation rules.
- [Configuration schema](docs/SCHEMA.txt) — authoritative structural schema and
  string roles.
- [Design](docs/DESIGN.txt) — runtime and design semantics, execution boundaries,
  and explicit design decisions.
- [yslib/dotfiles](https://github.com/yslib/dotfiles) — complete real-world
  configuration for Linux, macOS, and Windows.

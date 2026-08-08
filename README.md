# dot

`dot` is a conservative, declarative bootstrap runner for personal development
environments and dotfiles across Linux, macOS, and Windows.

It reads a TOML manifest, selects one target and optional profile, and
coordinates external package providers, manual installation actions, generic
actions, and symbolic links. The manifest remains a readable inventory of the
environment it describes.

The Rust workspace separates the reusable `dot-core` library from `dot-cli`,
the package that produces the `dot` executable.

> [yslib/dotfiles](https://github.com/yslib/dotfiles) is the complete
> application example used to develop `dot`. It describes an Arch Linux
> base/Hyprland/laptop profile tree plus independent macOS and Windows
> environments.

`dot` is intentionally not a package manager or a general-purpose
configuration DSL. It does not search repositories, solve dependencies,
compare versions, implement installers, or keep an installed-state database.
Those responsibilities remain with declared commands such as `pacman`, `brew`,
`scoop`, `npm`, and `cargo`.

![macos](./assets/macos.gif)

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
cargo build --release -p dot-cli --bin dot
```

The binary is written to `target/release/dot` (`dot.exe` on Windows).

## Quick start

Create `.dot.toml` in the directory containing the files you want to manage:

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

Inspect the complete selected intent without executing it:

```console
dot dry-run --target workstation
```

Check whether the effective providers are currently available:

```console
dot check providers --target workstation
```

Apply the environment:

```console
dot apply --target workstation
```

Without an explicit source, `dot` checks `./.dot.toml` first, then
`~/.config/dot/.dot.toml` on Linux and macOS or
`%APPDATA%\dot\.dot.toml` on Windows. The first candidate whose filesystem
entry exists is selected; load, parse, or validation errors for that path do
not fall through to another candidate. An explicit local path bypasses
discovery and may use any filename. See the
[local filesystem details](docs/CONFIGURATION.md#local-filesystem).

Load a manifest directly from HTTPS:

```console
dot --config https://example.com/dot.toml dry-run --target workstation
```

Use the root `.dot.toml` from a persistent Git worktree:

```console
dot --git https://example.com/dotfiles.git \
  --git-worktree .dot-worktree \
  dry-run --target workstation
```

`--config SOURCE` accepts a local path or HTTPS URL and bypasses discovery. Git
clones a missing worktree and reuses an existing matching worktree without
updating it. See the [HTTPS](docs/CONFIGURATION.md#https) and
[Git worktree](docs/CONFIGURATION.md#git-worktree) guarantees for the complete
acquisition contract.

## Command line

The command is always explicit; its configuration source may be discovered or
selected with one of these forms:

```text
dot [--config SOURCE] <COMMAND>
dot --git REPOSITORY --git-worktree PATH <COMMAND>
```

`--config` conflicts with `--git`; `--git` and `--git-worktree` must be supplied
together. `<COMMAND>` is one of:

```text
apply
    [--target TARGET] [--profile PROFILE] [--job KIND:ID]...

dry-run
    [--target TARGET] [--profile PROFILE] [--job KIND:ID]...

check providers
    [--target TARGET] [--profile PROFILE]

list targets [--all]

list profiles
    [--target TARGET]

list jobs
    [--target TARGET] [--profile PROFILE]
```

The optional profile is one globally unique profile node name inside the
target. Omitting it selects the target root; `--profile @root` is the explicit
equivalent. If the target is omitted, selection succeeds only when exactly one
target is compatible. Execution commands and provider check reject an explicit
incompatible target. `list profiles` and `list jobs` may inspect an explicitly
named incompatible target structurally.

### Exact job selection

Omitting `--job` selects all effective jobs. Repeat `--job` to select exact
package, action, and link jobs:

```console
dot dry-run --target workstation \
  --job package:ripgrep \
  --job link:nvim

dot apply --target workstation \
  --job package:ripgrep \
  --job link:nvim
```

The accepted forms are exactly:

```text
package:ID
action:ID
link:ID
```

A provider-backed package automatically includes its required provider.
Providers cannot be selected directly, and a Batch package remains one
indivisible job. Selector argument order does not control execution order.

## Machine-readable lists

The `list` commands provide machine-facing catalogs for shell scripts,
completion engines, fzf, and other external tools. Output is UTF-8, headerless
TSV with one record per line and fixed columns:

```text
list targets:   TARGET  COMPATIBILITY  OS  ARCH  DISTRO  DISTRO_FAMILY  ENVIRONMENT
list profiles:  PROFILE PATH           DEPTH
list jobs:      SELECTOR KIND          ID  VIA  DETAIL
```

`list targets` includes only compatible targets by default; `--all` includes
every target and labels compatibility. `list profiles` includes the root row
`@root<TAB><root><TAB>0`. `list jobs` describes the unresolved effective
package, action, and link records and never emits provider rows. Row order is
not guaranteed and may reflect current containers or traversal.

The first field is the canonical reusable selector and is written verbatim.
In later fields, backslash, tab, carriage return, and newline are escaped as
`\\`, `\t`, `\r`, and `\n`. All records are prepared before stdout is written,
so configuration or selection errors leave stdout empty. A downstream broken
pipe is normal successful termination.

See the [Design Model](docs/DESIGN.txt) for the stable meaning of every TSV
column.

### External fzf composition

This Bash example lets fzf select one or more complete TSV rows, extracts each
canonical first field, and passes it as a separately quoted `--job` argument:

```bash
#!/usr/bin/env bash
set -euo pipefail

target=workstation
profile=@root
command=${1:-dry-run}

case "$command" in
  apply|dry-run) ;;
  *) printf 'usage: %s [apply|dry-run]\n' "$0" >&2; exit 2 ;;
esac

selection_file=$(mktemp)
cleanup() {
  rm -f "$selection_file"
}
trap cleanup EXIT

if ! dot list jobs --target "$target" --profile "$profile" |
  fzf --multi > "$selection_file"; then
  printf 'job selection failed\n' >&2
  exit 1
fi

selectors=()
while IFS=$'\t' read -r selector _; do
  selectors+=("$selector")
done < "$selection_file"

((${#selectors[@]})) || {
  printf 'no jobs selected\n' >&2
  exit 1
}

job_args=()
for selector in "${selectors[@]}"; do
  job_args+=(--job "$selector")
done

dot "$command" \
  --target "$target" \
  --profile "$profile" \
  "${job_args[@]}"
```

Run the wrapper with `dry-run` to inspect the chosen plan or `apply` to execute
it. `dot` never invokes, configures, or depends on fzf; it does not read
selection from stdin. The wrapper turns fzf output into ordinary command-line
selectors before starting `apply` or `dry-run`, leaving their stdin available
to interactive child commands.

## Configuration

Configuration is intentionally finite and explicit. A target is a complete
environment declaration. A selected nested profile inherits the target and
only the profile nodes along the path to that node. Deeper keyed records
replace complete earlier records; record fields and lists do not merge.

A provider declares how to probe and install, with optional activation and
ensure actions. Every provider-backed package is one declared install unit: a
Single uses its table key as the package name, while a Batch supplies an
explicit non-empty `names` list. `dot` never coalesces separately declared
Single or Batch units into one install invocation or report unit, even when
their provider and arguments match. Provider grouping affects scheduling order
only.

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

A manual package contains its own command install action. Generic actions can
run idempotent commands or use Fetch Content, a deliberately small built-in
capability for materializing one HTTPS resource at one local path:

```toml
[targets.workstation.actions.remote-config]
source = "https://example.com/tool/config.toml"
target = "config/tool/config.toml"
on_conflict = "replace"
```

An eligible apply always fetches the source fresh. The only conflict policies
are `error` (the default) and `replace`; Fetch Content is not a download,
artifact, or cache manager. Richer transfers belong in command actions or
scripts. Links map existing sources to native symlink targets. The complete
configuration is statically validated at load, while environment and path
values are resolved only for the selected execution closure.

See the [Configuration Reference](docs/CONFIGURATION.md) for the complete
types, fields, examples, and interpolation rules, including the
[Fetch Content Action](docs/CONFIGURATION.md#fetch-content-action).

## Execution and diagnostics

Apply and dry-run build the same selected, resolved execution plan. Dry-run
renders that plan without running provider, package, or action commands and
without inspecting or changing link state. Its output describes intent, not
whether the current machine can satisfy it.

Apply executes serially in stable phases: providers in final effective provider
declaration order; manual packages in final effective package declaration
order; provider-backed packages grouped by final effective provider declaration
order and, within each group, by final effective package declaration order;
then actions and links in their final effective declaration order. Every
selected provider completes its readiness lifecycle before the package phase
starts. A provider failure blocks only selected packages that require it;
unrelated selected work continues. Planning is atomic, and the final report
exits non-zero if any selected item failed or was blocked.

`check providers` is separate from job selection and independently attempts
every effective provider. Activation resolution/application or probe
resolution/preparation can produce `NOT_READY` before a process is launched.
When preparation succeeds, the probe process executes at most once. Any
provider-local failure does not stop later providers from being attempted. The
command does not run ensure or install, process packages or actions, or
inspect links. Because a launched probe is an arbitrary external command,
provider check is diagnostic, not a side-effect-free simulation.

Apply and dry-run reports are human-readable tables, not stable serialized
interfaces. The list-command TSV contract is stable only for its column shape,
escaping, selector field, and field meanings; it does not guarantee row order.
Version 0.1.0 does not provide `--json`.

## Goals

- Make a personal development environment reproducible without hiding the
  commands that establish it.
- Keep each target locally complete, even when independent targets repeat data.
- Model only the small amount of domain knowledge needed for cross-platform
  bootstrap work.
- Keep the manifest readable as an explicit inventory.
- Let procedural edge cases remain ordinary shell or PowerShell scripts.

## Non-goals

`dot` deliberately omits:

- repository search, dependency solving, versions, updates, and uninstall;
- package or link receipts and other persistent managed-state databases;
- per-item condition expressions and arbitrary evaluation;
- cross-target inheritance, profile references, and multiple inheritance;
- action dependency graphs and provider dependency resolution;
- download or artifact management beyond one-shot Fetch Content, including
  release selection, caching, archive extraction, build, or service-management
  logic;
- implicit shell execution;
- link removal, garbage collection, copies, and fallback link strategies.

If installation logic is inherently procedural, invoke a script. If a feature
would require `dot` to understand how a particular package manager or arbitrary
program works, it probably belongs outside `dot`.

## Development platform override

The development-only `dev-platform-override` Cargo feature adds the global
`--platform <TOML>` option for compatibility testing:

```console
cargo run -p dot-cli --features dev-platform-override -- \
  --platform '{ os = "windows", arch = "x86_64" }' \
  dry-run
```

The injected `PlatformInfo` controls compatibility for dry-run, provider check,
and target listings, and omitted-target inference for profile/job listings.
Commands, environment variables, XDG paths, and filesystem state still belong
to the host. Apply accepts but ignores the override and always uses detected
host facts. `dot` prints a warning whenever this option is used. Without the
feature, `--platform` is not part of the CLI.

## Further reading

- [Configuration Reference](docs/CONFIGURATION.md) — user-facing types, fields,
  examples, and interpolation rules.
- [Configuration schema](docs/SCHEMA.txt) — authoritative structural schema and
  string roles.
- [Design](docs/DESIGN.txt) — runtime and design semantics, execution
  boundaries, and explicit design decisions.
- [Unified job execution](docs/JOB_EXECUTION.md) — selected-plan architecture,
  serial order, and failure behavior.
- [Future directions](docs/FUTURE.md) — non-normative exploration plans and
  product boundaries for possible post-v0.1.0 work.
- [yslib/dotfiles](https://github.com/yslib/dotfiles) — complete real-world
  configuration for Linux, macOS, and Windows.

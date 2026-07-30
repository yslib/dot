# dot 0.1.0

This document is a frozen, self-contained archive of the behavior delivered by
`dot` 0.1.0. It describes that release as it existed; it is not a living
specification and does not track later changes.

## Product boundary

`dot` is a conservative, declarative bootstrap runner for personal development
environments and dotfiles across Linux, macOS, and Windows.

Its job is deliberately small:

1. Read one TOML manifest.
2. Select one platform target and an optional profile.
3. Produce a concrete execution plan.
4. Ensure the selected providers are ready.
5. Install declared packages, run generic actions, and create symbolic links.
6. Report what was planned or completed.

`dot` understands a few minimum domain concepts—platforms, profiles, providers,
packages, actions, and links—but it does not understand the behavior of a
specific package manager. Commands such as `pacman`, `brew`, `scoop`, `cargo`,
or `npm` are data supplied by the manifest.

`dot` 0.1.0 is not:

- a package manager;
- a dependency solver;
- an installed-package database;
- an update, uninstall, or rollback system;
- a general-purpose programming language;
- a shell;
- a universal machine-state convergence engine.

The manifest is expected to state the user's intended bootstrap procedure
explicitly. Some repetition is accepted in exchange for predictable behavior
and readable platform-specific configurations.

## Release shape

The project is a Rust 2024 command-line application licensed under MIT. Version
0.1.0 can be built as a single executable named `dot`.

The release workflow produces binaries for:

- Linux x86-64 using musl, for a statically linked executable independent of
  glibc;
- macOS on Apple Silicon;
- Windows x86-64.

## Configuration discovery

An explicit configuration path can be supplied with `--config PATH`.

Without `--config`, `dot` searches in this order:

1. `.dot.toml` in the current working directory;
2. the user fallback path:
   - Linux and macOS: `~/.config/dot/.dot.toml`;
   - Windows: `%APPDATA%\dot\.dot.toml`.

The first existing candidate is selected. If that candidate exists but is
invalid, `dot` reports the error instead of silently falling through to another
file.

The selected configuration path is canonicalized when loaded. A broken
symbolic link, inaccessible target, or other canonicalization failure is a
configuration-loading error even if the manifest never uses the real-path
interpolation values.

The entry path and the canonical entity path remain distinct:

- the entry path preserves where the user addressed the manifest;
- the real path identifies the manifest file after following symbolic links.

## Configuration root

The root contains only a `targets` table:

```toml
[targets.macos]
platform = { os = "macos", arch = "aarch64" }

[targets.windows]
platform = { os = "windows", arch = "x86_64" }
```

Unknown fields are rejected. The entire configuration is statically validated
before a target, profile, or subset of jobs is executed.

## Targets and platforms

Each target is a complete environment family and has:

- a required platform constraint;
- zero or more providers;
- zero or more packages;
- zero or more actions;
- zero or more links;
- zero or more nested profiles.

A platform constraint requires `os` and may additionally constrain `arch`,
`distro`, `distro_family`, and `environment`. Every constraint accepts either
one value or a list of acceptable values.

```toml
[targets.arch-linux]
platform = {
  os = "linux",
  arch = ["x86_64", "aarch64"],
  distro = "arch"
}
```

When `--target` is omitted, selection succeeds only if exactly one configured
target is compatible with the current platform. An explicitly selected target
must also be compatible for `apply`, `dry-run`, and provider checks.

Target and selectable job identifiers use a restricted spelling suitable for
command-line selectors: the first character must be an ASCII letter, digit, or
underscore, and subsequent characters may additionally contain periods and
hyphens.

## Profiles

Profiles form an inline tree within a target:

```toml
[targets.arch-linux]
platform = { os = "linux", distro = "arch" }

[targets.arch-linux.packages.git]
provider = "pacman"

[targets.arch-linux.profiles.hyprland.packages.hyprland]
provider = "pacman"

[targets.arch-linux.profiles.hyprland.profiles.laptop.packages.tlp]
provider = "pacman"
```

A profile is selected by its node name, not by a path. Profile names must
therefore be unique within a target. Omitting `--profile`, or explicitly using
`--profile @root`, selects the target root.

Selecting a profile merges the nodes along the single path from the target root
to that profile. Providers, packages, actions, and links are keyed records. A
deeper record with the same key replaces the inherited record atomically:
fields and lists are not merged individually.

The 0.1.0 inheritance model intentionally has:

- no cross-target inheritance;
- no references between profiles;
- no multiple inheritance;
- no deletion or tombstone operation;
- no inheritance graph or cycle resolution.

The inline tree supplies the ordering relation structurally, so profile
evaluation is simply a root-to-node merge.

## Providers

A provider describes how to make one installation mechanism ready and how to
install packages through it:

```toml
[targets.arch-linux.providers.pacman]
probe = { program = "pacman", args = ["--version"] }
install = {
  program = "sudo",
  args = ["pacman", "-S", "--needed", "${package:names}"]
}
```

A provider has:

- `probe`: required command that reports whether the provider is usable;
- `activate`: optional environment patch applied to that provider's child
  commands;
- `ensure`: optional command or list of commands used to install or initialize
  the provider;
- `install`: required package-install command.

Provider readiness follows this lifecycle:

1. Apply the provider's activation environment.
2. Run `probe`.
3. If the probe succeeds, the provider is ready.
4. If the probe failure is eligible for recovery and `ensure` is configured,
   run each `ensure` command in order. A malformed command configuration fails
   immediately instead.
5. Reapply activation and run `probe` once more.
6. The provider is ready only if the second probe succeeds.

Providers are treated as top-level bootstrap dependencies. `dot` does not
resolve dependencies between providers. Activation changes only the environment
of spawned child processes; it does not mutate the parent shell.

## Packages

Packages have two forms.

### Single provider package

The table key is both the job identifier and the concrete package name:

```toml
[targets.arch-linux.packages.ripgrep]
provider = "pacman"
```

### Batch provider package

The table key is a logical batch identifier, while `names` contains the concrete
package names passed to the provider:

```toml
[targets.arch-linux.packages.cli-tools]
provider = "pacman"
names = ["bat", "fd", "fzf", "ripgrep"]
```

A batch is one indivisible execution and reporting unit. It avoids making all
packages from one provider a single accidental mega-batch while still allowing
packages that belong together to be installed in one invocation.
`names` must be nonempty and must not contain duplicates.

Both provider package forms may supply literal `provider_args`:

```toml
[targets.macos.packages.desktop-apps]
provider = "brew"
names = ["alacritty", "visual-studio-code"]
provider_args = ["--cask"]
```

When `provider_args` is nonempty, the provider's `install.args` must contain
exactly one `${package:provider_args}` element.

Each provider package refers to exactly one provider. There is no fallback
chain, provider competition, or package-name mapping hidden inside `dot`.

### Manual package

A manual package contains a generic action instead of a provider reference:

```toml
[targets.linux.packages.example-tool.install]
check = { program = "example-tool", args = ["--version"] }
exec = {
  program = "sh",
  args = ["scripts/install-example-tool.sh"]
}
```

This form covers downloads, source builds, or other installations that do not
belong to a configured provider.

## Generic actions

An action contains an optional `check` command and a required `exec` command:

```toml
[targets.macos.actions.shell-setup]
check = { program = "test", args = ["-f", "${env:HOME}/.zshrc.local"] }
exec = { program = "sh", args = ["scripts/setup-shell.sh"] }
```

Action behavior is:

- `check` exits with 0: the action is already satisfied and `exec` is skipped;
- `check` exits with 1: run `exec`, then run `check` once more;
- `check` exits with any other code: fail the action;
- `exec` exits unsuccessfully: fail the action;
- the post-execution check exits unsuccessfully: fail the action.

Without `check`, the action always runs.

Commands are spawned directly. No implicit shell parses the command. Shell
syntax is available only when the manifest explicitly chooses a shell such as
`sh`, `zsh`, `pwsh`, or `cmd` as `program`.

An executable command supports:

- `program`;
- `args`;
- optional `cwd`;
- optional `env`;
- optional `type = "exec"`, whose only accepted 0.1.0 value is `exec`.

An environment patch supports:

- `path_prepend`;
- `path_append`;
- `variables`.

Path fields accept either one value or a list. Environment patches are layered
onto each spawned child's environment and do not persist after `dot` exits.

## Symbolic links

Links are a small built-in filesystem operation:

```toml
[targets.macos.links.nvim]
source = "${dot:config_dir}/nvim"
target = "${xdg:config}/nvim"
on_conflict = "replace-link"
on_missing_parent = "create"
```

`source` and `target` are resolved before execution. Symbolic links are created
through Rust filesystem APIs rather than an external command.

Conflict behavior:

- `error`: fail if the target already exists and is not already the desired
  link;
- `replace-link`: replace an existing incorrect or broken symbolic link, but
  never overwrite a regular file or directory.

Missing-parent behavior:

- `create`: create the target's parent directories;
- `skip`: leave the link untouched and report it as skipped.

The defaults are `replace-link` and `create`.

Before mutating the filesystem, the complete selected link phase is checked for
multiple link records resolving to the same normalized target. A duplicate
target blocks the link phase. This prevents one declared link from silently
replacing another.

Version 0.1.0 does not provide hard links, junction management, copying,
unlinking, or garbage collection.

## String expressions

Interpolation uses an OmegaConf-like surface syntax:

```text
${resolver:payload}
```

The resolver registry is closed and built into `dot`. Users cannot define
custom resolvers.

Version 0.1.0 provides:

| Expression | Result |
| --- | --- |
| `${env:NAME}` | Environment variable `NAME` as a string |
| `${dot:config}` | Selected manifest entry path |
| `${dot:config_dir}` | Directory containing the selected entry path |
| `${dot:real_config}` | Canonical manifest entity path |
| `${dot:real_config_dir}` | Directory containing the canonical entity |
| `${dot:cwd}` | Current working directory |
| `${xdg:home}` | User home directory |
| `${xdg:config}` | User configuration directory |
| `${xdg:config_local}` | Local configuration directory |
| `${xdg:data}` | User data directory |
| `${xdg:data_local}` | Local data directory |
| `${xdg:cache}` | User cache directory |
| `${xdg:state}` | User state directory |
| `${xdg:runtime}` | User runtime directory, when available |
| `${xdg:executable}` | User executable directory, when available |
| `${xdg:documents}` | User documents directory, when available |
| `${package:names}` | Current provider package names as a string list |
| `${package:provider_args}` | Current provider arguments as a string list |

The string layer distinguishes several roles:

- literal strings that cannot contain interpolation;
- scalar string expressions that may be literals, templates, or exact scalar
  variables;
- exact typed variables;
- flat argument-list expressions.

`${package:names}` and `${package:provider_args}` are list-valued. They may
appear only as complete elements of a provider's `install.args`; resolving them
splices their elements into the final argument vector. They cannot be embedded
inside surrounding text. `provider_args` values themselves are literal and
cannot contain interpolation.

For example:

```toml
install = {
  program = "brew",
  args = ["install", "${package:provider_args}", "${package:names}"]
}
```

Malformed expressions, unknown resolvers, unavailable values, and type
mismatches are errors. A literal `${` can be written as `\${`.

There are no nested expressions, defaults, functions, arbitrary node
references, or user-defined resolver code in 0.1.0.

## Command-line interface

Every operation is explicit; invoking `dot` without a subcommand does not imply
`apply`.

```text
dot [--config PATH] apply [--target TARGET] [--profile PROFILE] [--job KIND:ID]...
dot [--config PATH] dry-run [--target TARGET] [--profile PROFILE] [--job KIND:ID]...
dot [--config PATH] check providers [--target TARGET] [--profile PROFILE]
dot [--config PATH] list targets [--all]
dot [--config PATH] list profiles [--target TARGET]
dot [--config PATH] list jobs [--target TARGET] [--profile PROFILE]
```

Because `--config` is global, it may also be placed after a subcommand.

### Job selection

`apply` and `dry-run` share the same selection model. A job selector is one of:

```text
package:ID
action:ID
link:ID
```

`--job` may be repeated. Duplicate or nonexistent selectors are errors. If no
job selector is supplied, all effective jobs are selected.

Providers are not directly selectable jobs. Selecting a provider-backed package
automatically includes the provider required by that package. A batch package
is selected by its batch identifier; individual names inside that batch cannot
be selected.

All selectors are validated atomically before runtime interpolation begins.
Only the selected jobs and their required provider closure are subsequently
resolved.

### Catalog output

The three `list` commands emit stable, headerless UTF-8 TSV intended for shell
composition and tools such as `fzf`.

`list targets` fields:

```text
TARGET  COMPATIBILITY  OS  ARCH  DISTRO  DISTRO_FAMILY  ENVIRONMENT
```

By default it lists compatible targets; `--all` includes incompatible targets.

`list profiles` fields:

```text
PROFILE  PATH  DEPTH
```

The first row is `@root`, followed by the target's named profile nodes.

`list jobs` fields:

```text
SELECTOR  KIND  ID  VIA  DETAIL
```

Backslashes, tabs, carriage returns, and line feeds are escaped in data fields.
Output is prepared atomically, so a validation or selection error leaves stdout
empty. A broken stdout pipe is treated as successful termination.

`dot` does not invoke `fzf`, provide an interactive selector, or consume
interactive selections from stdin. Composition belongs to the calling shell.

## Planning, dry-run, and apply

One owned execution plan is the internal fact source for both `dry-run` and
`apply`. It contains the selected providers and jobs after target/profile
merging, dependency closure, interpolation, and command construction.

The plan has four serial phases:

1. providers;
2. packages;
3. actions;
4. links.

Within the plan, records use a canonical, deterministic order rather than TOML
declaration order or `--job` argument order. Provider-backed packages precede
manual packages.

### `dry-run`

`dry-run` builds and resolves the selected execution plan, then prints its
human-facing representation. It does not:

- spawn commands;
- run action checks;
- probe providers;
- inspect link state;
- mutate the filesystem.

It is therefore a resolved statement of intent, not a prediction that every
operation will succeed.

### `apply`

`apply` executes the same selected plan serially. Version 0.1.0 has no concurrent
execution mode or background job manager.

Failures are contained where the phase model permits:

- a provider failure blocks packages that depend on that provider;
- unrelated providers and packages continue;
- one package failure does not stop later package jobs;
- action and link failures are reported per item while safe remaining work
  continues.

There is no retry, resume, rollback, receipt, or persisted execution state.

Child processes use normal foreground operating-system behavior. Version 0.1.0
does not implement custom signal forwarding, detached process supervision, or
recursive process-tree termination.

## Provider checks

`check providers` inspects the effective providers for the selected target and
profile.

For every provider it:

1. applies the provider activation environment;
2. runs the provider probe;
3. records readiness.

It does not run `ensure`, install packages, execute actions, or inspect links.
One provider's failure does not prevent the remaining providers from being
checked.

This command is intentionally narrower than `apply`: it is a diagnostic view of
the current environment, not a repair operation.

## Reporting

Human-facing commands use a structured internal report model rendered as a
dense table:

```text
TYPE  ITEM  VIA  STATUS  DETAIL
```

`dry-run` reports selected entries as `PLANNED`.

`apply` can report:

- providers: `READY` or `FAILED`;
- packages: `INSTALLED`, `FAILED`, or `BLOCKED`;
- actions: `SATISFIED`, `EXECUTED`, or `FAILED`;
- links: `SATISFIED`, `CREATED`, `REPLACED`, `SKIPPED`, `FAILED`, or
  `BLOCKED`.

A final summary includes total and per-kind counts. Any failed or blocked item
causes an unsuccessful command result.

Raw operating-system errors are retained. A small extensible diagnostic layer
may add platform-specific guidance—for example, explaining the Windows
privilege error returned when symbolic-link creation is not permitted—without
hiding the original error.

Version 0.1.0 has no JSON output mode.

## Development-only platform override

When compiled with the `dev-platform-override` feature, `dot` accepts a global
`--platform TOML` argument for compatibility-selection tests.

The injected value affects only runtime platform matching. It does not simulate
the target operating system:

- environment variables remain those of the host;
- XDG and user directories remain those of the host;
- commands run on the host;
- filesystem behavior remains that of the host.

`apply` ignores the override and emits a warning. The feature exists to inspect
and test selection behavior, not to emulate another platform.

## Explicit omissions in 0.1.0

The following capabilities were intentionally outside the release:

- package repository search, version selection, dependency solving, update, or
  uninstall;
- package receipts, installed-state tracking, rollback, retry, or resume;
- package fallback chains, all-of provider semantics, or implicit
  provider-specific package-name aliases;
- cross-target inheritance, global reusable data, templates, or data
  references;
- `when` conditions, general evaluation, loops, functions, or a programmable
  DSL;
- provider dependency graphs or action DAGs;
- concurrent job execution;
- built-in HTTP downloads, archive extraction, checksums, source builds, or
  service management;
- implicit shell evaluation;
- simulated command execution in `dry-run`;
- readiness checks for actions or links;
- direct provider selection;
- selection of individual names inside a batch package;
- interactive UI or an `fzf` dependency;
- stable JSON or another structured output protocol.

These omissions define the 0.1.0 product boundary: `dot` supplies a small,
cross-platform execution framework while leaving package-manager knowledge and
machine-specific procedures visible in the user's manifest.

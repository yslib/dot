# dot

`dot` is a small, declarative bootstrap runner for personal development
environments and dotfiles across Windows, macOS, and Linux.

Its job is deliberately narrow: read an explicit TOML manifest, select one
target and profile, and coordinate the external tools needed to establish that
environment. It exists to replace a collection of platform-specific bootstrap
scripts with one understandable workflow.

## Usage (draft)

The command-line interface is still under development. `--dry-run` and
`check providers` are implemented end to end. The implicit apply operation
without `--dry-run` remains a development facade that only prints its
normalized dispatch value.

```text
dot [OPTIONS]
dot [OPTIONS] check providers
```

With no subcommand, `dot` selects the implicit apply operation:

```text
dot
dot --target arch-personal
dot --target arch-personal --profile laptop
dot --target arch-personal --profile laptop --dry-run
```

Provider readiness is an explicit nested check:

```text
dot check providers
dot check providers --target arch-personal --profile laptop
```

This command loads and selects the effective manifest, applies each effective
provider's child-process environment patch, and runs every provider probe once.
It never runs provider ensure or install, package actions, global actions, or
link reconciliation. Every provider is checked even after another provider
fails. The command exits 0 only when all probes are ready; an empty provider
set is ready.

Selection options are global and may appear before or after the subcommands:

```text
-c, --config <PATH>      TOML manifest; defaults to ./dot.toml
-t, --target <TARGET>    optional when the manifest has exactly one target
-p, --profile <PROFILE>  one globally unique profile name within the target
    --dry-run             render the implicit apply plan without executing it
-h, --help
-V, --version
```

`--profile` names one node directly, not a path. The selected node inherits its
unique ancestor chain from the inline profile tree. Profile names must be
unique within a target and cannot contain `/`.

`--dry-run` belongs only to the implicit apply operation and cannot be combined
with `check providers`. Version 1 defines no other check target.

### Development platform override

The development-only `dev-platform-override` Cargo feature adds a global
`--platform <TOML>` option. Its value is a TOML inline table containing a
complete synthetic platform. `os` and `arch` are required; `distro` and
`distro_family` are optional, and `environment` defaults to `native`.

```text
cargo run --features dev-platform-override -- \
  --dry-run \
  --platform '{ os = "windows", arch = "x86_64" }'

cargo run --features dev-platform-override -- \
  check providers \
  --platform '{ os = "linux", arch = "x86_64", distro = "ubuntu", distro_family = "debian" }'
```

Dry-run and `check providers` use the injected value for target compatibility.
The real apply operation accepts but ignores it and always uses detected
platform facts. The override does not emulate another operating system:
provider probes, environment variables, and standard directories still belong
to the host running dot. Every invocation that supplies `--platform` prints
this boundary to stderr; apply prints a separate warning that the option is
ignored.

Without the feature, `--platform` is not part of the CLI and is rejected as an
unknown option.

Dry-run loads and merges the selected manifest, resolves its safe built-in
interpolation, groups provider packages by provider and `provider_args`, and
prints the resulting providers, install batches, manual packages, actions, and
links. Provider activation patches are applied only to in-memory child
environments so their commands can be shown accurately. Dry-run never starts a
process and never inspects or modifies package, action, or link state. Its
human-readable output is explanatory and is not a stable serialized IR.

## Built-in resolvers

Interpolated strings use an OmegaConf-like surface:

```text
${resolver:payload}
```

The resolver set is closed and built into dot. TOML cannot define new
resolvers. Version 1 has no resolver defaults, nested interpolation,
expressions, configuration references, or automatic type conversion.

The scalar resolvers produce one string and may occupy a complete value or be
embedded in a larger string:

| Resolver | Value |
| --- | --- |
| `${env:NAME}` | `NAME` from the current effective child-process environment |
| `${dot:config}` | Absolute path of the loaded TOML file |
| `${dot:config_dir}` | Directory containing the loaded TOML file |
| `${dot:cwd}` | Directory from which dot was invoked |
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

For example:

```toml
target = "${xdg:config}/nvim"
program = "${xdg:executable}/rg"
cwd = "${dot:config_dir}"
```

An `xdg` path follows the host platform's standard directories. A path that is
not defined on the host is an error; dot does not invent a fallback. Likewise,
a missing environment variable is an error rather than an empty string.

The two `package` resolvers produce string lists:

| Resolver | Value |
| --- | --- |
| `${package:names}` | Complete package-name batch for one provider install |
| `${package:provider_args}` | Shared `provider_args` list for that batch |

List resolvers are available only in a provider's `install.args`. Each must
occupy one complete argument so dot can expand it into zero or more argv
elements:

```toml
install = { program = "brew", args = ["install", "${package:provider_args}", "${package:names}"] }
```

Resolver availability is intentionally narrow:

- links, global actions, manual-package actions, and provider `activate`,
  `probe`, and `ensure` may use `env`, `dot`, and `xdg`;
- provider `install` may additionally use `package:names` and
  `package:provider_args`;
- package `provider_args` values are literal and cannot contain resolvers.

To write a literal `${` sequence, escape it as `\${`. In TOML, a literal
single-quoted string can write this directly as `'\${literal}'`; a basic
double-quoted string writes it as `"\\${literal}"`. Resolved values are passed
as data and are never reinterpreted as shell syntax. Shell behavior must still
be requested explicitly through `bash`, `pwsh`, or another interpreter.

## Goal

`dot` should make a personal development environment easy to reproduce without
hiding what will happen.

- Each target describes one concrete environment, such as a personal Arch
  machine, a work Ubuntu machine, or a minimal server.
- Each profile is declared explicitly inside its target. Profiles form a small
  inline inheritance tree, so shared desktop configuration can be inherited by
  a more specific laptop profile.
- Selecting a profile follows exactly one path from the target root. Deeper
  declarations add new records or replace same-named records as a whole.
- Repetition between independent targets is acceptable. A locally complete
  declaration is often clearer than an abstraction that tries to erase real
  platform differences.
- The configuration should remain readable enough that the manifest itself is
  an inventory of the intended environment.

## Small domain model

`dot` understands only the minimum concepts needed for bootstrap work:

- **target**: one explicitly selected environment with a platform constraint;
- **profile**: one node in the target's inline inheritance tree;
- **provider**: an external installation capability such as `pacman`, `brew`,
  `scoop`, `npm`, or `cargo`;
- **package**: one package delegated to one provider, or one explicit manual
  installation action;
- **link**: one native symbolic-link intent for a dotfile;
- **action**: one generic `check`/`exec` operation for work that does not fit a
  smaller domain block.

The configuration language intentionally has little evaluation logic. Profile
merging is limited to a single root-to-child path, and a deeper record replaces
an ancestor record instead of recursively merging its fields. Interpolation is
limited to a small set of built-in runtime values needed by actions and
provider package batches.

Complex installation procedures belong in external shell or PowerShell
scripts invoked by an action. They do not expand the TOML format into a
programming language.

## Non-goals

### dot is not a package manager

`dot` delegates installation to package managers and other external commands.
It does not:

- access or index package repositories;
- search for packages or choose a preferred source;
- resolve package or provider dependencies;
- compare, solve, pin, or manage versions;
- implement downloads, archive extraction, builds, or installers;
- update or uninstall packages;
- maintain an installed-package database or persistent receipts.

Package-manager-specific knowledge stays in the user's provider declarations.
The external provider remains responsible for package semantics and
idempotency.

### dot is not a universal configuration language

The manifest is not intended to become a general-purpose DSL. It has no
arbitrary expressions, user-defined functions, embedded control flow, or
general resolver system. It does not provide conditionals on every item,
multiple inheritance, cross-target references, dependency graphs, or a
template language.

`dot` does not try to infer a perfect environment from the current machine.
The user explicitly chooses the target and profile whose declarations should
apply.

### dot is not a general system orchestrator

`dot` is for bootstrapping a personal development workflow and linking its
configuration files. It is not a service manager, provisioning platform,
configuration-management system, deployment engine, or replacement for shell
scripts when procedural logic is the clearest solution.

The project intentionally prefers a small, predictable model over universal
abstraction. If a feature would make `dot` responsible for understanding how
packages, operating systems, or arbitrary programs work, it is probably
outside the project's scope.

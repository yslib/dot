# dot

`dot` is a small, declarative bootstrap runner for personal development
environments and dotfiles across Windows, macOS, and Linux.

Its job is deliberately narrow: read an explicit TOML manifest, select one
target and profile, and coordinate the external tools needed to establish that
environment. It exists to replace a collection of platform-specific bootstrap
scripts with one understandable workflow.

## Usage (draft)

The command-line interface is currently a development facade. The binary
parses and normalizes these arguments into a dispatch value, then prints it;
configuration loading and execution are not implemented yet.

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

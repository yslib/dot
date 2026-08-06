# Configuration Reference

[SCHEMA.txt](SCHEMA.txt) is the sole authoritative structural schema for dot
configuration. This document is the human-facing explanation of that schema;
[DESIGN.txt](DESIGN.txt) contains the deeper runtime semantics and architectural
boundaries. When they differ, update `SCHEMA.txt` first and then synchronize
this reference.

## Configuration discovery

`dot` chooses one configuration using this exact precedence:

1. the path from an explicit `--config PATH`, when supplied;
2. `.dot.toml` in the current working directory;
3. `~/.config/dot/.dot.toml` on Linux and macOS, or
   `%APPDATA%\dot\.dot.toml` on Windows.

An explicit path is selected as given, may have any filename, and bypasses the
remaining discovery candidates. Among the automatic candidates, the first
whose filesystem entry exists is chosen. Read, parse, or validation failures
for the chosen path are reported immediately; `dot` does not fall back to
another candidate. It does not search parent directories recursively, merge
configuration files, or recognize `dot.toml` as a legacy default.

After selection, `dot` makes the selected path absolute without following
symbolic links. This is the **entry path**: when a symlink was selected, it is
the absolute symlink path. Loading then eagerly calls
`std::fs::canonicalize`, reads the resulting **canonical entity path**, and
retains the entry path's parent as `${dot:config_dir}` and the canonical path's
parent as `${dot:real_config_dir}`. The configuration protocol also retains the
captured invocation directory as `${dot:cwd}`; it retains neither file path. A
dangling or otherwise unresolvable entry therefore fails every command during
loading. Canonical directory spelling is inherited exactly from the host
Rust/OS filesystem API. Read, parse, and validation diagnostics continue to
identify the entry path.

## Type index

- [Foundational types](#foundational-types): [`string`](#string),
  [`identifier`](#identifier),
  [`selector_identifier`](#selector_identifier),
  [`environment_name`](#environment_name),
  [literal-string source](#literal-string-source),
  [string-expression source](#string-expression-source),
  [provider install flat-list expression](#provider-install-flat-list-expression),
  [`OneOrMany<T>`](#oneormanyt), and [keyed tables](#keyed-tables).
- [Structural types](#structural-types): [`Root`](#root), [`Target`](#target),
  [`Profile`](#profile), and [`PlatformConstraint`](#platformconstraint).
- [Package types](#package-types): [`Package`](#package),
  [`ProviderPackage`](#providerpackage),
  [`SingleProviderPackage`](#singleproviderpackage),
  [`BatchProviderPackage`](#batchproviderpackage), and
  [`ManualPackage`](#manualpackage).
- [Execution types](#execution-types): [`Provider`](#provider),
  [`EnvironmentPatch`](#environmentpatch), [`ExecAction`](#execaction),
  [`Action`](#action), [Command Action](#command-action), and
  [Fetch Content Action](#fetch-content-action).
- [Link types](#link-types): [`Link`](#link),
  [`LinkConflict`](#linkconflict), and
  [`LinkMissingParent`](#linkmissingparent).
- [Execution order, selection, and failure behavior](#execution-order-selection-and-failure-behavior),
  [Cross-cutting validation and defaults](#cross-cutting-validation-and-defaults),
  [Interpolation](#interpolation), and [Complete example](#complete-example).

The complete configuration tree is:

```text
Root
└── targets: { selector_identifier -> Target }
    ├── platform: PlatformConstraint
    ├── providers: { identifier -> Provider }
    ├── packages: { selector_identifier -> Package }
    ├── links: { selector_identifier -> Link }
    ├── actions: { selector_identifier -> Action }
    └── profiles: { selector_identifier -> Profile }
        ├── providers: { identifier -> Provider }
        ├── packages: { selector_identifier -> Package }
        ├── links: { selector_identifier -> Link }
        ├── actions: { selector_identifier -> Action }
        └── profiles: { selector_identifier -> Profile } (recursive)
```

A selected profile inherits the target and each profile on its lexical ancestor
path. Each keyed provider, package, link, or action record is atomic: a deeper
record with the same key replaces the complete earlier record. Fields and lists
inside a record are never merged. Unselected branches and replaced records do
not enter the effective manifest. They still undergo complete static
validation when the configuration is loaded.

Except for the single marked complete example at the end, every TOML snippet in
this reference is an explicitly contextual fragment. It illustrates the type
under discussion and is not intended to deserialize as a complete `Root` by
itself.

## Foundational types

### string

Shape: a TOML string, written as a basic string (`"text"`) or a literal string
(`'text'`). The schema assigns a more specific role to every string-bearing
field; that role determines interpolation and validation.

Contextual fragment:

```toml
program = "brew"
```

TOML parsing handles quoting and escapes first. A plain `string` has no
standalone runtime behavior or interpolation promise; use the documented
`identifier`, `environment_name`, literal-string source, string-expression
source, or provider install flat-list rules for the actual field.

### identifier

Shape: a non-empty string used for provider keys and references, package
installation names, and platform facts and constraints.

Contextual fragment:

```toml
provider = "brew"
```

Identifier syntax is validated during TOML deserialization. Identifiers must
not contain `${` anywhere, including `\${`, and do not accept interpolation.
This broad type deliberately permits names that need not round-trip as external
selectors. Providers, package `provider` references, Batch `names`, and
`PlatformConstraint` values use it.

### selector_identifier

Shape: a string matching this exact ASCII grammar:

```text
[A-Za-z0-9_][A-Za-z0-9._-]*
```

Target, profile, package, action, and link table keys use this narrower type.
Those keys are externally selectable identities, so the grammar excludes
whitespace, control characters, `:`, `/`, `\`, and `@`. This makes
`kind:identifier` unambiguous, keeps a selector safe as the first field of
machine-readable listings, reserves `@root`, and allows profile paths to use
`/` only as a display separator. Profile identifiers must also be globally
unique within one target. Like broad identifiers, selector identifiers never
interpolate.

### environment_name

Shape: a non-empty string used as a key in an environment `variables` table.

Contextual fragment:

```toml
variables = { CARGO_HOME = "${xdg:data}/cargo" }
```

Environment names are validated during TOML deserialization. A name cannot
contain `=` or `${` (even when preceded by a backslash), and never accepts
interpolation. The value paired with the name is a string-expression source
and may interpolate.

### Literal-string source

Shape: a TOML string whose consumer requires a validated literal string. This
source role is currently used for package `provider_args` elements.

Contextual fragment:

```toml
provider_args = ["--cask", '--label=\${literal}']
```

Literal strings do not resolve anything. Every source retains its deserialized
TOML string value and a recoverable parsed form. An unescaped `${` in this role
is rejected during whole-configuration static validation. To represent a
literal `${`, the deserialized TOML value must preserve `\${` for the
expression parser; that parser consumes the backslash and produces literal
`${` data. A TOML literal string is a convenient carrier for that value. This
does not retain lexical TOML quoting or escape spelling. Literal strings are
data and are never shell syntax.

### String-expression source

Shape: a TOML string that may be promoted, in context, to exactly one resolved
string. Its recoverable source form is a literal, a string template, an exact
variable, or malformed expression syntax.

Contextual fragment:

```toml
cwd = "${dot:config_dir}/scripts"
```

String expressions accept the string-valued `env`, `dot`, and `xdg` resolvers
listed in [Interpolation](#interpolation). A literal has no variable; a string
template combines literal fragments with one or more string-valued variables;
an exact variable is one resolver call occupying the entire source. A complete
exact variable takes the resolver's declared result type.

These sources reject unknown resolvers, invalid payloads, list-valued package
variables, nesting, defaults, and expressions. An unescaped `${` starts
resolver syntax. When the deserialized TOML value contains `\${`, the
expression parser consumes the backslash and produces literal `${`. Malformed
syntax remains recoverable during deserialization, then fails static promotion
during configuration loading. Static promotion verifies syntax, resolver
existence, payload shape, and result type without evaluating a runtime value.
Only the selected execution closure is resolved later.

TOML string syntax is only the carrier. Strings that look alike at that level
can represent different typed forms:

| TOML string value | Parsed and typed meaning | Result |
| --- | --- | --- |
| `"literal"` | literal string | exactly one string |
| `"prefix-${env:HOME}"` | string template with a string variable | exactly one string |
| `"${env:HOME}"` | exact string variable | exactly one string |
| `"${package:names}"` | exact list variable, valid only in provider install args | zero or more strings in the general list model; current package names contain one or more |

String-valued exact variables may occupy a whole source or participate in a
string template. List-valued variables cannot interpolate into text. These
source and result-type distinctions do not add any TOML syntax.

### Provider install flat-list expression

Shape: the complete provider `install.args` array. Each source element becomes
either one string expression, the exact list variable `${package:names}`, or
the exact list variable `${package:provider_args}`.

Contextual fragment:

```toml
args = ["install", "--root=${xdg:data}", "${package:provider_args}", "${package:names}"]
```

The two package variables must each occupy a complete array element:

| List variable | Expansion cardinality |
| --- | --- |
| `${package:names}` | one or more argv elements |
| `${package:provider_args}` | zero or more argv elements |

For `${package:names}`, the one-or-more result is the Single package key or the
non-empty Batch `names`. Neither list variable can be embedded in text.
Escaping one makes it literal syntax rather than a list expansion. All
string-valued resolvers remain available in one-string parts.

Expansion is ordered and has exactly one flattening layer: each string
expression contributes one argv element, while each exact list variable
contributes its zero-or-more values in place. Nested literal arrays are invalid
configuration values for `install.args`; there is no alternate spread syntax
or recursive-flattening behavior. This expression type is accepted only for
`Provider.install.args`.

### OneOrMany<T>

Shape: either one value of `T` or a TOML array of `T`. The schema uses it for
platform dimensions, environment path entries, and provider `ensure` actions.

Scalar contextual fragment:

```toml
os = "linux"
```

List contextual fragment:

```toml
os = ["linux", "macos"]
```

The scalar and list forms deserialize to distinct `One` and `Many` shapes but
have the same element semantics. Interpolation is determined by `T`: for
example, platform identifiers do not interpolate, while environment path
string-expression sources do.

### Keyed tables

Shape: a TOML table mapping either an `identifier` or
`selector_identifier` key to a typed record, such as
`{ <package_id: selector_identifier>: Package }`.

Contextual fragment:

```toml
[targets.workstation.packages]
ripgrep = { provider = "brew" }
```

Keys cannot interpolate. Provider maps use broad `identifier` keys. Target,
profile, package, link, and action maps use `selector_identifier` keys because
those identities are externally selectable. Each key is unique within its
declaration map, and keyed tables retain TOML declaration order. During profile
inheritance, the same record key at a deeper level replaces the entire ancestor
record, removes its earlier position, and takes the deeper declaration's
position in the effective table. No field-level merge, list merge, deletion, or
tombstone syntax exists. Record-field order and general map key order, including
environment-variable order, are not execution schedules.

## Structural types

### Root

Shape:

```text
{ targets: { selector_identifier -> Target } }
```

| Field | Type | Required | Interpolation |
| --- | --- | --- | --- |
| `targets` | keyed table of `Target` | yes | keys do not interpolate |

Contextual fragment:

```toml
[targets.workstation]
platform = { os = "linux" }
```

`Root` rejects unknown fields. When target selection is omitted, dot selects a
target only when exactly one configured target is compatible with the active
platform facts. Zero compatible targets and multiple compatible targets are
errors. An explicitly selected target still has its compatibility checked for
execution; structural profile and job inspection may name an incompatible
target.

### Target

Shape: one required platform constraint plus optional keyed maps of providers,
packages, links, actions, and recursively nested profiles.

| Field | Type | Required | Interpolation |
| --- | --- | --- | --- |
| `platform` | `PlatformConstraint` | yes | its identifier values do not interpolate |
| `providers` | keyed table of `Provider` | no, defaults empty | keys do not interpolate; fields follow their types |
| `packages` | keyed table of `Package` | no, defaults empty | keys do not interpolate; fields follow their types |
| `links` | keyed table of `Link` | no, defaults empty | keys do not interpolate; paths may interpolate |
| `actions` | keyed table of `Action` | no, defaults empty | keys do not interpolate; action fields may interpolate |
| `profiles` | keyed table of `Profile` | no, defaults empty | keys do not interpolate |

Contextual fragment:

```toml
[targets.linux]
platform = { os = "linux", arch = ["x86_64", "aarch64"] }
```

A target is a complete base declaration and does not inherit from another
target. Selecting it without a profile uses only its root declarations.
Unknown fields are rejected.

### Profile

Shape: optional keyed maps of providers, packages, links, actions, and child
profiles. All five maps default to empty.

| Field | Type | Required | Interpolation |
| --- | --- | --- | --- |
| `providers` | keyed table of `Provider` | no | keys do not interpolate; fields follow their types |
| `packages` | keyed table of `Package` | no | keys do not interpolate; fields follow their types |
| `links` | keyed table of `Link` | no | keys do not interpolate; paths may interpolate |
| `actions` | keyed table of `Action` | no | keys do not interpolate; action fields may interpolate |
| `profiles` | keyed table of `Profile` | no | keys do not interpolate |

Contextual fragment:

```toml
[targets.workstation.profiles.desktop.profiles.laptop.packages]
power-tools = { provider = "system" }
```

Profiles form a lexical tree, not a reference graph. Exactly zero or one node
is selected directly by its globally unique id within a target. A child
inherits its target and ancestors; siblings and descendants outside that path
do not participate. A deeper record with the same key completely replaces its
ancestor record and moves to the deeper declaration's position in the effective
keyed table. Profiles cannot alter the target platform.

### PlatformConstraint

Shape: `os` plus optional platform dimensions, each expressed as
`OneOrMany<identifier>`.

| Field | Type | Required | Interpolation |
| --- | --- | --- | --- |
| `os` | `OneOrMany<identifier>` | yes | no |
| `arch` | `OneOrMany<identifier>` | no | no |
| `distro` | `OneOrMany<identifier>` | no | no |
| `distro_family` | `OneOrMany<identifier>` | no | no |
| `environment` | `OneOrMany<identifier>` | no | no |

Contextual fragment:

```toml
platform = { os = "linux", arch = ["x86_64", "aarch64"], distro_family = ["arch", "debian"], environment = "native" }
```

Different fields combine with AND; multiple values within one field combine
with OR. Missing optional fields impose no constraint. Known examples include
`windows`, `macos`, and `linux` for `os`, and `native`, `wsl`, and `container`
for `environment`. The constraint filters omitted-target inference and target
listings, and is an assertion for execution with an explicitly selected
target. An execution mismatch fails before actions or filesystem mutation.
Structural profile and job inspection can examine an explicitly named
incompatible target. All values are broad identifiers and do not interpolate.

## Package types

### Package

Shape: the untagged union `ProviderPackage | ManualPackage`. TOML structure,
not a `type` discriminator, selects the variant.

Provider-package contextual fragment:

```toml
ripgrep = { provider = "brew" }
```

Manual-package contextual fragment:

```toml
[targets.workstation.packages.tool.install]
exec = { program = "./install-tool" }
```

Each package key is its stable declaration and report id. Provider packages
reference an effective provider; manual packages carry a Command Action.
Package keys are selector identifiers; provider references are broad
identifiers. Neither interpolates. Unknown fields and shapes that match neither
variant are rejected.

### ProviderPackage

Shape: the untagged union `SingleProviderPackage | BatchProviderPackage`.

Single contextual fragment:

```toml
ripgrep = { provider = "brew", provider_args = ["--quiet"] }
```

Batch contextual fragment:

```toml
cli-tools = { provider = "brew", names = ["bat", "fd", "fzf"] }
```

Single and Batch are distinct variants. Runtime never infers the kind from an
optional or empty `names` value. Every declaration is one explicit install
unit and one report item; dot never coalesces separate declarations into one
install invocation or report unit. Provider-backed units are grouped by
provider only to determine execution order. Provider ids and names are
non-interpolated identifiers. `provider_args` elements are non-interpolated
literal-string sources.

### SingleProviderPackage

Shape:

```text
{ provider: identifier, provider_args?: [literal-string source] }
```

| Field | Type | Required | Interpolation |
| --- | --- | --- | --- |
| `provider` | `identifier` | yes | no |
| `provider_args` | list of literal-string sources | no | no |

Contextual fragment:

```toml
[targets.workstation.packages.ripgrep]
provider = "brew"
provider_args = ["--quiet"]
```

A Single has no `names` field. Its surrounding package key (`ripgrep` here) is
both the concrete name sent to the provider and the stable report id. A ready
provider invokes `install` exactly once for this unit; an unavailable provider
blocks the unit without invoking install. Separate Singles are never coalesced
into one install invocation or report unit, even when their provider and
arguments match; sharing a provider groups them only for execution order.

`provider_args` belongs to this unit and preserves order. If it is non-empty,
the referenced provider's `install.args` must contain exactly one complete
`${package:provider_args}` element. Its values are literal and do not
interpolate.

### BatchProviderPackage

Shape:

```text
{
  provider: identifier,
  names: [identifier],
  provider_args?: [literal-string source]
}
```

| Field | Type | Required | Interpolation |
| --- | --- | --- | --- |
| `provider` | `identifier` | yes | no |
| `names` | non-empty list of `identifier` | yes | no |
| `provider_args` | list of literal-string sources | no | no |

Contextual fragment:

```toml
[targets.workstation.packages.cli-tools]
provider = "brew"
names = ["bat", "fd", "fzf"]
provider_args = ["--force"]
```

The surrounding key (`cli-tools`) is the stable logical id used for profile
replacement and reporting; `names` is the complete concrete provider input.
`names` is required, must be non-empty, and must be internally unique. The same
concrete name may appear in a different declaration.

A Batch is one install unit, never an inferred grouping. A ready provider is
invoked once with the whole list; an unavailable provider blocks the unit
without invoking install. The batch has one shared result: dot does not infer
partial success, retry individual names, or create per-name report statuses.
As with a Single, non-empty literal `provider_args` requires exactly one
complete `${package:provider_args}` element in the provider's `install.args`.

### ManualPackage

Shape:

```text
{ install: Command Action }
```

| Field | Type | Required | Interpolation |
| --- | --- | --- | --- |
| `install` | Command Action | yes | its string expressions accept string-valued resolvers |

Contextual fragment:

```toml
[targets.workstation.packages.starship.install]
check = { program = "starship", args = ["--version"] }
exec = { program = "bash", args = ["${dot:config_dir}/scripts/install-starship"] }
```

The package key is a diagnostic/report id. The install action uses the normal
Command Action lifecycle: without `check`, `exec` runs on every apply. Fetch
Content is not a manual-package install form. A manual package has no provider
and no access to package list variables or an implicit provider environment.
Unknown fields are rejected.

## Execution types

### Provider

Shape: required `probe` and `install` actions, with optional `activate` and
one-or-many `ensure` actions.

| Field | Type | Required | Interpolation |
| --- | --- | --- | --- |
| `activate` | `EnvironmentPatch` | no | string-valued resolvers |
| `probe` | `ExecAction` | yes | string-valued resolvers |
| `ensure` | `OneOrMany<ExecAction>` | no | string-valued resolvers |
| `install` | provider-install `ExecAction` | yes | string-valued resolvers plus complete package list variables in `args` |

Contextual fragment:

```toml
[targets.workstation.providers.brew]
activate = { path_prepend = ["/opt/homebrew/bin", "/usr/local/bin"] }
probe = { program = "brew", args = ["--version"] }
ensure = { program = "bash", args = ["${dot:config_dir}/install-brew"] }
install = { program = "brew", args = ["install", "${package:provider_args}", "${package:names}"] }
```

Complete execution includes every effective provider, even one with no
assigned packages. Exact job execution includes only providers required by the
selected provider-backed packages; providers are not themselves selectable.
A failed or unstartable probe may run `ensure`; an ensure list runs in order
and stops on failure. After successful ensure, dot reapplies activate and
probes once more. Provider install then runs once per selected Single or Batch
unit only when the provider is ready. An unavailable provider blocks its units
without invoking install, while unrelated work continues.

Package list variables are invalid in `activate`, `probe`, and `ensure`, and
are valid only as complete `install.args` elements. The provider install's
`program`, `cwd`, and environment values remain string-expression sources;
only its argument sources participate in the flat-list expression. Unknown
fields are rejected.

### EnvironmentPatch

Shape: optional one-or-many path entries and an optional environment-variable
map.

```text
{
  path_prepend?: string-expression source | [string-expression source],
  path_append?: string-expression source | [string-expression source],
  variables?: { environment_name -> string-expression source },
}
```

| Field | Type | Required | Interpolation |
| --- | --- | --- | --- |
| `path_prepend` | one or many string-expression sources | no | string-valued resolvers |
| `path_append` | one or many string-expression sources | no | string-valued resolvers |
| `variables` | `{ environment_name -> string-expression source }` | no, defaults empty | names no; values use string-valued resolvers |

Contextual fragment:

```toml
env = { path_prepend = "${xdg:home}/bin", path_append = ["/opt/tools/bin"], variables = { TOOL_HOME = "${xdg:data}/tool" } }
```

#### Phase model

The implementation models this record as `EnvironmentPatch<S>`. Source
configuration uses `S = string-expression source`; a planned process uses
`S = resolved string`. The parameter distinguishes source and resolved phases
without changing the writable TOML fields above.

The patch affects child processes launched by dot and never persistently edits
the user's shell. Values resolve against the effective environment immediately
before the patch is applied. For a provider operation, ordering is: current dot
process environment, provider `activate`, then that ExecAction's `env`. Action
variables override provider variables; action prepends come before provider
PATH entries, and appended entries are placed at the end. Global and manual
actions have no implicit provider patch.

### ExecAction

Shape:

```text
{
  program: string-expression source,
  args?: [string-expression source],
  cwd?: string-expression source,
  env?: EnvironmentPatch,
}
```

| Field | Type | Required | Interpolation |
| --- | --- | --- | --- |
| `program` | string-expression source | yes | string-valued resolvers |
| `args` | list of string-expression sources; provider install uses provider-install argument sources | no, defaults empty | string-valued resolvers; provider install also accepts complete package list variables |
| `cwd` | string-expression source | no; inherits the dot process cwd | string-valued resolvers |
| `env` | `EnvironmentPatch` | no | string-valued resolvers in values |

Contextual fragment:

```toml
exec = { program = "git", args = ["-C", "${dot:config_dir}", "status"], cwd = "${dot:cwd}" }
```

The legacy `type` field, including its former `exec` value, has been removed
and is rejected as an unknown field. There is no compatibility behavior for
it. Process execution is identified by the surrounding structural record.

Provider `install` has the same writable fields, except each `args` element is
a provider-install argument source. Its complete `args` array becomes the
provider install flat-list expression. This is separate from a package
declaration's `provider_args`, which is literal unit data expanded only by
`${package:provider_args}`.

#### Phase model

The implementation models this record as `ExecAction<S, A>`: `S` is the type
of `program`, `cwd`, and environment values, while `A` is the type of each
source argument.

- An ordinary source action uses
  `ExecAction<string-expression source, string-expression source>`.
- A provider-install source action uses
  `ExecAction<string-expression source, provider-install argument source>`.
- A planned process uses `ExecAction<resolved string, resolved string>`.

After resolution, a planned process has one resolved `program`, zero or more
resolved `args`, an optional resolved `cwd`, and an optional environment patch
of resolved strings. These phase types do not change the writable TOML shape.

When `cwd` is omitted, dot does not explicitly set the child process working
directory. The child therefore inherits the current working directory of the
running dot process, not a working directory from another action or a global
action setting.

dot starts `program` directly with the resolved argv, cwd, and environment.
There is no implicit shell: pipes, redirects, `&&`, command substitution, shell
expansion, quoting, and globbing are not interpreted. Invoke `bash`, `pwsh`, or
another interpreter explicitly when shell behavior is intended. Resolved
values remain typed process data and are never reinterpreted as shell syntax.

### Action

Shape: the strict, structural (untagged) union `Command Action | Fetch Content
Action`.

No discriminator selects the variant. Every object must match exactly one of
the two complete field sets below; unknown fields, incomplete objects, and
ambiguous mixed objects containing both command and Fetch Content fields are
rejected. The legacy `type` field is not accepted. Manual-package `install`
accepts only Command Action, while entries in an `actions` keyed table accept
either variant.

Contextual fragments:

```toml
[targets.workstation.actions.prepare-cache]
exec = { program = "mkdir", args = ["-p", "${xdg:cache}/dot"] }

[targets.workstation.actions.remote-config]
source = "https://example.com/tool/config.toml"
target = "config/tool/config.toml"
on_conflict = "replace"
```

Both forms use the surrounding action key as the `action:ID` selector and
report identity. They run in the same action phase in final effective
declaration order. One action failure does not stop later actions or the
deferred link phase, but it does contribute to the final failed apply status.

### Command Action

Shape:

```text
{
  check?: ExecAction,
  exec: ExecAction
}
```

| Field | Type | Required | Interpolation |
| --- | --- | --- | --- |
| `check` | `ExecAction` | no | string-valued resolvers |
| `exec` | `ExecAction` | yes | string-valued resolvers |

Contextual fragment:

```toml
[targets.workstation.actions.prepare-cache]
check = { program = "test", args = ["-d", "${xdg:cache}/dot"] }
exec = { program = "mkdir", args = ["-p", "${xdg:cache}/dot"] }
```

#### Phase model

The implementation models this record as `CommandAction<S, A>`, containing
optional and required `ExecAction<S, A>` records. Source fields are promoted to
typed string expressions during complete configuration validation. A selected
Command Action is later resolved to strings during planning. These generic
parameters distinguish source and planned phases; users write only the
concrete `check` and `exec` shape above.

Without `check`, `exec` runs on every apply. Check exit code 0 means satisfied
and skips exec; 1 means unsatisfied, so dot runs exec and checks exactly once
more; any other code means check failed. The action fails if the post-exec
check is not 0 even when exec succeeded. Manual-package and global Command
Actions use this same lifecycle, direct-process rules, and string expressions.
They do not accept package list variables.

### Fetch Content Action

Shape:

```text
{
  source: string-expression source,
  target: string-expression source,
  on_conflict?: "error" | "replace",
}
```

| Field | Type | Required | Interpolation |
| --- | --- | --- | --- |
| `source` | semantic resource locator as a string-expression source | yes | string-valued resolvers |
| `target` | semantic resource locator as a string-expression source | yes | string-valued resolvers |
| `on_conflict` | `"error"` or `"replace"` | no, defaults to `"error"` | no |

Contextual fragment:

```toml
[targets.workstation.actions.remote-config]
source = "https://example.com/tool/config.toml"
target = "config/tool/config.toml"
on_conflict = "replace"
```

The locator roles leave room for future explicitly supported pairs, but the
current pair is exactly an explicit HTTPS source with an authority and host to
a native local-disk target. Source URL userinfo is rejected, and a target URL
is not supported.

Runtime resolution is selected-only and occurs during pure planning. An
absolute target stays absolute. A relative target is relative to the directory
containing the selected configuration entry, equivalent to the path context
represented by `${dot:config_dir}`. It is not relative to the canonical entity
directory unless the value explicitly uses `${dot:real_config_dir}`.

Dry-run resolves the source and target, and its human table displays that
resolved pair without network access or target inspection. On apply, every
action eligible to transfer fetches the source fresh. Target handling is:

| Existing final target entry | `error` | `replace` |
| --- | --- | --- |
| missing | create | create |
| regular file | reject | replace |
| symbolic link | reject without following it | replace without following it |
| directory | reject | reject |
| other special entry | reject | reject |

`replace` therefore permits only a regular file or symbolic link. It is not
transactional even without concurrent writers. Transfer and flush into a
same-directory staging file happen first, but commit removes the existing file
or symbolic link before installing the staged file. If that final install
fails, the target can be absent; dot does not restore the original.

Separately, commit-time target reinspection is best effort under a
cooperative-concurrency assumption: another writer must not mutate the target
concurrently. A concurrent mutation can cause failure or affect a competing
entry; there is no cross-platform object-identity or rollback guarantee.

Fetch Content is a one-shot materialization capability, not a download,
artifact, or cache manager. It has no built-in integrity verification,
conditional-request, retry, resume, decompression, or extraction behavior. Use
a Command Action or an external script for richer transfer cases. See
[DESIGN.txt](DESIGN.txt) for the detailed transport, staging, reporting, and
concurrency contract.

## Link types

### Link

Shape: source and target string-expression sources plus two optional policy
enums.

| Field | Type | Required | Interpolation |
| --- | --- | --- | --- |
| `source` | string-expression source | yes | string-valued resolvers |
| `target` | string-expression source | yes | string-valued resolvers |
| `on_conflict` | `LinkConflict` | no, defaults `replace-link` | no |
| `on_missing_parent` | `LinkMissingParent` | no, defaults `create` | no |

Contextual fragment:

```toml
[targets.workstation.links.editor]
source = "${dot:real_config_dir}/home/editor"
target = "${xdg:config}/editor"
on_conflict = "replace-link"
on_missing_parent = "create"
```

This Unix/macOS example deliberately asks for a source located under the
canonical configuration entity's directory. On Windows, use a TOML literal
string and a backslash suffix:

```toml
[targets.windows.links.editor]
source = '${dot:real_config_dir}\home\AppData\Local\nvim'
target = '${xdg:config_local}\nvim'
```

`std::fs::canonicalize` on Windows may return a verbatim path beginning with
`\\?\`, and dot preserves it. Appending `/home/...` textually to that form
does not add Windows path separators; the literal-string example above both
uses backslashes and avoids TOML basic-string escape processing.

An unqualified relative source such as `source = "home/editor"` is resolved
from the selected configuration **entry directory**, not from
`${dot:real_config_dir}`. Symlinked configurations therefore do not silently
change the relative-source base to their repository or entity directory.
Users who want that base must request it explicitly as above. Target must
resolve to an absolute path. Apply requires source to exist as a regular file
or directory and creates a native symbolic link. A matching link is satisfied.
All effective link paths resolve before mutation, and duplicate resolved
targets prevent the link phase from starting. Link ids and policy literals do
not interpolate; source and target accept string-valued resolvers.

### LinkConflict

Accepted literals: `"error"` and `"replace-link"`; default:
`"replace-link"`. Neither value interpolates.

Contextual fragments demonstrating every enum value:

```toml
strict = { source = "home/a", target = "/tmp/a", on_conflict = "error" }
managed = { source = "home/b", target = "/tmp/b", on_conflict = "replace-link" }
```

`error` fails when the target is an incorrect symbolic link. `replace-link`
may replace an incorrect or broken symbolic link. Neither policy ever replaces
a regular file or directory; ordinary filesystem objects always cause failure.

### LinkMissingParent

Accepted literals: `"create"` and `"skip"`; default: `"create"`. Neither
value interpolates.

Contextual fragments demonstrating every enum value:

```toml
created = { source = "home/a", target = "/tmp/a", on_missing_parent = "create" }
optional = { source = "home/b", target = "/tmp/b", on_missing_parent = "skip" }
```

`create` recursively creates a missing target parent. `skip` treats the link as
currently inapplicable and makes no mutation. This policy is independent of
`LinkConflict`.

## Execution order, selection, and failure behavior

The effective manifest is merged in this order: target, lexical profile
ancestors from outermost to innermost, then the selected profile. A same-id
replacement is a whole-record replacement at the deeper declaration's position.
The resulting final effective declaration positions define this serial phase
and subphase order:

1. Providers in final effective provider declaration order. Every selected
   provider completes its readiness lifecycle before any package starts; each
   provider's `ensure` list retains its declared list order.
2. Manual packages in final effective package declaration order.
3. Provider-backed packages, grouped by final effective provider declaration
   order and, within each provider group, by final effective package declaration
   order.
4. Actions in final effective action declaration order.
5. Links in final effective link declaration order.

Manual packages therefore run after the complete provider-readiness phase and
before provider-backed installs. Complete selection includes all effective
providers and jobs. Exact selection filters the same phase sequence to the
requested package, action, and link identities plus the providers required by
selected provider-backed packages. The order of repeated CLI selectors is only
selection input and never schedules work.

Runtime failure isolation remains local wherever later work is safe. A provider
failure blocks only provider-backed packages that require it; unrelated
providers and packages continue. A failed provider install, manual package, or
action does not stop unrelated later work. Before any selected link mutates the
filesystem, the complete selected link phase is preflighted for duplicate
resolved targets. A duplicate blocks that whole link phase; without a duplicate,
normalization, source-preparation, and reconciliation failures remain local to
their links and later nonblocked links continue.

Structural list commands and human-readable table renderers do not guarantee
row order. Their rows may naturally reflect current containers or traversal,
but that presentation is not a scheduling rule or stable ordering interface.

## Cross-cutting validation and defaults

Validation and evaluation have four distinct boundaries:

Configuration entry selection, absolute-path construction, eager
canonicalization, and reading happen before these expression boundaries.
Consequently every command, including dry-run and structural list commands,
requires a resolvable selected configuration entry.

1. **Parsing and deserialization** check TOML structure, field types, required
   fields, broad and selector identifier rules, environment-name rules, and
   fixed literals. All object shapes reject unknown fields. String-bearing
   source roles retain both their deserialized TOML string value and a
   recoverable literal, template, exact-variable, or malformed parsed form.
   They do not retain lexical TOML quoting or escape spelling.
2. **Complete static validation** promotes every source expression in every
   target and profile declaration, including unselected branches and records
   that a deeper merge could replace. Promotion checks expression syntax,
   resolver existence and payload form, result type, and list-variable
   placement without evaluating resolver values. Validation also constructs
   the target root and every root-to-profile effective merge. In each effective
   scope, profile names must be globally unique, provider references must
   resolve, Batch `names` must be non-empty and unique, and a package with
   non-empty `provider_args` requires exactly one complete
   `${package:provider_args}` element in its effective provider install args.
3. **Selection and runtime resolution** choose and merge one target/profile
   scope, validate the complete requested job-selector set, add the required
   provider closure, and evaluate resolver values only for that selected
   closure. A missing environment variable or unavailable `xdg` value in an
   unselected job is therefore not read. Complete job selection resolves all
   effective providers and jobs; exact selection resolves only selected
   package/action/link jobs and providers required by selected
   provider-backed packages.
4. **Execution** probes providers, runs processes, transfers Fetch Content, and
   reconciles links only after planning succeeds. Dry-run stops before this
   boundary: it performs no Fetch Content network request or target inspection
   and does not inspect or canonicalize link sources, although loading has
   already canonicalized the selected configuration entry. Structural list
   commands stop after complete validation and unresolved target/profile
   selection; they do not evaluate runtime resolver values.

Omitted provider, package, link, action, and profile maps deserialize as empty
maps. Omitted ExecAction `args` and EnvironmentPatch `variables` deserialize as
empty collections. Other fields marked optional remain absent and receive any
runtime default documented in their type section.

`check providers` shares parsing, complete static validation, and
target/profile selection, but has a narrower runtime boundary. It resolves,
applies, and checks only each effective provider's `activate` and `probe`
fields. A local interpolation, activation, preparation, execution, or
nonzero-probe result is recorded for that provider, and checking continues
with the remaining providers. The command does not runtime-resolve packages,
actions, links, provider `ensure`, or provider `install` sources.

## Interpolation

Interpolation uses the OmegaConf-like surface `${resolver:payload}` and a
closed, static registry. Configuration cannot add resolvers. A string-valued
variable may occupy a complete string-expression source or appear inside a
string template.

The resolver signatures are:

```text
env:*                   -> string
dot:*                   -> string
xdg:*                   -> string
package:names           -> list<string>
package:provider_args   -> list<string>
```

### String-valued resolver registry

| Resolver form | Resolved value |
| --- | --- |
| `${env:NAME}` | `NAME` from the current effective child environment |
| `${dot:config_dir}` | parent directory of the selected entry path |
| `${dot:real_config_dir}` | string form of the canonical entity path's parent directory |
| `${dot:cwd}` | working directory from which dot was started |
| `${xdg:home}` | current user's home directory |
| `${xdg:config}` | standard user configuration directory |
| `${xdg:config_local}` | local/non-roaming configuration directory |
| `${xdg:data}` | standard user data directory |
| `${xdg:data_local}` | local/non-roaming data directory |
| `${xdg:cache}` | standard user cache directory, when defined |
| `${xdg:state}` | standard user state directory, when defined |
| `${xdg:runtime}` | standard user runtime directory, when defined |
| `${xdg:executable}` | standard per-user executable directory, when defined |
| `${xdg:documents}` | current user's Documents directory, when available |

The `dot` values describe the current invocation. Configuration loading always
computes the lexical entry directory and canonical entity directory before any
resolver evaluation. On Windows, the real directory can contain the verbatim
`\\?\` prefix returned by the host API. All three `dot` path calls share the
same string-valued availability shown below. Like every path-to-string
resolver, `dot` and `xdg` use Rust's
`Path::to_str()` boundary without lossy replacement; resolution fails when a
path is not Unicode-representable. The `xdg` vocabulary follows XDG directories
on Linux and platform-standard equivalents on Windows and macOS. A missing
environment variable or an unavailable platform directory is an error; it
never silently becomes an empty string.

### Provider-package list-valued variables

| Resolver form | Resolved value | Availability |
| --- | --- | --- |
| `${package:names}` | complete concrete-name list for the current Single or Batch unit | one complete `provider.install.args` element only |
| `${package:provider_args}` | complete ordered provider-argument list for the current unit | one complete `provider.install.args` element only |

For a Single, `${package:names}` expands to its surrounding package key. For a
Batch, it expands to its declared non-empty `names` list. An omitted
`provider_args` expands to zero elements. These are exact list-valued variables,
not string interpolation: they cannot be embedded or used in activate, probe,
ensure, manual/global actions, links, or any other field. If a unit declares
non-empty `provider_args`, the provider install args must contain exactly one
`${package:provider_args}` element.

### Availability by string-bearing role

| Role | Schema type | String variables | Package list variables |
| --- | --- | --- | --- |
| target/profile/package/action/link keys | `selector_identifier` | no | no |
| provider keys/refs, package names, platform values | `identifier` | no | no |
| environment map keys | `environment_name` | no | no |
| package `provider_args` | literal-string source | no | no |
| provider `activate` path/variable values | string-expression source | yes | no |
| ordinary ExecAction `program`, `args`, `cwd`, `env` values | string-expression source | yes | no |
| provider `install` `program`, `cwd`, `env` values | string-expression source | yes | no |
| provider `install.args` | provider install flat-list expression | yes | exact package lists as a complete element only |
| Fetch Content Action `source`, `target` | string-expression source | yes | no |
| Link `source`, `target` | string-expression source | yes | no |
| fixed enum literals | fixed literal | no | no |

Identifiers and environment names reject every `${` substring, even escaped.
For literal-string and string-expression source roles, unescaped `${`
introduces resolver syntax. If the deserialized TOML value preserves `\${`,
the expression parser consumes the backslash and produces literal `${`. Fixed
enums accept only their declared literals. Unknown resolvers, unsupported or
missing payloads, nested interpolation, defaults, expressions, missing values,
and a resolver used outside its allowed context are errors.

Environment patches resolve in application order: current dot process
environment, provider activation when applicable, then the individual action's
patch. Later resolution therefore observes successful earlier activation data
without mutating the parent process. Every resolved value retains its field
type: list results become argv elements, while programs, working directories,
environment values, and link paths remain those kinds of data. No result is
reinterpreted by a shell.

## Complete example

Unlike the preceding contextual fragments, the following is one self-contained
configuration. It includes a target and platform, a full provider lifecycle,
Single and Batch install units with provider arguments, a manual package, an
Command Action with check and exec, a Fetch Content Action, a Link with both
policies, and a nested Profile.

<!-- complete-example:start -->
```toml
[targets.workstation]
platform = { os = "linux", arch = ["x86_64", "aarch64"], environment = "native" }

[targets.workstation.providers.brew]
ensure = [
  { program = "bash", args = ["${dot:config_dir}/scripts/install-brew"] },
  { program = "brew", args = ["tap", "example/tools"] },
]

[targets.workstation.providers.brew.activate]
path_prepend = ["/opt/homebrew/bin", "${xdg:home}/.homebrew/bin"]
variables = { HOMEBREW_NO_ANALYTICS = "1" }

[targets.workstation.providers.brew.probe]
program = "brew"
args = ["--version"]

[targets.workstation.providers.brew.install]
program = "brew"
args = ["install", "${package:provider_args}", "${package:names}"]

[targets.workstation.packages.ripgrep]
provider = "brew"
provider_args = ["--quiet"]

[targets.workstation.packages.cli-tools]
provider = "brew"
names = ["bat", "fd", "fzf"]
provider_args = ["--force"]

[targets.workstation.packages.starship.install.check]
program = "starship"
args = ["--version"]

[targets.workstation.packages.starship.install.exec]
program = "bash"
args = ["${dot:config_dir}/scripts/install-starship"]

[targets.workstation.actions.prepare-cache.check]
program = "test"
args = ["-d", "${xdg:cache}/dot"]

[targets.workstation.actions.prepare-cache.exec]
program = "mkdir"
args = ["-p", "${xdg:cache}/dot"]

[targets.workstation.actions.remote-config]
source = "https://example.com/tool/config.toml"
target = "config/tool/config.toml"
on_conflict = "replace"

[targets.workstation.links.shell]
source = "${dot:config_dir}/home/.zshrc"
target = "${xdg:home}/.zshrc"
on_conflict = "replace-link"
on_missing_parent = "create"

[targets.workstation.profiles.work.packages]
work-cli = { provider = "brew" }

[targets.workstation.profiles.work.profiles.container.actions.prepare-container]
exec = { program = "mkdir", args = ["-p", "${xdg:cache}/dot-container"] }
```
<!-- complete-example:end -->

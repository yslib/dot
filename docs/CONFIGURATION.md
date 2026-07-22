# Configuration Reference

[SCHEMA.txt](SCHEMA.txt) is the sole authoritative structural schema for dot
configuration. This document is the human-facing explanation of that schema;
[DESIGN.txt](DESIGN.txt) contains the deeper runtime semantics and architectural
boundaries. When they differ, update `SCHEMA.txt` first and then synchronize
this reference.

## Type index

- [Foundational types](#foundational-types): [`string`](#string),
  [`identifier`](#identifier), [`environment_name`](#environment_name),
  [`literal_string`](#literal_string), [`scalar_template`](#scalar_template),
  [`provider_install_arg`](#provider_install_arg),
  [`OneOrMany<T>`](#oneormanyt), and [keyed tables](#keyed-tables).
- [Structural types](#structural-types): [`Root`](#root), [`Target`](#target),
  [`Profile`](#profile), and [`PlatformConstraint`](#platformconstraint).
- [Package types](#package-types): [`Package`](#package),
  [`ProviderPackage`](#providerpackage),
  [`SingleProviderPackage`](#singleproviderpackage),
  [`BatchProviderPackage`](#batchproviderpackage), and
  [`ManualPackage`](#manualpackage).
- [Execution types](#execution-types): [`Provider`](#provider),
  [`EnvironmentPatch`](#environmentpatch), [`ExecAction<A>`](#execactiona),
  [`ExecActionType`](#execactiontype), and [`Action`](#action).
- [Link types](#link-types): [`Link`](#link),
  [`LinkConflict`](#linkconflict), and
  [`LinkMissingParent`](#linkmissingparent).
- [Cross-cutting validation and defaults](#cross-cutting-validation-and-defaults),
  [Interpolation](#interpolation), and [Complete example](#complete-example).

The complete configuration tree is:

```text
Root
└── targets: { identifier -> Target }
    ├── platform: PlatformConstraint
    ├── providers: { identifier -> Provider }
    ├── packages: { identifier -> Package }
    ├── links: { identifier -> Link }
    ├── actions: { identifier -> Action }
    └── profiles: { identifier -> Profile }
        ├── providers: { identifier -> Provider }
        ├── packages: { identifier -> Package }
        ├── links: { identifier -> Link }
        ├── actions: { identifier -> Action }
        └── profiles: { identifier -> Profile } (recursive)
```

A selected profile inherits the target and each profile on its lexical ancestor
path. Each keyed provider, package, link, or action record is atomic: a deeper
record with the same key replaces the complete earlier record. Fields and lists
inside a record are never merged. Unselected branches and replaced records do
not enter the effective manifest.

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
`identifier`, `environment_name`, `literal_string`, or `scalar_template` rules
for the actual field.

### identifier

Shape: a non-empty string used for table keys, provider references, platform
values, and similar names.

Contextual fragment:

```toml
provider = "brew"
```

Identifier syntax is validated during TOML deserialization. Identifiers must
not contain `${` anywhere, including `\${`, and do not accept interpolation.
Profile names are checked for global uniqueness within a target during manifest
selection. The CLI profile selector rejects `/`; configuration declarations
should avoid `/` so every profile node remains selectable. A slash in a
declaration is not rejected by identifier deserialization.

### environment_name

Shape: a non-empty string used as a key in an environment `variables` table.

Contextual fragment:

```toml
variables = { CARGO_HOME = "${xdg:data}/cargo" }
```

Environment names are validated during TOML deserialization. A name cannot
contain `=` or `${` (even when preceded by a backslash), and never accepts
interpolation. The value paired with the name is a `scalar_template` and may
interpolate.

### literal_string

Shape: a TOML string whose parsed value is treated literally rather than as a
template. It is currently used for package `provider_args` elements.

Contextual fragment:

```toml
provider_args = ["--cask", '--label=\${literal}']
```

Literal strings do not resolve anything. An unescaped `${` is rejected when
the selected effective manifest is planned. `\${` represents literal `${` in
the interpolation syntax; a TOML literal string is convenient when the parsed
value must retain that backslash. Literal strings are data and are never shell
syntax.

### scalar_template

Shape: a TOML string containing literal fragments and zero or more supported
scalar resolver calls. A call may occupy the entire value or be embedded.

Contextual fragment:

```toml
cwd = "${dot:config_dir}/scripts"
```

Scalar templates accept the `env`, `dot`, and `xdg` scalar resolvers listed in
[Interpolation](#interpolation). They reject unknown resolvers, invalid
payloads, list-valued package resolvers, nesting, defaults, and expressions.
An unescaped `${` starts resolver syntax; `\${` represents literal `${` after
TOML parsing. Resolution happens during planning in the field's execution
context, not merely during TOML deserialization.

### provider_install_arg

Shape: one provider-install argument is either a `scalar_template`, the exact
list resolver `${package:names}`, or the exact list resolver
`${package:provider_args}`.

Contextual fragment:

```toml
args = ["install", "--root=${xdg:data}", "${package:provider_args}", "${package:names}"]
```

The two package resolvers must each occupy a complete array element:

| List resolver | Expansion cardinality |
| --- | --- |
| `${package:names}` | one or more argv elements |
| `${package:provider_args}` | zero or more argv elements |

For `${package:names}`, the one-or-more result is the Single package key or the
non-empty Batch `names`. Neither resolver can be embedded in text. Escaping a
package resolver makes it scalar literal syntax rather than a list expansion.
All scalar-template resolvers remain available here. This type is accepted only
for `Provider.install.args`.

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
`scalar_template` values do.

### Keyed tables

Shape: a TOML table mapping an `identifier` key to a typed record, such as
`{ <package_id>: Package }`.

Contextual fragment:

```toml
[targets.workstation.packages]
ripgrep = { provider = "brew" }
```

Keys are identifiers and cannot interpolate. Target keys are unique in the
root table. Provider, package, link, and action keys are unique within their
declaration map. During profile inheritance, the same record key at a deeper
level replaces the entire ancestor record; no field-level merge, deletion, or
tombstone syntax exists.

## Structural types

### Root

Shape:

```text
{ targets: { identifier -> Target } }
```

| Field | Type | Required | Interpolation |
| --- | --- | --- | --- |
| `targets` | keyed table of `Target` | yes | keys do not interpolate |

Contextual fragment:

```toml
[targets.workstation]
platform = { os = "linux" }
```

`Root` rejects unknown fields. A command selects one target by id; `--target`
may be omitted only when the root contains exactly one target. dot never
chooses among multiple targets using platform facts.

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
ancestor record. Profiles cannot alter the target platform.

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
for `environment`. The constraint is an assertion after explicit target
selection, not a target filter. A mismatch fails before actions or filesystem
mutation. All values are identifiers and do not interpolate.

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
reference an effective provider; manual packages carry an `Action`. Package
keys and provider references are identifiers and do not interpolate. Unknown
fields and shapes that match neither variant are rejected.

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
unit and one report item; dot performs no automatic provider grouping.
Provider ids and names are non-interpolated identifiers. `provider_args`
elements are non-interpolated `literal_string` values.

### SingleProviderPackage

Shape:

```text
{ provider: identifier, provider_args?: [literal_string] }
```

| Field | Type | Required | Interpolation |
| --- | --- | --- | --- |
| `provider` | `identifier` | yes | no |
| `provider_args` | list of `literal_string` | no | no |

Contextual fragment:

```toml
[targets.workstation.packages.ripgrep]
provider = "brew"
provider_args = ["--quiet"]
```

A Single has no `names` field. Its surrounding package key (`ripgrep` here) is
both the concrete name sent to the provider and the stable report id. A ready
provider invokes `install` exactly once for this unit; an unavailable provider
blocks the unit without invoking install. Separate Singles are never grouped,
even when their provider and arguments match.

`provider_args` belongs to this unit and preserves order. If it is non-empty,
the referenced provider's `install.args` must contain exactly one complete
`${package:provider_args}` element. Its values are literal and do not
interpolate.

### BatchProviderPackage

Shape:

```text
{ provider: identifier, names: [identifier], provider_args?: [literal_string] }
```

| Field | Type | Required | Interpolation |
| --- | --- | --- | --- |
| `provider` | `identifier` | yes | no |
| `names` | non-empty list of `identifier` | yes | no |
| `provider_args` | list of `literal_string` | no | no |

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
{ install: Action }
```

| Field | Type | Required | Interpolation |
| --- | --- | --- | --- |
| `install` | `Action` | yes | its action templates accept scalar resolvers |

Contextual fragment:

```toml
[targets.workstation.packages.starship.install]
check = { program = "starship", args = ["--version"] }
exec = { program = "bash", args = ["${dot:config_dir}/scripts/install-starship"] }
```

The package key is a diagnostic/report id. The install action uses the normal
Action lifecycle: without `check`, `exec` runs on every apply. A manual package
has no provider and no access to package list resolvers or an implicit provider
environment. Unknown fields are rejected.

## Execution types

### Provider

Shape: required `probe` and `install` actions, with optional `activate` and
one-or-many `ensure` actions.

| Field | Type | Required | Interpolation |
| --- | --- | --- | --- |
| `activate` | `EnvironmentPatch` | no | scalar resolvers |
| `probe` | `ExecAction<scalar_template>` | yes | scalar resolvers |
| `ensure` | `OneOrMany<ExecAction<scalar_template>>` | no | scalar resolvers |
| `install` | `ExecAction<provider_install_arg>` | yes | scalar resolvers plus complete package list elements in `args` |

Contextual fragment:

```toml
[targets.workstation.providers.brew]
activate = { path_prepend = ["/opt/homebrew/bin", "/usr/local/bin"] }
probe = { program = "brew", args = ["--version"] }
ensure = { program = "bash", args = ["${dot:config_dir}/install-brew"] }
install = { program = "brew", args = ["install", "${package:provider_args}", "${package:names}"] }
```

Every effective provider is activated and probed, even with no assigned
packages. A failed or unstartable probe may run `ensure`; an ensure list runs
in order and stops on failure. After successful ensure, dot reapplies activate
and probes once more. Provider install then runs once per declared Single or
Batch unit only when the provider is ready. An unavailable provider blocks its
units without invoking install, while unrelated providers continue.

Package list resolvers are invalid in `activate`, `probe`, and `ensure`, and are
valid only as complete `install.args` elements. Unknown fields are rejected.

### EnvironmentPatch

Shape: optional one-or-many path entries and an optional environment-variable
map.

| Field | Type | Required | Interpolation |
| --- | --- | --- | --- |
| `path_prepend` | `OneOrMany<scalar_template>` | no | scalar resolvers |
| `path_append` | `OneOrMany<scalar_template>` | no | scalar resolvers |
| `variables` | `{ environment_name -> scalar_template }` | no, defaults empty | names no; values scalar resolvers |

Contextual fragment:

```toml
env = { path_prepend = "${xdg:home}/bin", path_append = ["/opt/tools/bin"], variables = { TOOL_HOME = "${xdg:data}/tool" } }
```

The patch affects child processes launched by dot and never persistently edits
the user's shell. Values resolve against the effective environment immediately
before the patch is applied. For a provider operation, ordering is: current dot
process environment, provider `activate`, then that ExecAction's `env`. Action
variables override provider variables; action prepends come before provider
PATH entries, and appended entries are placed at the end. Global and manual
actions have no implicit provider patch.

### ExecAction<A>

Shape:

```text
{
  type?: "exec",
  program: scalar_template,
  args?: [A],
  cwd?: scalar_template,
  env?: EnvironmentPatch,
}
```

| Field | Type | Required | Interpolation |
| --- | --- | --- | --- |
| `type` | `ExecActionType` | no | no |
| `program` | `scalar_template` | yes | scalar resolvers |
| `args` | list of `A` | no, defaults empty | determined by `A` |
| `cwd` | `scalar_template` | no; inherits the dot process cwd | scalar resolvers |
| `env` | `EnvironmentPatch` | no | scalar resolvers in values |

Generic contextual fragment (`A = scalar_template`):

```toml
exec = { type = "exec", program = "git", args = ["-C", "${dot:config_dir}", "status"], cwd = "${dot:cwd}" }
```

Generic ExecAction arguments are scalar templates. Provider `install` is the
specialization `ExecAction<provider_install_arg>`: its `args` additionally
accept the two package list resolvers. This is separate from a package
declaration's `provider_args`, which is literal unit data expanded only by
`${package:provider_args}`.

When `cwd` is omitted, dot does not explicitly set the child process working
directory. The child therefore inherits the current working directory of the
running dot process, not a working directory from another action or a global
action setting.

dot starts `program` directly with the resolved argv, cwd, and environment.
There is no implicit shell: pipes, redirects, `&&`, command substitution, shell
expansion, quoting, and globbing are not interpreted. Invoke `bash`, `pwsh`, or
another interpreter explicitly when shell behavior is intended. Resolved
values remain typed process data and are never reinterpreted as shell syntax.

### ExecActionType

Accepted literal: `"exec"`. The field is optional; omission selects the same
direct-process execution behavior. No other literal is accepted and the value
does not interpolate.

Contextual fragment showing the only enum value:

```toml
type = "exec"
```

The discriminator is reserved for the execution kind. It does not request a
shell and does not change interpolation rules.

### Action

Shape:

```text
{ check?: ExecAction<scalar_template>, exec: ExecAction<scalar_template> }
```

| Field | Type | Required | Interpolation |
| --- | --- | --- | --- |
| `check` | generic `ExecAction` | no | scalar resolvers |
| `exec` | generic `ExecAction` | yes | scalar resolvers |

Contextual fragment:

```toml
[targets.workstation.actions.prepare-cache]
check = { program = "test", args = ["-d", "${xdg:cache}/dot"] }
exec = { program = "mkdir", args = ["-p", "${xdg:cache}/dot"] }
```

Without `check`, `exec` runs on every apply. Check exit code 0 means satisfied
and skips exec; 1 means unsatisfied, so dot runs exec and checks exactly once
more; any other code means check failed. The action fails if the post-exec
check is not 0 even when exec succeeded. Manual-package and global actions use
this same lifecycle, direct-process rules, and scalar interpolation. They do
not accept package list resolvers.

## Link types

### Link

Shape: source and target templates plus two optional policy enums.

| Field | Type | Required | Interpolation |
| --- | --- | --- | --- |
| `source` | `scalar_template` | yes | scalar resolvers |
| `target` | `scalar_template` | yes | scalar resolvers |
| `on_conflict` | `LinkConflict` | no, defaults `replace-link` | no |
| `on_missing_parent` | `LinkMissingParent` | no, defaults `create` | no |

Contextual fragment:

```toml
[targets.workstation.links.editor]
source = "${dot:config_dir}/home/editor"
target = "${xdg:config}/editor"
on_conflict = "replace-link"
on_missing_parent = "create"
```

A relative source is resolved from the loaded configuration directory; target
must resolve to an absolute path. Apply requires source to exist as a regular
file or directory and creates a native symbolic link. A matching link is
satisfied. All effective link paths resolve before mutation, and duplicate
resolved targets prevent the link phase from starting. Link ids and policy
literals do not interpolate; source and target accept scalar resolvers.

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

## Cross-cutting validation and defaults

Validation has three distinct boundaries:

1. **Parsing and deserialization** check TOML structure, type shapes, required
   fields, identifier rules, and environment-name rules. All object shapes
   reject unknown fields. Parsing alone does not validate every resolver
   occurrence.
2. **Selection, merge, and planning** choose one target and optional profile,
   replace keyed records along its ancestor path, then validate semantic rules
   and resolve strings in the resulting effective manifest. Apply and dry-run
   perform this complete effective-manifest validation before execution.
3. **Execution** probes providers, runs processes, and reconciles links only
   after planning succeeds. Dry-run stops before this boundary.

Omitted provider, package, link, action, and profile maps deserialize as empty
maps. Omitted ExecAction `args` and EnvironmentPatch `variables` deserialize as
empty collections. Other fields marked optional remain absent and receive any
runtime default documented in their type section.

`check providers` intentionally has a narrower validation boundary: it selects
and merges the effective manifest but resolves and validates only provider
`activate` and `probe` fields. It does not validate unrelated packages,
actions, links, provider `ensure`, or provider `install` strings. Declarations
outside the selected profile ancestry, and ancestor declarations replaced by a
deeper record, are excluded from interpolation validation for all commands.

## Interpolation

Interpolation uses the OmegaConf-like surface `${resolver:payload}` and a
closed, static registry. Configuration cannot add resolvers. A scalar resolver
may fill a complete `scalar_template` or be embedded within literal text.

### Scalar resolver registry

| Resolver form | Resolved value |
| --- | --- |
| `${env:NAME}` | `NAME` from the current effective child environment |
| `${dot:config}` | absolute path of the loaded TOML file |
| `${dot:config_dir}` | directory containing the loaded TOML file |
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

The `dot` values describe the current invocation. The `xdg` vocabulary follows
XDG directories on Linux and platform-standard equivalents on Windows and
macOS. A missing environment variable or an unavailable platform directory is
an error; it never silently becomes an empty string.

### Provider-package list resolvers

| Resolver form | Resolved value | Availability |
| --- | --- | --- |
| `${package:names}` | complete concrete-name list for the current Single or Batch unit | one complete `provider.install.args` element only |
| `${package:provider_args}` | complete ordered provider-argument list for the current unit | one complete `provider.install.args` element only |

For a Single, `${package:names}` expands to its surrounding package key. For a
Batch, it expands to its declared non-empty `names` list. An omitted
`provider_args` expands to zero elements. List resolvers cannot be embedded or
used in activate, probe, ensure, manual/global actions, links, or any other
field. If a unit declares non-empty `provider_args`, the provider install args
must contain exactly one `${package:provider_args}` element.

### Availability by string-bearing role

| Role | Schema type | Scalar resolvers | Package list resolvers |
| --- | --- | --- | --- |
| table keys/ids/provider refs/platform values | `identifier` | no | no |
| environment map keys | `environment_name` | no | no |
| package `provider_args` | `literal_string` | no | no |
| provider `activate` path/variable values | `scalar_template` | yes | no |
| generic ExecAction `program`, `args`, `cwd`, `env` values | `scalar_template` | yes | no |
| provider `install` `program`, `cwd`, `env` values | `scalar_template` | yes | no |
| provider `install.args` | `provider_install_arg` | yes | package lists as a complete element only |
| Link `source`, `target` | `scalar_template` | yes | no |
| fixed enum literals | fixed literal | no | no |

Identifiers and environment names reject every `${` substring, even escaped.
For `literal_string` and template roles, unescaped `${` introduces resolver
syntax and `\${` represents literal `${` after TOML parsing. Fixed enums accept
only their declared literals. Unknown resolvers, unsupported or missing
payloads, nested interpolation, defaults, expressions, missing values, and a
resolver used outside its allowed context are errors.

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
Action with check and exec, a Link with both policies, and a nested Profile.

<!-- complete-example:start -->
```toml
[targets.workstation]
platform = { os = "linux", arch = ["x86_64", "aarch64"], environment = "native" }

[targets.workstation.providers.brew]
ensure = [
  { program = "bash", args = ["${dot:config_dir}/scripts/install-brew"] },
  { type = "exec", program = "brew", args = ["tap", "example/tools"] },
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
type = "exec"
program = "mkdir"
args = ["-p", "${xdg:cache}/dot"]

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

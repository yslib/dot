# Fetch Content Action Design

## Status

Approved for implementation planning on 2026-08-03.

This document defines a deliberately small built-in Action that transfers
content from one locator to another. The first implementation supports only an
HTTPS source and a local filesystem target. It does not introduce a top-level
download or artifact concept.

## Motivation

Some configuration files should be fetched directly without first cloning the
complete dotfiles repository. Ordinary command Actions can already implement
this by invoking platform-specific tools such as `curl` or PowerShell, but a
large collection of these declarations repeats transport, path, and conflict
handling across targets.

The feature belongs under the existing Action concept because it adds no new
selection, inheritance, dependency, ordering, or lifecycle semantics. A fetch
remains one user-selected Action, runs in the existing Action phase, and
materializes its result completely at its target. It creates no cached or
referencable resource.

## Product boundary

A Fetch Content Action expresses one operation:

```text
fetch content from source locator
    -> materialize it at target locator
```

The supported locator pair in the first implementation is:

```text
HTTPS URL -> local filesystem path
```

`source` and `target` are intentionally named by their direction rather than
by the current protocol. A later release may add another explicit locator pair
when it has the same one-transfer, one-materialized-result semantics. The first
implementation does not need a dynamic protocol registry, plugin system, or
generic transport graph. Planning may represent the one supported pair as a
closed typed variant that can be extended later.

Fetch Content is not a download manager. It does not create a reusable
download object, expose its result to another job, or retain any state after
the Action completes.

## Configuration schema

The top-level Action value becomes a structurally distinguished sum:

```text
#action =
    #command_action
  | #fetch_content_action

#command_action = {
  check?: #exec_action,
  exec: #exec_action,
}

#fetch_content_action = {
  source: #string_expression_source,
  target: #string_expression_source,
  on_conflict?: #fetch_content_conflict,
}

#fetch_content_conflict =
    "error"
  | "replace"

#exec_action = {
  program: #string_expression_source,
  args?: [#string_expression_source],
  cwd?: #string_expression_source,
  env?: #environment_patch<#string_expression_source>,
}
```

Example:

```toml
[targets.workstation.actions.remote-config]
source = "https://example.com/config.toml"
target = "configs/app.toml"
on_conflict = "replace"
```

No `type` discriminator is used. A Command Action requires `exec`; a Fetch
Content Action requires both `source` and `target`. All records continue to
reject unknown fields, so the two shapes cannot be mixed. Missing required
fields and mixed variant fields are configuration errors.

`on_conflict` defaults to `"error"`. It does not interpolate.

`source` and `target` accept the ordinary string-valued `env`, `dot`, and `xdg`
resolvers. They do not accept package list variables.

## Removal of `ExecAction.type`

The existing optional `ExecAction.type = "exec"` field and the
`ExecActionType` schema/Rust type are removed everywhere. The field has no
behavioral effect, and there is no concrete execution variant that justifies
retaining it as a speculative extension point.

Shell execution remains explicit by setting `program` to `bash`, `pwsh`, or
another interpreter. Downloading is not an ExecAction execution mode because
it has different fields and lifecycle semantics. Timeout, retry, I/O, and
elevation policies would be orthogonal process policies rather than variants
of the current `type` field.

This is an intentional breaking schema change. Configurations containing
`type = "exec"` are rejected as having an unknown field. No compatibility
alias or deprecation period is provided.

The existing user-facing Command Action syntax remains otherwise unchanged.
Manual package `install` continues to accept only the Command Action lifecycle;
Fetch Content is added only to the target/profile `actions` maps.

## Validation and planning

Whole-configuration static validation continues to inspect every target,
profile, selected or unselected branch, and replaced record.

For every Fetch Content Action, static validation:

- validates the Action variant shape;
- rejects unknown or mixed fields;
- promotes the `source` and `target` string expressions;
- validates the fixed `on_conflict` literal.

Static validation does not resolve environment or path values, parse the
runtime locator values, inspect the target, or access the network.

After target/profile and exact job selection, planning resolves only the
selected closure. For a selected Fetch Content Action, planning:

1. resolves `source` and `target` against the ordinary Action resolution
   context;
2. requires `source` to be a valid absolute HTTPS URL;
3. interprets `target` as a native local filesystem path;
4. joins a relative local target to the selected configuration entry directory
   (`${dot:config_dir}` semantics);
5. retains an absolute local target unchanged;
6. rejects every unsupported source/target locator pair;
7. produces one owned, resolved Fetch Content Action in the ExecutionPlan.

The relative target base is deliberately the configuration entry directory,
not the canonical entity directory. A configuration selected through a
symbolic link therefore keeps the same entry-relative behavior used by
relative Link sources. A user who needs the repository/entity directory must
write `${dot:real_config_dir}` explicitly.

Planning remains atomic. A selected invalid URL, unsupported protocol pair, or
path-resolution error returns no ExecutionPlan, so apply executes nothing.

Dry-run resolves and displays the selected source and target but does not
inspect filesystem state or access the network. Structural list commands do
not resolve runtime values.

## Execution semantics

Fetch Content Actions run in the existing Action phase and canonical Action ID
order. They remain selectable as `action:ID`. They introduce no new execution
phase, selector kind, provider closure, or dependency edge.

The local target is inspected with non-following filesystem metadata. Conflict
behavior is:

| Existing target | `error` | `replace` |
| --- | --- | --- |
| Missing | fetch and create | fetch and create |
| Regular file | fail before network access | fetch and replace |
| Symbolic link, including one pointing to a directory | fail before network access | fetch and replace the link itself |
| Real directory | fail before network access | fail before network access |
| FIFO, socket, device, or other special entry | fail before network access | fail before network access |

A hard link is observed as a regular file. Replacing it changes only the target
directory entry; other hard-link entries remain attached to the previous file.

The implementation must not follow a target symbolic link when inspecting or
replacing it. `replace` is explicit authorization to replace an existing
regular file or symbolic-link directory entry. It is not authorization to
remove a directory or special filesystem entry.

When the target is eligible for creation or replacement, execution:

1. creates missing target parent directories recursively;
2. performs one logical HTTPS fetch, with no application-level retry;
3. accepts only successful HTTP responses;
4. may follow a bounded redirect chain only while every URL remains HTTPS;
5. writes the response body to transient staging storage associated with the
   local target;
6. commits the staged content to the target after the response completes;
7. reports `CREATED`, `REPLACED`, or `FAILED`.

There is no content comparison and no `SATISFIED` outcome. Every eligible
apply performs network access, even if the fetched bytes equal the current
target bytes.

Staging prevents an interrupted response from being intentionally streamed
into the existing target. It is implementation hygiene, not a promise of
cross-platform atomic replacement, rollback, or recovery. A transfer failure
must not begin the target commit. Staging cleanup is best effort. If the commit
itself fails after target replacement has begun, the Action reports failure
and makes no stronger target-restoration guarantee.

One Fetch Content failure remains local to that Action and does not stop later
unrelated Actions or Links. As with ordinary Actions, users must not encode
dependencies between Fetch Content Actions or rely on one Action producing an
input for another.

## HTTPS behavior

The first implementation supports public HTTPS GET requests only. It provides
no configuration for authentication, headers, cookies, proxy selection,
client certificates, retry, timeout tuning, or redirect policy. Redirect
limits are an internal safety bound, not schema.

Redirects to a non-HTTPS scheme are rejected. Transport failures, an exhausted
redirect bound, and non-success HTTP responses fail the Action.

The implementation may stream the response to staging and need not retain the
complete response in memory. There is no configurable size limit and no claim
that Fetch Content is suitable for large artifacts.

## Reporting and listing

Fetch Content remains an Action report subject.

- `list jobs` emits selector `action:ID`, kind `action`, `VIA` value `fetch`,
  and unresolved `source -> target` detail.
- Dry-run emits one `PLANNED` Action item with resolved source and target.
- Apply emits `CREATED`, `REPLACED`, or `FAILED`.
- Fetch-specific failures retain a structured stage/message in report evidence.
- Overall command success and failure aggregation remains unchanged.

`check providers` does not resolve or execute Fetch Content Actions.

The human-readable report remains non-stable. The existing list-command TSV
contract remains the stable machine-facing catalog interface, with `fetch` as
the new `VIA` value for this Action variant.

## Explicit non-goals

Fetch Content does not support:

- SHA-256 or any other checksum;
- content equality checks or a satisfied-state probe;
- cache directories, cache keys, or cache reuse;
- ETag, Last-Modified, conditional requests, or update detection;
- receipts, persisted state, or resource identity;
- exposing fetched content or a staging path to another job;
- authentication, custom headers, cookies, or secrets;
- configurable retries, mirrors, or resume;
- archive detection, decompression, extraction, or multiple outputs;
- file mode, ownership, executable-bit, or metadata management;
- templating, transformation, merge, or patch behavior;
- directory synchronization or cleanup;
- duplicate-target detection between separate Actions;
- a user-extensible transport registry.

If checksum validation, caching, versioning, resumability, shared artifacts, or
resource lifecycle becomes necessary, that work must be evaluated as a
separate top-level Download/Artifact concept rather than accumulated as fields
on Fetch Content Action. Users who need richer behavior should use an ordinary
Command Action and an external script.

## Implementation boundaries

The implementation should preserve the existing phase separation:

```text
source schema Action variant
    -> complete static expression validation
    -> selected source/target resolution and locator-pair validation
    -> resolved Fetch Content Action in ExecutionPlan
    -> Action-phase execution
    -> typed Action outcome
    -> CommandReport projection
```

The existing Command Action runner remains responsible for
`check -> exec -> post-check`. A Fetch Content executor performs only the
fetch/commit behavior above. The top-level Action dispatcher selects the
appropriate runner from the resolved Action variant.

Production network I/O should sit behind a narrow internal boundary so tests
can inject deterministic responses without external internet access. This is
an internal testing seam, not a public transport plugin API.

## Documentation and migration

The implementation must update the current:

- `docs/SCHEMA.txt` first;
- `docs/DESIGN.txt` runtime semantics and non-goals;
- `docs/CONFIGURATION.md` reference and examples;
- `README.md` feature summary, execution description, and non-goals;
- fixtures and tests that currently use `type = "exec"`.

The archived v0.1.0 documentation under `docs/archive/v0.1.0/` remains
unchanged.

## Test requirements

Implementation planning must cover at least:

1. structural deserialization of Command and Fetch Content Action variants;
2. rejection of mixed fields, incomplete Fetch Content records, unknown
   fields, and every legacy `type = "exec"` occurrence;
3. complete static expression validation in unselected/replaced Fetch Content
   records;
4. selected-only runtime resolution of source and target values;
5. HTTPS source validation and unsupported locator-pair errors;
6. absolute targets, entry-relative targets, and explicit
   `${dot:real_config_dir}` targets;
7. exact `action:ID` selection and unchanged Action phase ordering;
8. dry-run performing no network or target inspection;
9. `error` behavior for every existing target category without network access;
10. `replace` behavior for regular files and symbolic links, including a link
    that points to a directory;
11. rejection of real directories and platform-available special file types;
12. successful creation with missing parent directories;
13. successful replacement, transport failure, redirect rejection, HTTP
    failure, staging failure, and commit failure;
14. failure isolation between Fetch Content, Command Actions, and later Links;
15. list TSV and apply/dry-run report projection for the new Action variant;
16. formatting, Clippy, complete host tests, and the existing cross-platform CI
    matrix.

## Acceptance criteria

The feature is complete only when:

- existing Command Action configurations without `type` behave unchanged;
- legacy explicit `type = "exec"` is rejected;
- Fetch Content uses the existing Action map, selector, profile replacement,
  phase, failure isolation, and reporting pipeline;
- apply performs one fresh fetch for every eligible Fetch Content Action;
- only regular files and symbolic links can be replaced explicitly;
- relative local targets resolve from the configuration entry directory;
- dry-run and list commands perform no network access;
- no cache, checksum, receipt, or resource-reference behavior is introduced;
- schema, runtime design, user documentation, fixtures, and tests agree.

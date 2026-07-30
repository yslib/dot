# Future Direction: Static Data References

## Status

This is a non-normative exploration plan for work after v0.1.0. It records a
product direction and the questions that must be answered before a syntax or
implementation plan is approved.

The current behavior remains defined by:

- [SCHEMA.txt](SCHEMA.txt) for configuration structure and field capabilities;
- [DESIGN.txt](DESIGN.txt) for runtime semantics and product boundaries;
- [CONFIGURATION.md](CONFIGURATION.md) for the user-facing configuration
  reference;
- [JOB_EXECUTION.md](JOB_EXECUTION.md) for selected-plan and execution
  semantics.

Nothing in this document changes the v0.1.0 schema.

## Motivation

Targets are intentionally complete, independent environment declarations.
Profiles provide a deliberately limited inline inheritance tree within one
target. These constraints make the final intent easy to read, but sufficiently
similar targets and profile branches can repeat package lists, provider
records, links, actions, and ordinary scalar values.

A future release may add one root-level data area, tentatively called `data`,
whose values can be referenced from target and profile declarations. The goal
is to remove selected mechanical duplication without introducing
cross-target inheritance or hiding the final effective intent.

This is data reuse, not domain dependency management.

## Product boundary

Any accepted design must preserve these rules:

- dot remains a bootstrap runner, not a package manager.
- A reference copies or substitutes declared configuration data. It does not
  mean that one package, provider, action, link, target, or profile depends on
  another.
- Package-to-provider lookup remains the only existing domain reference needed
  to install provider-backed packages.
- A target remains the unit selected by platform compatibility. Data reuse
  must not create target inheritance, multiple inheritance, or target
  composition.
- The profile model remains one lexical inline tree. Data reuse must not turn
  profiles into a graph.
- References do not create execution edges. Stable serial job phases and order
  remain unchanged.
- Expansion is pure, deterministic, finite, and fully static. It performs no
  process execution, filesystem access, environment lookup, platform lookup,
  network access, or package-provider lookup.
- The expanded result must still satisfy the ordinary strict dot schema.
- Dry-run and apply continue to consume the same selected, resolved
  `ExecutionPlan`; neither command evaluates a different configuration
  language.
- The manifest must remain understandable without mentally executing a
  general-purpose evaluator.

The first version must not add:

- field merge, deep merge, overlays, spreads, append operators, deletion, or
  tombstones;
- conditional references, `when`, comparisons, boolean expressions, loops, or
  functions;
- user-defined resolvers, macros, templates, parameterized records, or
  executable transforms;
- cross-file imports, remote includes, package catalogs, or repository lookup;
- references in table keys or stable target/profile/package/action/link
  identities;
- implicit reference selection based on platform, profile, provider, or
  runtime state;
- domain-aware dependency resolution or cycle ordering.

## Semantic position

The intended pipeline is conceptually:

```text
TOML source
  -> parse source forms and data references
  -> validate and materialize referenced values
  -> strict ordinary Config
  -> complete static validation
  -> target/profile merge
  -> exact job selection and provider closure
  -> runtime string resolver evaluation
  -> ExecutionPlan
```

A data reference is therefore different from `${env:NAME}`,
`${dot:config_dir}`, or `${package:names}`:

- a data reference is a configuration-time typed value;
- existing resolvers read an explicit runtime resolution context;
- a data reference must not be reinterpreted as shell text;
- a structured value cannot be interpolated into a string.

The surface syntax may eventually resemble an exact variable reference, but
syntax similarity must not collapse these two phases in the implementation or
documentation.

## Candidate models to compare

No model is selected yet. The next design session should compare at least
these alternatives against real duplication in `yslib/dotfiles`.

### 1. Exact typed references

Supported schema positions accept either their normal value or one exact
reference to a globally declared value of the same type.

Conceptually:

```text
RefOr<T> = Inline(T) | Ref(data_id)
```

Advantages:

- preserves the destination field's static type;
- makes structured values possible without string coercion;
- exact replacement has one clear meaning;
- reference availability can be added only where evidence justifies it.

Costs:

- each supported schema position must deliberately admit `RefOr<T>`;
- definitions need a way to declare or infer a stable type;
- untagged TOML shapes require careful ambiguity diagnostics.

This is the leading architectural candidate, but its TOML spelling is
deliberately undecided.

### 2. Scalar and homogeneous-list data only

The first increment supports only explicitly typed scalar values and
homogeneous lists. Whole providers, packages, actions, links, and maps remain
inline.

Advantages:

- smallest type system and validation surface;
- exercises exact typed replacement without introducing generic record data;
- cannot become a hidden inheritance system.

Costs:

- may remove too little real duplication to justify the feature;
- repeated structured records remain repeated;
- later record support could require another syntax revision.

### 3. Generic recursive TOML values

`data` stores arbitrary TOML values and references splice them into the source
tree before ordinary deserialization.

Advantages:

- supports scalars, lists, records, and maps uniformly;
- can remove the widest range of duplication.

Risks:

- introduces an effectively untyped `Any` layer before the strong schema;
- definition errors may be reported only at use sites;
- heterogeneous recursive values, deep copying, and error paths add
  interpreter-like complexity;
- it is the easiest path toward templates, merge, imports, and a DSL.

This model should be rejected unless concrete use cases prove that the typed
alternatives cannot meet the goal.

## Cycle prevention

The preferred first-version model is non-recursive:

- root data entries may not reference other root data entries;
- target and profile fields may reference root data entries;
- referenced ordinary values may still contain existing runtime string
  expressions if their destination schema role permits them.

This makes a reference cycle structurally impossible instead of adding a graph
and cycle detector. If data-to-data references are ever proposed later, they
must be justified as a separate feature with an explicit acyclic model and
diagnostic design.

## Questions that must be resolved

Before writing a schema proposal, answer these questions with concrete
manifest examples:

1. Which current repetitions are actually painful enough to remove?
2. Are shared scalars and package-name lists sufficient, or is whole-record
   reuse necessary?
3. Must one data definition have one declared type, or may its type be inferred
   independently at every use site?
4. May data contain existing `${env:...}`, `${dot:...}`, and `${xdg:...}`
   source expressions? If so, which destination role validates them?
5. Is a reference allowed only where one complete value is expected?
6. What TOML form is visually unambiguous beside ordinary inline tables and
   the existing `${resolver:payload}` syntax?
7. Are unused data entries validated, and against which type?
8. How does an error identify both the reference use and the original
   definition?
9. Which maps may be referenced: one record value, a complete keyed map, or
   neither?
10. Does the feature materially improve the complete dotfiles example after
    expansion, or merely shorten it?

## Investigation plan for the next session

1. Record at least three real duplication cases from the current dotfiles
   manifest. Include one scalar/list case and one structured-record case.
2. Write each case in the three candidate models without changing code.
3. Compare readability of source and fully expanded intent. Prefer the model
   whose source remains obvious to a user reading TOML directly.
4. Define the smallest accepted value kinds and explicitly list unsupported
   kinds.
5. Choose the reference spelling only after the type and expansion semantics
   are fixed.
6. Update `SCHEMA.txt` first with the proposed source and materialized types.
7. Update `DESIGN.txt` with the new static phase and reaffirm unchanged target,
   profile, job-selection, and execution semantics.
8. Add failing fixtures for valid references, type mismatches, unknown names,
   forbidden positions, unused definitions, and the chosen cycle-prevention
   rule.
9. Write an implementation plan only after the schema proposal and examples
   have been reviewed.
10. Keep the first implementation behind the smallest complete semantic
    boundary; do not add merge, parameters, imports, or recursive references
    opportunistically.

## Acceptance criteria for a future design

A design is ready for implementation planning only when it can demonstrate:

- configurations without `data` remain behaviorally unchanged;
- every reference has one statically known destination type;
- expansion order cannot affect the result;
- cycles are impossible by construction or rejected before selection;
- unknown and mistyped references fail complete configuration validation;
- target/profile merge semantics remain atomic replacement by keyed record;
- the expanded configuration can be explained without new runtime evaluation;
- dry-run output continues to show the final selected intent, not reference
  machinery;
- no new package-manager, provider, or dependency semantics enter dot;
- the complete real-world example is measurably clearer rather than merely
  shorter.

## Suggested next-session starting prompt

> Read `docs/FUTURE.md`, `docs/SCHEMA.txt`, and the current dotfiles
> `.dot.toml`. Do not implement anything yet. Extract three concrete
> duplication cases, compare the exact-typed-reference, scalar/list-only, and
> generic-TOML models, and propose the smallest schema that preserves dot's
> non-DSL and non-package-manager boundaries.

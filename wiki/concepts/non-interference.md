---
title: Non-Interference
slug: non-interference
summary: The predicate Legion uses to decide whether two operations may run in parallel; the conjunction of region disjointness, field disjointness, and compatible privilege/coherence; what makes implicit parallelism work.
tags: [dependence-analysis, data-model, parallelism, for-program-reasoning]
subsystem: legion
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/07_privileges.md
  - raw/tutorials/08_partitioning.md
  - raw/website-pages/overview.md
github:
  - https://github.com/StanfordLegion/legion/tree/master/runtime/legion
related:
  - wiki/concepts/privilege.md
  - wiki/concepts/coherence-mode.md
  - wiki/concepts/region-requirement.md
  - wiki/concepts/logical-analysis.md
  - wiki/concepts/partition.md
  - wiki/concepts/field-level-non-interference.md
---

## TL;DR
Non-interference is the formal predicate Legion's logical analysis checks for every pair of region requirements: two operations interfere iff their region requirements overlap **AND** their field sets overlap **AND** their privilege/coherence combination conflicts. If any of those three is "no", the two operations are non-interfering and may run in parallel. The confusion: it's not a property an operation *has* — it's a property of a *pair* of operations. The same task is non-interfering with one neighbor and interfering with another.

## Mental model
Non-interference is the AND of three orthogonal axes:
1. **Region**: do the two requirements name regions whose points overlap?
2. **Field**: do the two requirements name overlapping fields?
3. **Privilege/coherence**: are the access modes compatible (RO+RO, REDUCE+REDUCE same op, anything else with EXCLUSIVE → conflict)?

If any axis is independent (disjoint regions, disjoint fields, or compatible access), the operations are non-interfering. The runtime then has *permission* to parallelize them — actual parallelism still requires the mapper to place them on different processors.

## Mechanism & API
The check happens during `logical-analysis.md` (pipeline stage 2):

1. For each pair `(req_A, req_B)` of region requirements across in-flight operations:
   - **Region overlap**: do `req_A.region` and `req_B.region` share any points? Disjoint partitions on the same parent → no overlap. Different region trees → no overlap. Overlapping subregions → overlap.
   - **Field overlap**: `req_A.privilege_fields ∩ req_B.privilege_fields == ∅` → no overlap.
   - **Privilege/coherence**: see table below.
2. If **all three** indicate overlap *and* conflict, the operations interfere; the runtime adds a dependence edge.

**Privilege/coherence compatibility** (under default `EXCLUSIVE` coherence):

| A \ B | RO | RW | WD | REDUCE (op X) | REDUCE (op Y) |
|---|---|---|---|---|---|
| RO | non-interfering | conflict | conflict | conflict | conflict |
| RW | conflict | conflict | conflict | conflict | conflict |
| WD | conflict | conflict | conflict | conflict | conflict |
| REDUCE X | conflict | conflict | conflict | **non-interfering** | conflict |
| REDUCE Y | conflict | conflict | conflict | conflict | non-interfering (same op only) |

Relaxed coherence modes (`ATOMIC`, `SIMULTANEOUS`, `RELAXED` — `coherence-mode.md`) widen the non-interference predicate at the cost of application-side synchronization.

**Three forms of non-interference** (from tutorial 07):
- **Region non-interference**: disjoint regions or different region trees.
- **Field-level non-interference**: same region, disjoint fields. See `field-level-non-interference.md`.
- **Privilege non-interference**: same region, same fields, compatible privileges (RO/RO or same-op REDUCE/REDUCE).

The runtime exploits all three multiplicatively — that's how DAXPY-style codes get full per-field, per-subregion parallelism even when launches are sequential at the source level.

## Invariants
- The non-interference predicate is **symmetric**: A non-interferes with B iff B non-interferes with A.
- The predicate is **sound** at the logical level: any pair logical analysis marks non-interfering is provably safe to reorder. (Sound but imprecise — see `logical-analysis.md`.)
- Two `READ_ONLY` requirements on the same region/fields are **always** non-interfering.
- Two `REDUCE` requirements on the same region/fields are **non-interfering iff** they use the same `ReductionOpID`.
- `WRITE_DISCARD` is no friendlier than `READ_WRITE` for non-interference (both conflict with everything); its advantage is the *elision of init copies*, not concurrency.
- Coherence weaker than `EXCLUSIVE` widens the predicate but transfers ordering responsibility to the application.

## Performance implications
- **Non-interference is what implicit parallelism is built from.** Every parallelism win in a Legion program comes from making the predicate evaluate to "non-interfering" on a pair the application would have wanted to run in parallel.
- **Narrowing privileges + narrowing fields + using disjoint partitions** = three knobs that multiply non-interference. The standard tactic for fixing `pitfalls/false-dependencies-overbroad-privileges.md`.
- Reduction patterns are the **only way to express concurrent commutative-associative updates** to the same data; non-interference under `REDUCE` is the formal reason it works.
- The cost of computing non-interference is per-pair-of-requirements — heavy use of distinct regions and fields increases the cost; aliased partitions can blow it up.

## Debug signals
- **Legion Spy `-d`** dataflow graph: every edge between operations represents a non-interference check that returned "interfering". Edges that shouldn't be there indicate over-broad regions, fields, or privileges.
- **`-DPRIVILEGE_CHECKS`** complements: catches violations of declared privileges (orthogonal to non-interference).
- **Parallel-looking code that serializes** in Legion Prof is almost always a non-interference miss — review the region, field, and privilege axes.

## Failure modes
- Conservative over-declaration (whole region instead of subregion, all fields instead of one, RW instead of RO) → false interference → unnecessary serialization. See `pitfalls/false-dependencies-overbroad-privileges.md`.
- Aliased partition claimed disjoint → application thinks parallel but runtime detects (correct) interference. See `pitfalls/non-disjoint-disjoint-partition.md`.

## Source pointers
- **Implementation tree**: https://github.com/StanfordLegion/legion/tree/master/runtime/legion
- **Tutorial (privileges)**: https://legion.stanford.edu/tutorial/privileges.html
- **Tutorial (partitioning)**: https://legion.stanford.edu/tutorial/partitioning.html
- **Overview**: `raw/website-pages/overview.md`

## Related
- `wiki/concepts/privilege.md` — the access-mode axis.
- `wiki/concepts/coherence-mode.md` — modulates the privilege axis.
- `wiki/concepts/region-requirement.md` — the unit non-interference is computed over.
- `wiki/concepts/logical-analysis.md` — the stage that runs the check.
- `wiki/concepts/partition.md` — the structural way to make regions disjoint.
- `wiki/concepts/field-level-non-interference.md` — the field-axis specialization.

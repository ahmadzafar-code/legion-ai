---
title: Region Requirement
slug: region-requirement
summary: The struct attached to a task launcher naming one (sub)region (or partition + projection), a privilege, a coherence mode, a field set, and the parent region; the operand declaration the runtime uses for dependence analysis.
tags: [data-model, execution, dependence-analysis, for-program-reasoning, for-perf-debug]
subsystem: legion
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/07_privileges.md
  - raw/tutorials/08_partitioning.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
related:
  - wiki/concepts/task-launcher.md
  - wiki/concepts/privilege.md
  - wiki/concepts/coherence-mode.md
  - wiki/concepts/logical-region.md
  - wiki/concepts/partition.md
  - wiki/concepts/dependence-analysis.md
  - wiki/concepts/non-interference.md
  - wiki/concepts/field-level-non-interference.md
  - wiki/concepts/virtual-mapping.md
  - wiki/concepts/write-discard-privilege.md
  - wiki/concepts/read-only-privilege.md
  - wiki/concepts/read-write-privilege.md
  - wiki/concepts/reduce-privilege.md
  - wiki/concepts/projection-functor.md
---

## TL;DR
A `RegionRequirement` is the per-region declaration on a launcher. Each one names a single (sub)region (or a `LogicalPartition` + projection functor ID for index launches), a privilege, a coherence mode, a field set, and a parent region (for subregion containment proofs). The runtime compares these against in-flight requirements during logical analysis to compute non-interference. The confusion: it's a *contract*, not a request — the runtime trusts what's declared. A `READ_WRITE` requirement on a region you only read forces serialization that costs you perf; a region you only write but request `READ_WRITE` forces an init copy. Get them tight.

## Mental model
A region requirement is one operand on a Legion "instruction" (a task launch). Where a CPU instruction encodes `(operand, access mode)` like `mov [eax], 5`, a Legion region requirement encodes `(region, privilege, coherence, fields)`. The whole point of writing them precisely is letting the runtime overlap independent operands — exactly like an OOO CPU schedules around true dependencies.

## Mechanism & API
**Constructors** (most common forms):
```cpp
// Single-region requirement on a logical region:
RegionRequirement r1(lr, READ_ONLY, EXCLUSIVE, parent_lr);

// Projection requirement for IndexLauncher: each point gets its subregion via projection_id
RegionRequirement r2(lp, /*projection_id=*/0, WRITE_DISCARD, EXCLUSIVE, parent_lr);

// Reduction requirement: pass an op ID instead of a privilege:
RegionRequirement r3(lr, /*redop=*/MY_SUM_REDOP, EXCLUSIVE, parent_lr);
```

**Fields**:
- `region` or `partition` — the (sub)region or partition handle.
- `projection_id` (index launches only) — projection functor selecting a per-point subregion.
- `privilege` — `READ_ONLY` / `READ_WRITE` / `REDUCE` / `WRITE_DISCARD` (see `privilege.md`).
- `prop` — coherence (`EXCLUSIVE`/`ATOMIC`/`SIMULTANEOUS`/`RELAXED`, see `coherence-mode.md`).
- `parent` — the parent region the privilege traces back to (used to verify subset rule).
- `privilege_fields` — `std::set<FieldID>` of fields the requirement covers. Add via `add_field(fid)`.
- `instance_fields` — fields the runtime materializes in the physical instance (usually same as `privilege_fields`).
- `redop` — for `REDUCE` privilege, the `ReductionOpID`.
- `tag` — `MappingTagID` per-requirement hint to the mapper.
- `flags` — bit field for special properties (e.g., `NO_ACCESS_FLAG` for virtual mapping).
- `virtual_map` — request virtual mapping for this requirement (no physical instance created).

**Adding to a launcher**:
```cpp
launcher.add_region_requirement(r1);
launcher.add_field(/*req_idx=*/0, FID_X);
launcher.add_field(0, FID_Y);
```

**Inside the task**:
```cpp
const FieldAccessor<READ_ONLY, double, 1> acc_x(regions[0], FID_X);
// regions[i] is the PhysicalRegion for region_requirements[i]
```

## Invariants
- Every requirement's privilege must be a **subset** of the parent task's privilege on the same region (or sub-tree of it).
- A requirement's **field set** restricts what accessors inside the task may touch; access outside it is undefined (debug: `-DPRIVILEGE_CHECKS` catches it).
- A projection requirement on `IndexLauncher` produces a **per-point subregion**; the projection functor must be deterministic and pure (`partition.md`).
- A `WRITE_DISCARD` requirement declares **no read dependence**; if the task reads, undefined behavior.
- A `REDUCE` requirement carries a `ReductionOpID`; two `REDUCE` requirements with the *same* op are non-interfering, but with different ops they conflict.
- `parent` must be an ancestor of `region` in the region tree — the runtime uses it to verify the privilege chain.

## Performance implications
- **The single most consequential field for performance is `privilege_fields`.** A whole-region requirement on a broad field set is the canonical [false-dependence](../pitfalls/false-dependencies-overbroad-privileges.md) bug.
- **Narrow the region** (use a partition + projection) and **narrow the fields** (only what you actually touch). The runtime's non-interference predicate is the AND of region disjointness, field disjointness, and privilege/coherence compatibility — all three multiply.
- `virtual_map = true` skips instance creation entirely, useful on inner tasks that only delegate to subtasks (see `leaf-task.md` and inner-task discussion there).
- `tag` lets the mapper apply per-requirement policy (prefer GPU memory, request a specific layout) without changing the task signature.

## Debug signals
- **Legion Spy `-d`** dataflow graph: each requirement is an edge; spurious edges between supposedly-parallel tasks mean over-broad region or privilege.
- **`-DPRIVILEGE_CHECKS`**: accessor operations check the actual privilege at access time; throws if mismatched.
- **`-DBOUNDS_CHECKS`**: accessor operations check the point lies within the requirement's region; throws if out of bounds.
- **Privilege-subset error** at submit time: requirement's privilege exceeds the parent task's — the runtime rejects.

## Failure modes
- [False dependencies from over-broad privileges](../pitfalls/false-dependencies-overbroad-privileges.md) — the dominant region-requirement bug.
- Forgetting `parent` mismatch with `region` → privilege-subset check fails at submission.

## Source pointers
- **Legion API**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Tutorial (privileges)**: https://legion.stanford.edu/tutorial/privileges.html
- **Tutorial (partitioning + projection)**: https://legion.stanford.edu/tutorial/partitioning.html

## Related
- `wiki/concepts/task-launcher.md` — what carries region requirements.
- `wiki/concepts/privilege.md` — one field of every requirement.
- `wiki/concepts/coherence-mode.md` — the second.
- `wiki/concepts/logical-region.md` — the region named.
- `wiki/concepts/partition.md` — and the partition for projection requirements.
- `wiki/concepts/dependence-analysis.md` — what consumes the requirements.

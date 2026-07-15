---
title: Instance Layout
slug: instance-layout
summary: The shape of a physical instance in memory; declared via LayoutConstraintSet (specialization, ordering, alignment, field-set, memory-kind) and what the mapper picks per task.
tags: [data-model, memory, instances, for-perf-debug, for-program-reasoning]
subsystem: legion
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/website-pages/mapper.md
  - raw/tutorials/realm_04_region_instances.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion_mapping.h
related:
  - wiki/concepts/physical-instance.md
  - wiki/concepts/mapper.md
  - wiki/concepts/map-task.md
  - wiki/concepts/region-instance.md
  - wiki/concepts/reduction-instance.md
  - wiki/applications/miniaero.md
  - wiki/applications/circuit.md
---

## TL;DR
Instance layout is the per-instance shape decision: AOS (array-of-structs) vs. SOA (struct-of-arrays), dimension order (C vs. Fortran style), alignment, which fields are packed together, and which memory kind hosts it. The mapper expresses layout choices via `LayoutConstraintSet` — passed to `find_or_create_physical_instance` in `map-task.md`. The confusion: layout is not a property of the *logical region* — different physical instances of the same logical region can have different layouts at the same time. The runtime issues copies between them when needed.

## Mental model
Instance layout is your `numpy.ndarray.flags['C_CONTIGUOUS']` choice writ large: same logical data, different memory shape, dramatically different kernel performance. SOA wins for vectorized "process one field across all points" loops; AOS wins for irregular "touch many fields per point" loops. The mapper makes the call.

## Mechanism & API
A `LayoutConstraintSet` collects constraints the runtime must satisfy when materializing an instance (per `raw/website-pages/mapper.md`):

```cpp
LayoutConstraintSet constraints;
constraints.add_constraint(SpecializedConstraint(AFFINE_SPECIALIZE));
constraints.add_constraint(FieldConstraint(field_set, /*contig=*/false));
constraints.add_constraint(OrderingConstraint(dims, /*contig=*/false));
constraints.add_constraint(MemoryConstraint(target_memory.kind()));
constraints.add_constraint(AlignmentConstraint(field_id, ALIGN_BOUNDARY, 64));
```

Constraint kinds:
- **`SpecializedConstraint`** — the "kind" of instance. `AFFINE_SPECIALIZE` is the standard affine-addressing instance (per `region-instance.md`).
- **`FieldConstraint`** — which fields the instance holds and (with `contig=true`) whether they're laid out contiguously (AOS) or interleaved with the index axis (SOA).
- **`OrderingConstraint`** — the dimension order. `{DIM_X, DIM_Y, DIM_Z}` is C-style row-major; reverse for Fortran.
- **`MemoryConstraint`** — which memory kind to allocate in (`SYSTEM_MEM`, `GPU_FB_MEM`, `Z_COPY_MEM`, ...).
- **`AlignmentConstraint`** — minimum byte alignment for a given field.

**Calling site** (in `map-task.md`):
```cpp
PhysicalInstance inst; bool created;
runtime->find_or_create_physical_instance(
    ctx, target_memory, constraints,
    std::vector<LogicalRegion>{lr},
    inst, created);
```
Returns an existing instance if a compatible one is valid; otherwise allocates a fresh one matching the constraints.

**AOS vs SOA** (per `region-instance.md`):
- AOS via `block_size = 1` in the underlying Realm `create_instance` call (interleaves fields per point).
- SOA via `block_size = 0` (separates fields into per-field arrays).
- Pick AOS for "touch all fields of one point", SOA for "touch one field across all points" / vectorization.

**Per-task variant constraints** (`task-variant.md`): register variants with `TaskLayoutConstraintSet` so the runtime can pre-create instances of the right shape before invoking the variant.

## Invariants
- Two instances of the same logical region with **different** layouts are both legal; the runtime issues copies between them when needed.
- A layout is **fixed at allocation**; to change layout you destroy and recreate.
- Constraint sets are **conjunctive**: every constraint must be satisfied for an instance to be considered compatible.
- A `find_or_create_physical_instance` call may reuse a compatible instance OR allocate a fresh one — check the `created` out-parameter to know which.
- Mismatched layout between a task's `FieldAccessor` and the allocated instance is silently slow (or UB for radical mismatches) — accessor type and layout must agree.

## Performance implications
- **The single biggest tuning knob inside `map-task.md`.** Wrong layout = slow leaf kernels regardless of how much else you tune.
- **Cross-memory layout mismatch** = automatic DMA on every consumer; visible as `legion-prof.md` channel-row activity.
- Reusing compatible instances (`find_or_create`) avoids allocator churn and `pitfalls/instance-fragmentation.md`.
- Per-variant layout constraints let the mapper pre-allocate the right shape; without them, the runtime may pick a default that needs reshaping later.
- **AOS instances are friendlier to AMD/CUDA `cudaMallocPitch`-style kernels**; SOA is friendlier to `cuBLAS`/vectorized kernels.

## Debug signals
- **`LoggingWrapper`** logs the `LayoutConstraintSet` for each `map_task` call.
- **Slow leaf kernels despite expected hardware** → check the layout matches the kernel's access pattern.
- **Heavy DMA between two memories** → instances exist in both with different layouts; consolidate.
- **`pitfalls/instance-fragmentation.md`** → many short-lived instances with varying constraints; stabilize them.

## Failure modes
- Layout mismatch with accessor type → silent wrong output or UB.
- Inconsistent constraints between `map_task` calls → no instance reuse → fragmentation.

## Source pointers
- **Mapper API**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion_mapping.h
- **Reference**: `raw/website-pages/mapper.md`
- **Realm layouts**: `raw/tutorials/realm_04_region_instances.md`

## Related
- `wiki/concepts/physical-instance.md` — what the layout governs.
- `wiki/concepts/mapper.md` — where layout is decided.
- `wiki/concepts/map-task.md` — the callback that calls `find_or_create_physical_instance` with constraints.
- `wiki/concepts/region-instance.md` — the Realm primitive layout sits on top of.
- `wiki/concepts/reduction-instance.md` — a variant layout for `reduce-privilege.md` tasks.

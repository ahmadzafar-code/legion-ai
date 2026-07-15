---
title: Reduction Instance
slug: reduction-instance
summary: A physical instance specialized for REDUCE-privilege accesses; stores per-replica partial accumulators initialized to the operator's identity, folded into the destination when a non-REDUCE consumer reads.
tags: [data-model, instances, memory, parallelism, for-perf-debug]
subsystem: legion
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/07_privileges.md
  - raw/tutorials/realm_08_reductions.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion_mapping.h
related:
  - wiki/concepts/physical-instance.md
  - wiki/concepts/reduce-privilege.md
  - wiki/concepts/instance-layout.md
  - wiki/concepts/region-instance.md
  - wiki/concepts/dma-system.md
  - wiki/concepts/realm-barrier.md
---

## TL;DR
A reduction instance is a physical instance dedicated to `REDUCE`-privilege accesses. It's allocated with the cells initialized to the registered operator's **identity** (zero for sum, +∞ for min, etc.); each contributor folds its values in; when a downstream consumer takes a non-`REDUCE` privilege (`READ_ONLY`/`READ_WRITE`), the runtime folds the reduction instance into the destination using the operator's `apply`/`fold` methods. The confusion: reduction instances are *separate buffers* from the "real" data — multiple concurrent same-op reducers each get their own (or share one with atomics), and they're combined at the boundary, not in place.

## Mental model
Reduction instances are OpenMP private accumulators promoted to a first-class runtime concept. Each parallel contributor sums into its own copy; the runtime tree-folds them into the result when the next consumer needs the actual data. No locks, no contention, no serialization on the hot path.

## Mechanism & API
The mapper requests a reduction instance via the `SpecializedConstraint` (per `instance-layout.md`):

```cpp
LayoutConstraintSet constraints;
constraints.add_constraint(SpecializedConstraint(REDUCTION_FOLD_SPECIALIZE,
                                                 /*redop=*/REDOP_SUM));
constraints.add_constraint(FieldConstraint({FID_X}, /*contig=*/false));
constraints.add_constraint(MemoryConstraint(target_memory.kind()));

PhysicalInstance inst; bool created;
runtime->find_or_create_physical_instance(ctx, target_memory, constraints,
                                          regions, inst, created);
```

The `REDUCTION_FOLD_SPECIALIZE` is one of two reduction-instance specializations:
- **`REDUCTION_FOLD_SPECIALIZE`** — holds `RHS`-typed cells; folded with the operator's `fold` method at the boundary.
- **`REDUCTION_LIST_SPECIALIZE`** — a list of (point, value) pairs; useful for sparse reductions.

The runtime materializes the instance with cells set to `Reduction::identity`; from there, `REDUCE`-privilege tasks issue atomic-or-exclusive `apply`/`fold` operations on the cells.

**Folding at the boundary** (per `realm_08_reductions.md`):
- When a non-`REDUCE` consumer takes the data, the runtime issues a copy that runs the operator's `apply`/`fold` across the points, merging the reduction instance into the destination's layout.
- The copy is just another `dma-system.md` operation, visible on `legion-prof.md` channel rows.

**Per-shard accumulators**:
- Under control replication, each shard typically gets its own reduction instance for the local subset of points.
- The boundary fold consolidates across shards via the standard collective machinery.

## Invariants
- A reduction instance is **tied to one `ReductionOpID`**. Using it with a different operator is undefined.
- Cells are initialized to the operator's **`identity`** at allocation.
- The runtime is free to allocate **multiple reduction instances** for the same logical region/field — one per shard, processor, or contributor as needed.
- The fold at the boundary uses `fold(rhs, rhs)` to merge two reduction instances, then `apply(lhs, rhs)` to apply the merged result to the destination's `LHS`-typed buffer.
- A reduction instance **cannot be read directly** as data — only as the input to the folding pass.
- The instance's lifetime ends after the fold; the runtime collects it once the destination is up-to-date.

## Performance implications
- **The primary mechanism for concurrent commutative-associative updates.** Far cheaper than `READ_WRITE` + `Reservation`-style locking.
- The cost is the **boundary fold** — a copy that traverses every point. Acceptable when the reduction phase has many writers; not free.
- Reduction instances live in their own memory-row slabs in `legion-prof.md`; heavy presence indicates active reductions in progress.
- The mapper should place reduction instances in **fast, close-to-compute memory** (system or GPU framebuffer near the contributors).

## Debug signals
- **Legion Prof memory rows** show reduction instances as distinct slabs; channel-row activity at the boundary shows the fold copy.
- **Wrong reduction results** → suspect operator non-associativity, `apply`/`fold` bugs, or mixed operators on the same region. Test on a single processor first.
- **Excessive memory** under heavy reduction loads → many concurrent reduction instances; consider coarsening contributors or sharing instances.

## Failure modes
- Mixing operators on the same data → undefined behavior (the runtime tries to use one identity, one apply, gets inconsistent state).
- Reading a reduction instance directly via `READ_ONLY`/`READ_WRITE` accessor → UB; the instance isn't laid out as plain data.
- Non-associative operator → non-deterministic results across runs.

## Source pointers
- **Tutorial (Legion privileges + REDUCE)**: https://legion.stanford.edu/tutorial/privileges.html
- **Tutorial (Realm reductions)**: `raw/tutorials/realm_08_reductions.md`
- **Mapper API**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion_mapping.h

## Related
- `wiki/concepts/physical-instance.md` — host concept.
- `wiki/concepts/reduce-privilege.md` — what's stored here.
- `wiki/concepts/instance-layout.md` — `REDUCTION_FOLD_SPECIALIZE` constraint.
- `wiki/concepts/region-instance.md` — Realm primitive underneath.
- `wiki/concepts/dma-system.md` — issues the boundary fold copy.

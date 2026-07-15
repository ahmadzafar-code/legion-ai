---
title: Aliased Partition
slug: aliased-partition
summary: A partition whose subregions overlap; required for halo/ghost-cell patterns where neighboring point tasks must read each other's boundary data.
tags: [data-model, partitioning, parallelism, for-program-reasoning, for-perf-debug]
subsystem: legion
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/08_partitioning.md
  - raw/tutorials/09_multiple_partitions.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
related:
  - wiki/concepts/partition.md
  - wiki/concepts/disjoint-partition.md
  - wiki/concepts/ghost-region.md
  - wiki/concepts/non-interference.md
  - wiki/concepts/projection-functor.md
---

## TL;DR
An aliased partition is a partition whose subregions **can share points** — adjacent chunks overlap at their boundaries. The standard application is **ghost-cell halo patterns** in stencil codes: each chunk reads its own interior plus a slice of its neighbors'. The application declares `disjoint=false` (or uses a constructor that produces aliased output) and the runtime correctly serializes accesses to the overlapping regions. The confusion: aliased partitions are not bugs — they're the right tool for halo data. The bug is *aliased data declared as disjoint* (see `pitfalls/non-disjoint-disjoint-partition.md`).

## Mental model
Aliased partitioning is `arr[i-1:i+2]` per chunk — each worker reads its own slice plus a few neighbors' values. The "alias" is the deliberate overlap that enables stencil access patterns. Where `disjoint-partition.md` is for independent chunks, aliased is for cooperative chunks that exchange data at shared boundaries.

## Mechanism & API
**Constructed aliased**:
```cpp
IndexPartition ip = runtime->create_partition_by_field(
    ctx, lr, parent, field_id, color_space,
    /*disjoint=*/false);  // application asserts overlap is allowed
```

**Through `partition_by_image`/`preimage`** with overlapping source/target ranges:
```cpp
// E.g., a halo partition where each chunk reaches into neighbors
IndexPartition halo_ip = runtime->create_partition_by_image(
    ctx, parent_is, partition, lr, field_id, color_space,
    /*disjoint=*/false);  // overlap intentional
```

**Tutorial 9 pattern** (`raw/tutorials/09_multiple_partitions.md`) — multiple aliased + disjoint partitions of the same region used together:
- A `disjoint-partition.md` for *writing* updates (no chunks overlap → no write conflicts).
- An `aliased-partition.md` for *reading* halos (chunks overlap → reads neighbor data).
- A custom `projection-functor.md` ties index-launch points to the right halo subregion.

In stencil codes the disjoint partition is the "owner" mapping (each cell has one owner); the aliased partition is the "reader" mapping (each chunk's reader needs neighbors' data).

## Invariants
- An aliased partition's subregions are **non-disjoint by design** — the application has declared this explicitly.
- The runtime serializes accesses to the overlapping points (per `non-interference.md`: overlapping regions interfere).
- An aliased partition can still be **complete** (every parent point belongs to at least one subregion) — alias-vs-disjoint is orthogonal to complete-vs-incomplete.
- The runtime does **not** silently optimize aliased to disjoint or vice versa; the application's declaration is taken as ground truth.
- Aliased partitions can be **lifted from disjoint via image/preimage**: `partition_by_image` of a many-to-one source field produces an aliased target partition automatically.

## Performance implications
- **The cost of an aliased partition is serialization at the boundaries.** Sibling point tasks that touch only their own (disjoint) interior parallelize; those that touch overlapping halos serialize on the overlapping region.
- For stencils, the standard pattern is: aliased partition for the read-only halo + disjoint partition for the read-write interior. Reads parallelize because two `READ_ONLY` requirements on aliased regions are still non-interfering at the privilege level (`field-level-non-interference.md` adds field axis).
- Aliased partitions of large regions force more equivalence-set fragmentation than disjoint ones — visible as more utility-row activity during physical analysis.

## Debug signals
- **`legion-spy.md` `dataflow-graph.md`** between sibling point tasks: aliased partitions produce edges where disjoint partitions wouldn't.
- **Legion Prof** point-task serialization on what looks like data-parallel work → check whether the partition is aliased; if so, evaluate whether disjoint + halo would be faster.
- **`-lg:partcheck`** (`partition-checks.md`): does **not** flag aliased partitions; it only catches *misdeclared* disjoint partitions.

## Failure modes
- Declaring an aliased partition as `disjoint=true` → [non-disjoint disjoint partition](../pitfalls/non-disjoint-disjoint-partition.md) silent race.
- Using a fully aliased partition where a disjoint + halo pattern would work → unnecessary serialization.

## Source pointers
- **Legion API**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Tutorial (multiple partitions, halo)**: https://legion.stanford.edu/tutorial/multiple.html
- **Paper (dependent partitioning)**: `raw/publications/pdfs/dpl2016.pdf`

## Related
- `wiki/concepts/partition.md` — umbrella.
- `wiki/concepts/disjoint-partition.md` — the dual.
- `wiki/concepts/ghost-region.md` — the canonical use case.
- `wiki/concepts/non-interference.md` — why aliased regions serialize.
- `wiki/concepts/projection-functor.md` — how index-launch points address aliased subregions.

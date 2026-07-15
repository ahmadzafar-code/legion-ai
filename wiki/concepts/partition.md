---
title: Partition
slug: partition
summary: A coloring of an index space into named subspaces, which lifts to subregions of every logical region built on that index space; the bridge from "data" to "parallel work".
tags: [data-model, partitioning, parallelism, for-program-reasoning, for-perf-debug]
subsystem: legion
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/08_partitioning.md
  - raw/website-pages/debugging.md
  - raw/publications/publications.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/region_tree.h
related:
  - wiki/concepts/logical-region.md
  - wiki/concepts/privilege.md
  - wiki/concepts/task.md
  - wiki/concepts/mapper.md
  - wiki/concepts/index-space-launch.md
  - wiki/concepts/region-tree.md
  - wiki/concepts/region-requirement.md
  - wiki/concepts/non-interference.md
  - wiki/concepts/partition-checks.md
  - wiki/concepts/projection-functor.md
  - wiki/concepts/disjoint-partition.md
  - wiki/concepts/aliased-partition.md
  - wiki/concepts/dependent-partitioning.md
  - wiki/concepts/ghost-region.md
  - wiki/concepts/coloring.md
  - wiki/concepts/subregion.md
---

## TL;DR
A partition assigns colors to points of an `IndexSpace`, producing an `IndexPartition`. That lifts to a `LogicalPartition` of every `LogicalRegion` built on the parent index space, giving you subregions you can hand to point tasks in an `IndexLauncher`. Partitions may be **disjoint** (subregions don't overlap, max parallelism) or **aliased** (they do, e.g. ghost cells). The confusion: a "disjoint" partition is disjoint *because you said so* — the runtime trusts you unless `-lg:partcheck` is on.

## Mental model
Partitioning is to a Legion program what `domain decomposition` is to MPI — but lazy, named, and queryable. You don't physically split the data; you just label which points belong to which "chunk", and the runtime materializes per-chunk instances on demand. The bridge from `partition.md` to `parallel work` is the **projection functor**: an `IndexLauncher` over color space `i` says "for each color `i`, give point task `i` the subregion at color `i` via this projection". Projection ID 0 is the identity.

## Mechanism & API
The constructor family (under `Runtime`):
- `create_equal_partition(ctx, is, color_is)` — roughly equal-sized disjoint partition.
- `create_partition_by_field(ctx, lr, parent, fid, color_space)` — color from a field value.
- `create_partition_by_restriction(ctx, parent_is, color_space, transform, extent)` — affine block partition.
- `create_partition_by_image(ctx, ...)` / `create_partition_by_preimage(ctx, ...)` — **dependent partitioning** (paper `dpl2016.pdf`): partition one space based on the values of pointer fields in another.

Use them:
```cpp
IndexPartition ip = runtime->create_equal_partition(ctx, is, color_is);
LogicalPartition lp = runtime->get_logical_partition(ctx, lr, ip);
// ...
IndexLauncher l(TASK_ID, color_is, TaskArgument(), arg_map);
l.add_region_requirement(RegionRequirement(lp, /*proj=*/0, RW, EXCLUSIVE, lr));
```

Each `LogicalPartition` is parameterized by the parent region and an `IndexPartition`. Subregions are reached by `runtime->get_logical_subregion_by_color(ctx, lp, color)`.

## Invariants
- A partition is a property of an `IndexSpace`; it automatically lifts to every `LogicalRegion` on that index space — that's why partitioning the index space partitions all derived regions in lockstep.
- **Disjointness** and **completeness** are properties the application declares. The runtime trusts them unless `-lg:partcheck` is set. A non-disjoint "disjoint" partition silently causes data races.
- Subregions are first-class regions: they have their own region trees and can themselves be partitioned (hierarchical partitioning, paper `oopsla2013.pdf`).
- Partitioning is **lazy**: region-tree nodes for subregions are not materialized until the runtime needs them. Creating a partition of 1M colors is cheap.
- Projection functors are pure functions; ID 0 maps point `i` to subregion `i` (identity).

## Performance implications
- **Disjoint partitions on disjoint subregions with disjoint privileges = maximum point-task parallelism.** That's the whole game.
- Dependent partitioning (`by_image`, `by_preimage`) is the standard way to express stencils and unstructured-mesh halos without writing manual neighbor tables.
- Too few colors → not enough parallelism. Too many → runtime overhead and instance fragmentation. Profile with `legion-prof.md`.
- Aliased partitions force the runtime to serialize accesses to overlapping subregions — necessary for ghost-cell patterns but expensive when accidental.

## Debug signals
- **`-lg:partcheck`** — verifies declared disjointness at partition-creation time. Run with this on whenever you change partitioning code.
- **Legion Spy** dataflow graph: aliased partition → extra dependence edges between point tasks of the same `IndexLauncher`.
- **Legion Prof**: point tasks of an index launch should appear roughly concurrently on their processor rows. If they serialize, the partition is aliased or the privilege is wrong.

## Failure modes
- [Non-disjoint "disjoint" partition](../pitfalls/non-disjoint-disjoint-partition.md) — silent data races; use `-lg:partcheck`.
- [Long dependence chains](../pitfalls/long-dependence-chains.md) — index launch with aliased partition serializes.

## Source pointers
- **Header**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Region tree**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/region_tree.h
- **Tutorial**: https://legion.stanford.edu/tutorial/partitioning.html
- **Paper (dependent partitioning)**: `raw/publications/pdfs/dpl2016.pdf`
- **Paper (hierarchical)**: `raw/publications/pdfs/oopsla2013.pdf`

## Related
- `wiki/concepts/logical-region.md` — what gets partitioned.
- `wiki/concepts/privilege.md` — disjoint subregions × disjoint privileges = parallelism.
- `wiki/concepts/task.md` — `IndexLauncher` is how point tasks consume a partition.
- `wiki/concepts/mapper.md` — distributes point tasks across processors.

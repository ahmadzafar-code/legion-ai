---
title: Disjoint Partition
slug: disjoint-partition
summary: A partition whose subregions share no points; the kind that enables maximum non-interference between point tasks of an index launch.
tags: [data-model, partitioning, parallelism, for-program-reasoning, for-perf-debug]
subsystem: legion
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/08_partitioning.md
  - raw/website-pages/debugging.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
related:
  - wiki/concepts/partition.md
  - wiki/concepts/aliased-partition.md
  - wiki/concepts/non-interference.md
  - wiki/concepts/partition-checks.md
  - wiki/concepts/index-space-launch.md
---

## TL;DR
A disjoint partition splits an `IndexSpace` into subregions that share **no points** — every point belongs to exactly one subregion. Disjoint partitions are the structural prerequisite for `non-interference.md` between sibling subregions: an `index-space-launch.md` whose point tasks each touch their own subregion can be fully parallelized. Built-in shortcuts: `create_equal_partition`, `create_partition_by_restriction` (affine block), and `create_partition_by_field`/`by_image`/`by_preimage` (when the application data makes them disjoint). The confusion: the runtime trusts the application's `disjoint=true` flag — without `-lg:partcheck` (`partition-checks.md`), a misdeclared disjoint partition silently produces data races.

## Mental model
Disjoint partitioning is `chunk(arr, n)` in NumPy/MPI — slice the data once, no chunk overlaps another. Each chunk gets its own worker, the workers run in parallel without coordination because they touch disjoint state. Legion's contribution is making the disjointness explicit, runtime-checkable, and composable with field-level non-interference.

## Mechanism & API
**Disjoint by construction** (the runtime can prove disjointness):
```cpp
IndexPartition ip = runtime->create_equal_partition(ctx, is, color_is);
// or:
IndexPartition ip = runtime->create_partition_by_restriction(
    ctx, parent_is, color_space, transform, extent);
```

`create_equal_partition` splits the parent into N roughly-equal-sized chunks. `create_partition_by_restriction` builds an affine block partition; both produce disjoint partitions automatically.

**Disjoint by declaration** (application asserts it):
```cpp
IndexPartition ip = runtime->create_partition_by_field(
    ctx, lr, parent, field_id, color_space, /*disjoint=*/true);
```
Here the runtime trusts the application — the coloring derives from a field whose values must be unique across points. If they're not, you have a hidden data race.

**Use them** in an `index-space-launch.md`:
```cpp
LogicalPartition lp = runtime->get_logical_partition(ctx, lr, ip);
IndexLauncher launcher(TASK_ID, color_is, ...);
launcher.add_region_requirement(
    RegionRequirement(lp, /*proj=*/0, READ_WRITE, EXCLUSIVE, lr));
```

With the identity projection (`projection-functor.md` ID 0), each point task `i` gets subregion `lp[i]`; because they're disjoint, the tasks are non-interfering (assuming compatible privileges) and parallelize fully.

## Invariants
- The runtime **trusts** `disjoint=true`. Without `-lg:partcheck`, misdeclaration is undetected → silent data races.
- Disjoint partitions enable `non-interference.md` between sibling subregions; aliased partitions do not.
- Disjointness is a property of the *underlying `IndexPartition`* — it lifts to all `LogicalPartition`s derived from it.
- `create_equal_partition` and `create_partition_by_restriction` produce disjoint partitions by construction (the runtime knows mathematically, no flag needed).
- A disjoint partition may be **incomplete** (some parent-space points belong to no subregion) — disjointness ≠ completeness.

## Performance implications
- **The structural prerequisite for parallel point tasks.** Without disjoint partitioning, point tasks of an `IndexLauncher` over the partition serialize.
- Combined with **field-level non-interference** (`field-level-non-interference.md`), disjoint × disjoint-fields × compatible-privileges = full parallelism.
- The runtime materializes per-subregion physical instances on demand; disjoint partitions allow them to be independent in memory.
- For `partition-by-image`/`preimage` over large data, the partition-creation pass itself can be expensive; profile if it shows up in `legion-prof.md` utility rows.

## Debug signals
- **Run with `-lg:partcheck`** — see `partition-checks.md`. Catches declared-disjoint-but-actually-aliased partitions at creation time.
- **`legion-spy.md` `dataflow-graph.md`** between sibling point tasks: a disjoint partition should produce **no edges** between them. Edges = aliased somewhere.
- **Legion Prof point-task serialization** despite a disjoint partition → check `partition-checks` confirms; if it passes, the issue is elsewhere (privileges, coherence).

## Failure modes
- [Non-disjoint disjoint partition](../pitfalls/non-disjoint-disjoint-partition.md) — the canonical bug.

## Source pointers
- **Legion API**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Tutorial**: https://legion.stanford.edu/tutorial/partitioning.html
- **Paper (dependent partitioning)**: `raw/publications/pdfs/dpl2016.pdf`

## Related
- `wiki/concepts/partition.md` — umbrella.
- `wiki/concepts/aliased-partition.md` — the dual.
- `wiki/concepts/non-interference.md` — what disjointness enables.
- `wiki/concepts/partition-checks.md` — runtime verifier.
- `wiki/concepts/index-space-launch.md` — primary consumer.

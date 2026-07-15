---
title: Equal Partition
slug: equal-partition
summary: The simplest partition constructor; splits an index space into N roughly-equal-sized subspaces. Disjoint and complete by construction; the default for evenly-distributed data-parallel workloads.
tags: [data-model, partitioning, for-program-reasoning]
subsystem: legion
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/08_partitioning.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
related:
  - wiki/concepts/partition.md
  - wiki/concepts/disjoint-partition.md
  - wiki/concepts/coloring.md
  - wiki/concepts/index-space.md
---

## TL;DR
`create_equal_partition` is the simplest and most-used partition constructor in Legion. Given a parent `index-space.md` and a color space, it produces a `disjoint-partition.md` whose subregions are roughly equal in size, with the runtime computing the coloring automatically. **Disjoint and complete by construction** — no `-lg:partcheck` needed to verify. The confusion: "equal" doesn't mean each subregion has *exactly* the same number of points; it means the runtime balances them as evenly as integer arithmetic allows.

## Mental model
Equal partitioning is `np.array_split(arr, n)` — split into N chunks of roughly the same size. The runtime owns the splitting logic; the application just says "give me 8 chunks". Where MPI codes compute chunk bounds by hand (and get off-by-one errors), `create_equal_partition` makes the runtime do it correctly.

## Mechanism & API
```cpp
Rect<1> elem_rect(0, num_elements - 1);
IndexSpace is = runtime->create_index_space(ctx, elem_rect);

Rect<1> color_bounds(0, num_subregions - 1);
IndexSpace color_is = runtime->create_index_space(ctx, color_bounds);

IndexPartition ip = runtime->create_equal_partition(ctx, is, color_is);
```

The result is an `IndexPartition` whose:
- Number of subregions matches the color space's size.
- Subregions are disjoint and together cover every point in the parent.
- Each subregion has `floor(N/k)` or `ceil(N/k)` points (where N is parent points, k is color count).

For multi-dimensional index spaces, the runtime splits along the highest-cardinality dimension first; you generally don't need to think about this for typical Cartesian work.

**Use the partition** in an `index-space-launch.md`:
```cpp
LogicalPartition lp = runtime->get_logical_partition(ctx, lr, ip);
IndexLauncher launcher(TASK_ID, color_is, ...);
launcher.add_region_requirement(
    RegionRequirement(lp, /*proj=*/0, RW, EXCLUSIVE, lr));
```

## Invariants
- Disjoint by construction: each parent point belongs to **exactly one** subregion.
- Complete by construction: every parent point belongs to **some** subregion.
- Subregion sizes differ by at most 1.
- The coloring is **deterministic** given the parent index space and color space — same inputs always produce the same partition.
- Works on 1D, 2D, 3D, ... index spaces.

## Performance implications
- **Negligible runtime cost** — the coloring is computed by an algorithm, not materialized; subregion data structures are lazy.
- The most efficient partition constructor; use it for any data-parallel workload where chunks should be equal-size.
- Combined with `index-space-launch.md` over the color space, you get the canonical data-parallel pattern.

## Debug signals
- **No debug signals needed** — disjointness is by construction.
- **`legion-spy.md`** region-tree output renders the partition; visually confirm chunks.

## Failure modes
- Color-space size larger than parent point count → some subregions empty (rare; usually a sizing bug elsewhere).

## Source pointers
- **Legion API**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Tutorial**: https://legion.stanford.edu/tutorial/partitioning.html

## Related
- `wiki/concepts/partition.md` — umbrella.
- `wiki/concepts/disjoint-partition.md` — this is one.
- `wiki/concepts/coloring.md` — structural coloring concept.
- `wiki/concepts/index-space.md` — what's being partitioned.

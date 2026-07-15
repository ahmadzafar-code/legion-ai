---
title: Partition by Restriction
slug: partition-by-restriction
summary: A partition constructor that builds an affine block partition from a transform matrix and extent rectangle; disjoint by construction and useful when each subregion should be a translated copy of a base shape.
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
  - wiki/concepts/equal-partition.md
  - wiki/concepts/coloring.md
---

## TL;DR
`create_partition_by_restriction` builds a partition whose subregions are translated copies of a base extent — each color `c` gets the rectangle `transform * c + base_extent`. The math gives a regular affine block partition: equal-sized rectangles laid out by translation. Disjoint by construction when the transform's stride exceeds the extent's size; useful for sliding-window patterns, regular block decompositions of higher-dimensional data, and anywhere you'd write `lo = i*stride, hi = lo+size-1` by hand. The confusion: this is the math-heavy partition constructor — you provide a transform matrix and the runtime applies it; it's not magic but it does eliminate off-by-one errors.

## Mental model
`partition_by_restriction` is "give me N copies of this rectangle, each translated to the next position". Where `equal-partition.md` divides a known size into N chunks, restriction lets you specify the *chunk shape* and *stride* directly. Useful for stencils, sliding-window aggregations, and any pattern that's "tile the index space with this fixed shape".

## Mechanism & API
```cpp
Rect<2> color_bounds({0,0}, {nx-1, ny-1});
IndexSpace color_is = runtime->create_index_space(ctx, color_bounds);

// Each chunk is a (block_x, block_y) rectangle.
Transform<2, 2> t;
t[0][0] = block_x;  t[0][1] = 0;
t[1][0] = 0;        t[1][1] = block_y;
Point<2> extent_lo({0, 0});
Point<2> extent_hi({block_x - 1, block_y - 1});

IndexPartition ip = runtime->create_partition_by_restriction(
    ctx, parent_is, color_is, t,
    Rect<2>(extent_lo, extent_hi));
```

For color `c = (cx, cy)`, the subregion bounds are:
- `lo = (t * c)` = `(cx*block_x, cy*block_y)`
- `hi = lo + (block_x-1, block_y-1)`

I.e., subregion at color `(cx, cy)` is the rectangle `[cx*block_x .. cx*block_x+block_x-1] × [cy*block_y .. cy*block_y+block_y-1]`.

**Disjointness condition**: the transform's stride must equal or exceed the extent's size in each dimension. Otherwise adjacent colors' rectangles overlap (and the result is an `aliased-partition.md`).

## Invariants
- The math is **deterministic**: `subregion(c) = clip(t*c + extent, parent_bounds)`.
- Disjoint if and only if the transform's columns produce non-overlapping translations of the extent.
- Subregions at the parent's boundary are **clipped** to the parent — they may be smaller than the extent.
- Works on N-dimensional index spaces with M-dimensional color spaces; the transform is N×M.
- The runtime can prove disjointness from the transform + extent; no `-lg:partcheck` needed.

## Performance implications
- **Cheap to construct** — math, not data-dependent computation.
- The standard choice for **regular block partitions** of N-D data (Cartesian stencils, structured grids).
- Subregion structure is lazy; the partition handle is fine to hold for arbitrarily-large color spaces.

## Debug signals
- **Subregions smaller than extent at boundaries** = parent doesn't divide evenly by the stride. Expected; the runtime clips.
- **`legion-spy.md`** region-tree output renders the partition layout.

## Failure modes
- Stride less than extent → unintended aliased partition.
- Transform with the wrong dimensionality → compile-time error.

## Source pointers
- **Legion API**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Tutorial**: https://legion.stanford.edu/tutorial/partitioning.html

## Related
- `wiki/concepts/partition.md` — umbrella.
- `wiki/concepts/disjoint-partition.md` — typical result.
- `wiki/concepts/equal-partition.md` — sibling constructor.
- `wiki/concepts/coloring.md` — structural coloring.

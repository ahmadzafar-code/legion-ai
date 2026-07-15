---
title: Coloring
slug: coloring
summary: The assignment of points to colors that defines a partition; either supplied by the application (manual / by-field) or computed by the runtime (equal / by-restriction).
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
  - wiki/concepts/index-space.md
  - wiki/concepts/disjoint-partition.md
  - wiki/concepts/aliased-partition.md
  - wiki/concepts/dependent-partitioning.md
---

## TL;DR
A coloring is the mapping from points to colors that defines an `IndexPartition` — "point P belongs to subregion at color C". A coloring can be **explicit** (the application supplies `{point → color}` pairs), **functional** (a `partition-by-field` reads the color from a field per point), or **structural** (`create_equal_partition` / `create_partition_by_restriction` produce the coloring from the index space's shape). The confusion: "coloring" doesn't mean assigning unique IDs to points — multiple points can share a color (that's how chunks form), and the partition is disjoint iff each point has *at most one* color.

## Mental model
A coloring is the equivalence relation that defines a partition: "two points are in the same chunk iff they have the same color". The partition's structure (disjoint vs. aliased, complete vs. incomplete) falls out of the coloring's properties:
- Each point in **at most one** color set → disjoint partition.
- Each point in **at least one** color set → complete partition.
- Each point in **exactly one** color set → disjoint **and** complete.

## Mechanism & API
**Equal coloring** (structural, disjoint+complete by construction):
```cpp
IndexPartition ip = runtime->create_equal_partition(ctx, is, color_space);
```
Splits the parent's points evenly across the color space's points.

**By restriction** (structural, affine block; disjoint+complete by construction):
```cpp
Transform<2,1> t;  t[0][0] = stride;  t[1][0] = 0;
Point<2> extent = ...;
IndexPartition ip = runtime->create_partition_by_restriction(
    ctx, parent_is, color_space, t, extent);
```
Computes the coloring from an affine transform + extent — each color gets a translated copy of `extent`.

**By field** (functional, derived from data):
```cpp
IndexPartition ip = runtime->create_partition_by_field(
    ctx, lr, parent, FID_COLOR, color_space, /*disjoint=*/true);
```
Coloring = the value of `FID_COLOR` at each point. Disjoint iff each point has at most one value (always true for a single field).

**Manual coloring** (explicit):
```cpp
PointColoring point_coloring;
point_coloring[DomainPoint(0)].points.insert(...);
// ... fill in coloring ...
IndexPartition ip = runtime->create_index_partition(ctx, parent_is,
    color_space, point_coloring, /*part_kind=*/DISJOINT_KIND);
```

`PointColoring` is the explicit `color → set-of-points` map; suitable for irregular partitions that none of the structural constructors handle.

## Invariants
- A coloring uniquely determines an `IndexPartition`.
- The **disjointness** of the resulting partition is a property of the coloring: each point in at most one color → disjoint.
- The **completeness** is also a property: each point in at least one color → complete.
- Coloring values themselves are points in a separate `IndexSpace` called the **color space** — colors are not raw integers, they're `DomainPoint`s.
- The runtime trusts disjointness declarations (`disjoint=true`); use `partition-checks.md` to verify.
- Manual colorings via `PointColoring` are O(#points) in memory — for large index spaces, prefer structural or functional constructors.

## Performance implications
- **Structural colorings** (`equal`, `by_restriction`) are cheapest — the runtime doesn't need to materialize the per-point assignment, it computes on demand.
- **Functional colorings** (`by_field`) require one pass over the source data; cost is O(#points). Cache the result if reused.
- **Explicit colorings** (`PointColoring`) hold the full assignment in memory — large index spaces become memory-bottlenecked here.
- Most application bugs related to coloring manifest as `pitfalls/non-disjoint-disjoint-partition.md` — declared disjoint, actually aliased.

## Debug signals
- **`partition-checks.md`** (`-lg:partcheck`) confirms a declared-disjoint coloring is actually disjoint at creation time.
- **`legion-spy.md`** region-tree output renders the coloring graphically — visually confirm color assignments match intent.
- **Error codes 351-450** (`error-message-catalog.md`) cover coloring/partition issues: out-of-range colors, completeness violations, image/preimage errors.

## Failure modes
- A `PointColoring` that overlaps colors but the application asserts `DISJOINT_KIND` → `pitfalls/non-disjoint-disjoint-partition.md`.
- Manual coloring with off-by-one errors at chunk boundaries → same problem.
- Functional coloring over a field with unexpected duplicate values → aliased partition; declaring disjoint is a bug.

## Source pointers
- **Legion API**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Tutorial**: https://legion.stanford.edu/tutorial/partitioning.html
- **Paper (dependent partitioning)**: `raw/publications/pdfs/dpl2016.pdf`

## Related
- `wiki/concepts/partition.md` — what a coloring defines.
- `wiki/concepts/index-space.md` — what a coloring assigns colors to.
- `wiki/concepts/disjoint-partition.md` — what a non-overlapping coloring produces.
- `wiki/concepts/aliased-partition.md` — what an overlapping coloring produces.
- `wiki/concepts/dependent-partitioning.md` — data-driven coloring family.

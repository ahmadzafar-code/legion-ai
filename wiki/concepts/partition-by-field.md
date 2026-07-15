---
title: Partition by Field
slug: partition-by-field
summary: A dependent-partitioning constructor that colors each point by the value of a chosen field; the standard way to partition data driven by per-point ownership annotations.
tags: [data-model, partitioning, for-program-reasoning, for-perf-debug]
subsystem: legion
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/08_partitioning.md
  - raw/publications/publications.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
related:
  - wiki/concepts/partition.md
  - wiki/concepts/dependent-partitioning.md
  - wiki/concepts/disjoint-partition.md
  - wiki/concepts/coloring.md
  - wiki/concepts/partition-checks.md
---

## TL;DR
`create_partition_by_field` colors each point of an `index-space.md` by reading a chosen field's value at that point — point `p` gets color `field[p]`. The result is a partition whose disjointness depends on the field values: if each point has exactly one value, it's `disjoint-partition.md`; if values overlap (unlikely from a single field), it's aliased. The standard tool for "partition my data by which processor / node / category owns each cell". The confusion: the partition is **only as correct as the field data**; declaring `disjoint=true` and having stale field data later is a `pitfalls/non-disjoint-disjoint-partition.md` waiting to happen.

## Mental model
`partition_by_field` is `groupby(field_value)` for partitioning — group points by some attribute and turn each group into a subregion. Standard pattern: an ownership/categorization field colors the data, then index launches over the color space distribute work.

## Mechanism & API
```cpp
// Logical region holding the data + a color field.
LogicalRegion lr = ...;       // built over (is, fs)
// The color field FID_COLOR has, for each point, the color it should belong to.

Rect<1> color_bounds(0, num_colors - 1);
IndexSpace color_is = runtime->create_index_space(ctx, color_bounds);

IndexPartition ip = runtime->create_partition_by_field(
    ctx, lr, /*parent=*/lr, /*fid=*/FID_COLOR, color_is,
    /*part_kind=*/LEGION_DISJOINT_KIND);
```

For each point `p`, the runtime reads `lr.FID_COLOR[p]` and assigns `p` to the subregion at that color. The runtime materializes the partition lazily and respects the asynchronous nature of the read.

**Partition kind** (the last argument):
- `LEGION_DISJOINT_KIND` — application asserts each point has exactly one color (typical for ownership fields).
- `LEGION_ALIASED_KIND` — application acknowledges values may overlap (rare for single-field).
- `LEGION_COMPUTE_KIND` — runtime checks at creation time. Combined with `-lg:partcheck`, this is the safest option.

## Invariants
- Each point's color is read from the named field; the partition is **data-dependent**.
- Disjointness depends on the field's values — if every point has a unique color value, the result is disjoint.
- The construction call is asynchronous; the returned `IndexPartition` becomes usable once the field's data is settled.
- Subsequent changes to the field's values do **not** invalidate the partition — it captures a snapshot at creation time.
- Combined with `-lg:partcheck` (`partition-checks.md`) when `part_kind=LEGION_DISJOINT_KIND`, the runtime verifies the assertion at creation.

## Performance implications
- Cost is **O(parent points)** at partition creation — one pass over the field.
- Cache the partition handle; re-creating it from the same field repeatedly is wasted work.
- For iterative codes where the coloring field changes, re-creating is necessary — and visible as utility-row activity in `legion-prof.md`.
- The partition's lazy materialization keeps cold subregions free; only used ones cost memory.

## Debug signals
- **`-lg:partcheck`** with `LEGION_DISJOINT_KIND` confirms the assertion holds at creation time. Use this when changing coloring code.
- **`legion-spy.md`** region-tree output renders the partition's subregions; visually confirm chunk assignment.
- **Aliased "disjoint" partition discovered at runtime** → `pitfalls/non-disjoint-disjoint-partition.md`; the field had duplicates.

## Failure modes
- Coloring field with duplicate values + `LEGION_DISJOINT_KIND` → silent race; catch with `partition-checks.md`.
- Stale coloring field at partition-creation time → wrong partition; clear correctness bug.

## Source pointers
- **Legion API**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Tutorial**: https://legion.stanford.edu/tutorial/partitioning.html
- **Paper**: `raw/publications/pdfs/dpl2016.pdf` (Dependent Partitioning, OOPSLA 2016).

## Related
- `wiki/concepts/partition.md` — umbrella.
- `wiki/concepts/dependent-partitioning.md` — sibling constructors.
- `wiki/concepts/disjoint-partition.md` — typical result.
- `wiki/concepts/coloring.md` — functional coloring.
- `wiki/concepts/partition-checks.md` — runtime verifier.

---
title: Partition by Preimage
slug: partition-by-preimage
summary: The inverse of partition-by-image; for each color, the new subregion contains all *source* points whose pointers land in the target partition's color-`c` subregion.
tags: [data-model, partitioning, for-program-reasoning]
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
  - wiki/concepts/partition-by-image.md
  - wiki/concepts/disjoint-partition.md
  - wiki/concepts/aliased-partition.md
---

## TL;DR
`create_partition_by_preimage` is the **inverse** of `partition-by-image.md`: given an existing partition of a target space and a pointer field from a source space, it builds a partition of the source space where each color `c`'s subregion contains every source point whose pointer lands in the target partition's color-`c` subregion. Useful for "which source cells contribute to this target chunk?" queries — the dual question to "what target cells do my source cells touch?". Disjointness depends on whether a source point's pointer can land in multiple target colors; for a single-pointer field, that's impossible, so the preimage is disjoint when the target partition is disjoint.

## Mental model
`partition_by_preimage` answers the question "given a partition of targets, which sources feed into each target color?". Where `partition-by-image.md` is "expand my chunks to include all the cells they touch", preimage is "narrow the sources down to those that feed a particular target chunk". The two compose: image(preimage(P)) and preimage(image(P)) are typically supersets of P.

## Mechanism & API
```cpp
// target_partition is a partition of target_is.
// source_lr has FID_PTR pointing into target_is.

IndexPartition preimage_ip = runtime->create_partition_by_preimage(
    ctx, target_partition, source_lr, /*parent=*/source_lr,
    /*fid=*/FID_PTR, color_space,
    /*part_kind=*/LEGION_COMPUTE_KIND);
```

For each color `c`:
- The runtime walks every source point.
- For each, it reads `FID_PTR` to get the target it points to.
- If the target lies in `target_partition[c]`, the source point is added to `preimage_ip[c]`.

**Variants**:
- `create_partition_by_preimage_range` — `FID_PTR` is a range; preimage includes any source whose range overlaps any color's subregion.

## Invariants
- The result is **disjoint iff** `target_partition` is disjoint **and** `FID_PTR` is a single-point pointer field (no source has multiple targets across colors).
- The runtime can prove disjointness from these properties when `LEGION_COMPUTE_KIND` is used.
- Like all dependent partitions, the construction is a snapshot — subsequent changes to `FID_PTR` don't invalidate the partition.
- Asynchronous; usable once the partition's readiness event triggers.
- The number of subregions matches the target partition's color space.

## Performance implications
- Cost is **O(source points)** at construction.
- Often paired with `partition-by-image.md` to build round-trip "owner → halo → contributors" patterns in unstructured-mesh codes.
- The runtime caches the partition; iterative codes should reuse rather than rebuild.

## Debug signals
- **`-lg:partcheck`** + `LEGION_DISJOINT_KIND` confirms disjointness for single-pointer-field cases.
- **`legion-spy.md`** region-tree output shows the resulting subregions.

## Failure modes
- Multi-pointer `FID_PTR` (range) + asserting disjoint → catch with `partition-checks.md`; aliased preimage is expected for range fields.
- Source pointers outside `target_is` → error at construction.

## Source pointers
- **Legion API**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Tutorial**: https://legion.stanford.edu/tutorial/partitioning.html
- **Paper**: `raw/publications/pdfs/dpl2016.pdf`.

## Related
- `wiki/concepts/partition.md` — umbrella.
- `wiki/concepts/dependent-partitioning.md` — family.
- `wiki/concepts/partition-by-image.md` — the inverse.
- `wiki/concepts/disjoint-partition.md` — typical result for single-pointer fields.
- `wiki/concepts/aliased-partition.md` — typical result for range fields.

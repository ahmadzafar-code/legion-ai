---
title: Partition by Image
slug: partition-by-image
summary: A dependent-partitioning constructor that builds a partition of a target index space by projecting an existing partition through a pointer field; "for each color, the new subregion contains all the *targets* of pointers in the source partition's subregion of that color".
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
  - wiki/concepts/aliased-partition.md
  - wiki/concepts/ghost-region.md
  - wiki/concepts/partition-by-preimage.md
---

## TL;DR
`create_partition_by_image` builds a partition of a target index space by **projecting an existing partition through a pointer field**: for each color `c`, the new subregion at color `c` contains every target point that a pointer in the source partition's color-`c` subregion points to. The standard use case is **halo computation** for unstructured meshes — the source partition is "cells I own", the pointer field is "cells my cells touch", and the image is "cells my owned cells reach". Disjointness depends on whether different source colors' pointers can hit the same target — usually they can, so the image is aliased. The confusion: "image" is the math sense — the set-theoretic image of a function — not a picture.

## Mental model
`partition_by_image` is the dependent-partitioning answer to "given that I own these mesh cells and they touch those cells, give me a subregion of the cells I touch". The runtime computes the image set automatically; the application supplies the pointer field. Standard tool for unstructured-mesh codes that would otherwise build neighbor tables by hand.

## Mechanism & API
```cpp
// source_partition is a disjoint partition of source_is.
// FID_PTR is a pointer-typed field that, for each source point,
// names a target point in target_is.

IndexPartition image_ip = runtime->create_partition_by_image(
    ctx, target_is,
    runtime->get_logical_partition(ctx, source_lr, source_partition),
    /*parent=*/source_lr, /*fid=*/FID_PTR, color_space,
    /*part_kind=*/LEGION_COMPUTE_KIND);
```

For each color `c`:
- The runtime walks `source_partition[c]`'s points.
- For each, it reads `FID_PTR` to get a target point.
- All target points encountered form `image_ip[c]`.

If the same target point is reached from multiple source colors, the resulting image is **aliased**. The application declares disjointness via the `part_kind` argument; `LEGION_COMPUTE_KIND` lets the runtime decide.

**Variants**:
- `create_partition_by_image_range` — `FID_PTR` is a range (lo, hi), and the image includes the whole range, not just a single point.

## Invariants
- The runtime reads `FID_PTR` once per source point at construction time; the partition is a snapshot.
- Disjointness of the result is **data-dependent** — derived from the pointer values, not declared by the application.
- With `LEGION_COMPUTE_KIND`, the runtime determines disjointness; with `LEGION_DISJOINT_KIND`, the application asserts and `-lg:partcheck` can verify.
- Aliased images are **legal and common** — halo regions are the canonical example.
- The construction call is asynchronous; subsequent uses wait on the partition's readiness event.

## Performance implications
- **Cost is O(source points)** at partition creation, plus a sort/group step for the image set.
- For very large source partitions, image construction can be a measurable cost — visible in `legion-prof.md` utility rows.
- Caching is critical for iterative codes that build the same image repeatedly.
- Combined with `ghost-region.md` patterns, image partitions enable halo exchange without manual neighbor tables.

## Debug signals
- **`-lg:partcheck`** with `LEGION_DISJOINT_KIND` confirms (or denies) disjointness at creation time.
- **`legion-spy.md`** region-tree shows the resulting partition's subregions; visually confirm they include the expected neighbors.
- **Unexpected `pitfalls/non-disjoint-disjoint-partition.md`** under image partitions → the pointer field has overlaps between source colors.

## Failure modes
- `FID_PTR` with overlapping ranges across source colors + `LEGION_DISJOINT_KIND` → silent race; catch with `partition-checks.md`.
- Pointer values outside `target_is` → caught at creation time (typically with an error).

## Source pointers
- **Legion API**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Tutorial**: https://legion.stanford.edu/tutorial/partitioning.html
- **Paper**: `raw/publications/pdfs/dpl2016.pdf`.

## Related
- `wiki/concepts/partition.md` — umbrella.
- `wiki/concepts/dependent-partitioning.md` — family.
- `wiki/concepts/disjoint-partition.md` / `wiki/concepts/aliased-partition.md` — possible results.
- `wiki/concepts/ghost-region.md` — standard use case.
- `wiki/concepts/partition-by-preimage.md` — the inverse.

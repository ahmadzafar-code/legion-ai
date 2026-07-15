---
title: Ghost Region
slug: ghost-region
summary: A halo region — the slice of a neighbor's chunk that a stencil task must read; expressed in Legion as a subregion of an aliased partition that overlaps a sibling's interior.
tags: [data-model, partitioning, parallelism, for-program-reasoning, for-perf-debug]
subsystem: legion
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/09_multiple_partitions.md
  - raw/tutorials/12_explicit_ghost_regions.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
related:
  - wiki/concepts/aliased-partition.md
  - wiki/concepts/disjoint-partition.md
  - wiki/concepts/partition.md
  - wiki/concepts/projection-functor.md
  - wiki/concepts/non-interference.md
  - wiki/applications/circuit.md
  - wiki/applications/pennant.md
  - wiki/applications/miniaero.md
---

## TL;DR
A ghost region (a.k.a. halo) is the small slice of a neighboring chunk that a stencil task needs to read in order to compute its own chunk's update — the boundary values of the cell to the left, above, and in front. In Legion, ghost regions are expressed as subregions of an `aliased-partition.md` that overlaps neighbors' disjoint chunks. The standard pattern uses **two partitions of the same region**: a disjoint partition for writes (each chunk owns its cells exclusively) and an aliased partition for reads (each chunk reads its own interior plus halo cells from neighbors). The confusion: ghost regions are not a separate Legion type — they're just subregions, but their *purpose* in the program is the halo.

## Mental model
Ghost regions are MPI's halo exchange made first-class. Where MPI codes have explicit `MPI_Sendrecv` calls to exchange boundary data with neighbors, Legion codes declare an aliased read partition that overlaps neighbors — and the runtime works out the implied copies automatically. The application gets the same parallelism as MPI without writing the message-passing boilerplate.

## Mechanism & API
**Tutorial 9 / Tutorial 12 pattern**:
```cpp
// 1. The disjoint partition: each chunk's own interior cells.
IndexPartition disjoint_ip = runtime->create_equal_partition(ctx, is, color_is);
LogicalPartition disjoint_lp = runtime->get_logical_partition(ctx, lr, disjoint_ip);

// 2. The aliased partition: each chunk's own interior + neighbor cells.
// Often built via partition_by_image or partition_by_union of disjoint +
// adjacent slices.
IndexPartition aliased_ip = build_halo_partition(disjoint_ip, halo_width);
LogicalPartition aliased_lp = runtime->get_logical_partition(ctx, lr, aliased_ip);

// 3. The stencil task reads aliased_lp (with halo) and writes disjoint_lp (own cells).
IndexLauncher launcher(STENCIL_TASK_ID, color_is, ...);
launcher.add_region_requirement(
    RegionRequirement(aliased_lp, /*proj=*/0, READ_ONLY, EXCLUSIVE, lr));  // halo
launcher.region_requirements[0].add_field(FID_X_PREV);
launcher.add_region_requirement(
    RegionRequirement(disjoint_lp, /*proj=*/0, READ_WRITE, EXCLUSIVE, lr));  // own
launcher.region_requirements[1].add_field(FID_X_NEXT);
```

Inside `stencil_task`:
- `regions[0]` is the aliased subregion (own + neighbors); `READ_ONLY` so concurrent readers don't conflict.
- `regions[1]` is the disjoint subregion (own only); `READ_WRITE` for the update.

The runtime, seeing one `READ_ONLY` aliased + one `READ_WRITE` disjoint, infers that:
- The disjoint writes don't conflict with each other (independent chunks).
- The aliased reads need synchronization with the **prior** disjoint writes to their target neighbors — the runtime issues the implied copies as needed.

In iterative stencils, the previous iteration's write to `FID_X_NEXT` becomes the next iteration's read from `FID_X_PREV` (via field rotation), and the halo is always read-only with respect to the current iteration's writes — fully parallel within each iteration.

**Tutorial 12** (`12_explicit_ghost_regions.md`) shows the explicit form where the application builds the halo partition itself via dependent partitioning (`dependent-partitioning.md`).

## Invariants
- A ghost region is **just a subregion** — Legion has no special "ghost type".
- The aliased partition that defines ghost regions must be **declared aliased** (`disjoint=false`); declaring it disjoint with overlapping subregions is a race (`pitfalls/non-disjoint-disjoint-partition.md`).
- The runtime issues copies to keep halo data current; the application does **not** write halo-exchange code explicitly.
- Two iterations of a stencil with field rotation produce no false dependences between same-iteration chunks because the read-halo references the *previous* field, which the current iteration's writes don't touch.
- Halo width is application-specified (1 cell for nearest-neighbor stencils, larger for higher-order schemes).

## Performance implications
- **The Legion-native way to express halo exchange**; the runtime overlaps halo copies with compute by default when the dependence structure permits.
- A larger halo costs more inter-chunk copy bandwidth — visible in `dma-system.md` channel rows.
- For multi-node runs, halos become inter-node copies; consider where the partition's color space distributes across nodes.
- Combined with **tracing** (`tracing.md`), the halo-exchange pattern collapses to memoized event chains on the second and subsequent iterations.

## Debug signals
- **Legion Prof channel rows** during a stencil iteration show the halo-exchange copies. Healthy: bounded, predictable. Unhealthy: oscillating, growing, or larger than expected → wrong halo partition.
- **`legion-spy.md`** dataflow graph shows the read-halo edges from prior writers; visually distinct from the local-chunk writes.
- **Wrong stencil results at chunk boundaries** → the halo partition is too narrow or doesn't include the right neighbors.

## Failure modes
- Aliased partition declared disjoint → silent race; see `pitfalls/non-disjoint-disjoint-partition.md`.
- Halo width smaller than the stencil's footprint → boundary cells read stale data.

## Source pointers
- **Legion API**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Tutorial (multiple partitions)**: https://legion.stanford.edu/tutorial/multiple.html
- **Tutorial (explicit ghost regions)**: `raw/tutorials/12_explicit_ghost_regions.md`

## Related
- `wiki/concepts/aliased-partition.md` — the partition kind ghost regions live in.
- `wiki/concepts/disjoint-partition.md` — the companion write-partition.
- `wiki/concepts/partition.md` — umbrella.
- `wiki/concepts/projection-functor.md` — addresses per-point halo subregions.
- `wiki/concepts/non-interference.md` — why the read-halo + disjoint-write pattern parallelizes.

---
title: Dependent Partitioning
slug: dependent-partitioning
summary: A family of operations (partition_by_field / by_image / by_preimage / by_difference / by_union / by_intersection) that compute one partition from existing data — the standard mechanism for unstructured-mesh halos and pointer-following partitions.
tags: [data-model, partitioning, for-program-reasoning, for-perf-debug]
subsystem: legion
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/publications/pdfs/dpl2016.pdf
  - raw/tutorials/08_partitioning.md
  - raw/publications/publications.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
related:
  - wiki/concepts/partition.md
  - wiki/concepts/disjoint-partition.md
  - wiki/concepts/aliased-partition.md
  - wiki/concepts/ghost-region.md
  - wiki/concepts/projection-functor.md
  - wiki/concepts/partition-by-field.md
  - wiki/concepts/partition-by-image.md
  - wiki/concepts/partition-by-preimage.md
  - wiki/concepts/equal-partition.md
  - wiki/concepts/partition-by-restriction.md
  - wiki/applications/pennant.md
---

## TL;DR
Dependent partitioning (paper `dpl2016.pdf`, OOPSLA 2016) is the family of operations that compute a new `IndexPartition` from existing data — the values of a field, an existing partition, or the result of set operations between partitions. The most useful members: `partition_by_field` (color from a field value), `partition_by_image` (project a partition through pointers), `partition_by_preimage` (the inverse), and the boolean set ops (`by_union`/`by_intersection`/`by_difference`). The framework is a small **dependent partitioning sublanguage (DPL)** the runtime can statically analyze. Key results from the paper: **86-96% reduction** in partitioning code (vs. hand-rolled colorings) and **2.6-12.7×** speedup on single-thread partitioning; further **29×** on 64 nodes for distributed partitions. The confusion: dependent partitioning is not a *kind* of partition (disjoint vs aliased) — it's a *way of constructing one*. The result can be disjoint or aliased depending on the input data.

## Mental model
Dependent partitioning is partition-as-a-query: "for each color in `color_space`, return the set of points whose field value equals this color" (by_field), or "for each color, return the set of points pointed to by points in the source partition's color" (by_image). The application declares its intent; the runtime materializes the result. For unstructured meshes and graph-style data, this is how you partition without writing manual neighbor tables.

## Mechanism & API

**DPL — Dependent Partitioning Language** (`dpl2016.pdf` §2): the framework is formalized as a small imperative sublanguage with constructs for index spaces, fields, functions, properties, and assertions. The grammar (Fig. 2 in the paper):
- `idx` declares an index space.
- `field id : idxtype → rngtype` declares a field; rng may be a base type or another index space (a pointer).
- `function id : basetype → basetype; property propstmt` declares a function with assertable properties (e.g., `function left(x) = x-1; property left(x) ≥ 0`) — these properties enable static analysis without running the function.
- Set operations: `idx S = A & B` (intersection), `A | B` (union), `A - B` (difference).
- Image / preimage: `A → f` (image of A through field f), `A ← f` (preimage).
- `assert` clauses on disjointness (`A * B`) or containment (`A ≤ B`).

DPL is **decidable and statically analyzable** (paper §3): assertions in DPL programs are checked by translating them into a fragment of first-order logic with Presburger arithmetic + uninterpreted functions, then dispatched to an SMT solver (Z3 / CVC4). The paper proves a NEXPTIME complexity bound for the DPLSAT decision procedure — exponential worst case, fast in practice.

**Output-sensitive algorithm** for image/preimage (`dpl2016.pdf` §5.2): naive computation of `image(P, partition, FID_PTR)` is O(N²) — every source point may point to every target. The DPL implementation uses an **output-sensitive algorithm** that runs in O(N log N + M) where M is the number of non-empty image subregions. Approximate index spaces (bounding intervals + sparse cluster lists) further speed up the cross-rank intersection tests.

**The main entry points** in Legion (per `raw/tutorials/08_partitioning.md` and `dpl2016.pdf`):

- **`create_partition_by_field`** — color from a `FieldID`'s value:
  ```cpp
  // For each point in lr, look up its color from field FID_COLOR;
  // group points with the same color into a subregion.
  IndexPartition ip = runtime->create_partition_by_field(
      ctx, lr, parent, FID_COLOR, color_space);
  ```
  Disjoint by construction (each point has one color).

- **`create_partition_by_image`** — image of an existing partition through a pointer field:
  ```cpp
  // For each color C, the new subregion contains all the *targets* of pointers
  // stored in source_partition[C].
  IndexPartition image_ip = runtime->create_partition_by_image(
      ctx, parent_is, source_partition, lr, FID_PTR, color_space);
  ```
  The result is disjoint iff the source pointers are unique; aliased otherwise.

- **`create_partition_by_preimage`** — the inverse: for each color, the *sources* whose pointers land in that target subregion.

- **Set operations** — `create_partition_by_union(a, b)`, `create_partition_by_intersection(a, b)`, `create_partition_by_difference(a, b)`. Useful for building halo partitions: "expanded chunk" = union of (interior partition, neighbors partition).

- **`create_pending_partition`** — manual construction; the application supplies the coloring directly.

## Invariants
- The runtime materializes dependent partitions **lazily** — region-tree nodes for subregions exist only when used.
- A dependent partition's disjointness is **derived from the input data**; the application must declare it (with the `disjoint=true`/`false` flag) and the runtime trusts the declaration. Use `partition-checks.md` to verify.
- Set operations (`by_union`/`by_intersection`/`by_difference`) preserve disjointness when their inputs satisfy the relevant algebraic property — see the paper for the cases.
- The computation is performed asynchronously; the returned `IndexPartition` becomes usable once the partition-creation event triggers.
- `partition_by_image`/`preimage` over large data sets is expensive — visible as a long bar on `legion-prof.md` utility rows.

## Performance implications

Per `dpl2016.pdf` §6 (PENNANT, Circuit, MiniAero benchmarks):

- **The standard way to express unstructured-mesh and graph partitions** without hand-built neighbor tables.
- **86-96% reduction** in partition-related code: PENNANT went from 163 LOC to 6, Circuit from 159 to 8, MiniAero from 51 to 7.
- **2.6-12.7× speedup** on single-thread partitioning vs. hand-rolled implementations — the output-sensitive image/preimage algorithm dominates.
- **Verification dynamic checks** at runtime are nearly eliminated (most assertions discharged statically) — additional 25% wall-time win on PENNANT.
- **29× speedup at 64 nodes** for distributed partitions — the implementation parallelizes the partition computation across nodes.
- Lazy materialization keeps cold subregions free; only used ones cost memory.
- For halo patterns: `by_image` from a disjoint owner partition typically produces an aliased halo partition automatically. Pair with `ghost-region.md`.
- For iterative workloads, cache the partition handle; recomputing dependent partitions every step is wasted unless the source data actually changes.

## Debug signals
- **Partition-creation calls dominating Legion Prof utility rows** → expensive dependent partitioning; consider caching.
- **`-lg:partcheck`** catches misdeclared disjointness in dependent partitions (`partition-checks.md`).
- **`legion-spy.md`**'s region-tree output renders the partition's coloring; useful for confirming the partition matches your intent.

## Failure modes
- `partition_by_image` over a field with duplicate pointer values → resulting partition is **aliased**; declaring `disjoint=true` triggers a `partition-checks.md` failure.
- Computing a heavy dependent partition every iteration of a loop → repeated expensive analysis; cache.

## Source pointers
- **Legion API**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Paper**: `raw/publications/pdfs/dpl2016.pdf` — *Dependent Partitioning* (OOPSLA 2016)
- **Tutorial**: https://legion.stanford.edu/tutorial/partitioning.html

## Related
- `wiki/concepts/partition.md` — umbrella.
- `wiki/concepts/disjoint-partition.md` — common result.
- `wiki/concepts/aliased-partition.md` — also a common result.
- `wiki/concepts/ghost-region.md` — frequent use case.
- `wiki/concepts/projection-functor.md` — companion mechanism for per-point addressing.

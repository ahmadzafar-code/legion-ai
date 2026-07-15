---
title: Index Space Launch
slug: index-space-launch
summary: A single Legion API call that creates many parallel point tasks indexed by a Rect/Domain; the standard, scalable way to express data-parallel work.
tags: [execution, parallelism, for-perf-debug, for-program-reasoning]
subsystem: legion
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/03_index_tasks.md
  - raw/tutorials/08_partitioning.md
  - raw/publications/publications.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
related:
  - wiki/concepts/task.md
  - wiki/concepts/partition.md
  - wiki/concepts/logical-analysis.md
  - wiki/concepts/control-replication.md
  - wiki/concepts/mapper.md
  - wiki/concepts/future-map.md
  - wiki/concepts/task-launcher.md
  - wiki/concepts/argument-map.md
  - wiki/concepts/slice-task.md
  - wiki/concepts/projection-functor.md
---

## TL;DR
An **index-space launch** is a single API call (`IndexTaskLauncher` → `Runtime::execute_index_space`) that creates many task instances ("point tasks"), each labeled with a point from an index domain. The launch returns a `FutureMap` indexing per-point results. Each point task gets per-point arguments from an `ArgumentMap` and a subregion of any region requirement that uses a `LogicalPartition` with a projection functor. The confusion: in the runtime's logical analysis the entire index launch is a **single operation node**, regardless of how many point tasks it expands to — that's the scaling property the technique gives you.

## Mental model
Index-space launches are Legion's SIMD/SPMD: one verb at the source level, many independent instances at execution time. The point space is the program-visible parallel-for iteration domain; the projection functor is the addressing scheme that maps point → subregion. The paper `idx2021.pdf` describes how this lets the runtime represent billions of parallel tasks compactly.

## Mechanism & API
```cpp
// 1. Define a Rect/Domain — the iteration space.
Rect<1> launch_bounds(0, num_points - 1);

// 2. (Optional) Build an ArgumentMap of per-point inputs.
ArgumentMap args;
for (int i = 0; i < num_points; i++)
  args.set_point(i, TaskArgument(&input_for_i, sizeof(int)));

// 3. Build the launcher.
IndexTaskLauncher launcher(TASK_ID, launch_bounds, TaskArgument(NULL, 0), args);

// 4. (For partitioned data) attach a region requirement using a LogicalPartition + projection.
launcher.add_region_requirement(
    RegionRequirement(input_lp, /*projection_id=*/0, READ_ONLY, EXCLUSIVE, input_lr));
launcher.region_requirements[0].add_field(FID_X);

// 5. Execute. Returns a FutureMap.
FutureMap fm = runtime->execute_index_space(ctx, launcher);
fm.wait_all_results();
// fm.get_result<T>(point) reads one point's return value.
```

Inside the task body:
- `task->index_point` — the `DomainPoint` for this point task.
- `task->local_args` / `task->local_arglen` — the per-point bytes from the `ArgumentMap`.
- `task->args` / `task->arglen` — the global `TaskArgument` shared by all points.
- `regions[i]` — the `PhysicalRegion` for each region requirement (already projected to this point's subregion).

Projection functors decide which subregion each point reaches into. ID `0` is the identity (`point i → subregion at color i`); custom functors implement stencils, halos, gather patterns. See `partition.md`.

## Invariants
- The index launch is **one logical-analysis node**, regardless of how many points. Per-point dependencies emerge in `physical-analysis.md`.
- Point tasks within an index launch run on a single registered `TaskID` (one task variant or a set of variants); each point may map to a different processor.
- The `FutureMap` is **complete only when all point tasks have completed**; `wait_all_results()` is a fence on the whole launch.
- If the region requirement uses a `LogicalPartition` + projection, the per-point subregion is determined by the projection functor — the mapper does not pick it.
- Each point task gets exactly one `local_args`/`local_arglen` slice from the `ArgumentMap`; if no entry is set for a point, it gets empty `local_args`.

## Performance implications
- **The default scaling primitive in Legion.** Replace any `for` loop of `execute_task` calls with `execute_index_space` when the work is data-parallel.
- One logical-analysis node instead of N means logical analysis cost is O(1) in N, not O(N) (paper `idx2021.pdf`). For large N this is the difference between a runnable program and one stuck in stage 2.
- Combined with disjoint `partition.md`, the point tasks parallelize fully. Combined with aliased partitions and ghost regions, they implement stencils with minimum dependency.
- Under `control-replication.md`, each shard handles 1/N of the point space — multiplying the scaling win.
- Under `tracing.md`, the per-launch analysis is memoized on repeat — multiplying the scaling win again.

## Debug signals
- **Legion Spy dataflow graph**: a single colored node represents the entire index launch (Lesson 7). Use the **event graph** (`-e`) to see the per-point structure.
- **Legion Prof**: point tasks of one index launch appear nearly simultaneously on their target processor rows. If they serialize, the partition is aliased or the privilege is wrong.
- **FutureMap completion**: `fm.wait_all_results()` is a synchronization point; in Prof, it appears as a fence with all point bars completing before it.

## Failure modes
- [Long dependence chains](../pitfalls/long-dependence-chains.md) — using `for { execute_task(); }` instead of `execute_index_space()`.
- [Non-disjoint "disjoint" partition](../pitfalls/non-disjoint-disjoint-partition.md) — point tasks of the launch serialize.

## Source pointers
- **Legion API**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Tutorial**: https://legion.stanford.edu/tutorial/index_tasks.html
- **Paper (scaling)**: `raw/publications/pdfs/idx2021.pdf` — *Index Launches: Scalable, Flexible Representation of Parallel Task Groups*

## Related
- `wiki/concepts/task.md` — single-task counterpart.
- `wiki/concepts/partition.md` — how to give each point its own data.
- `wiki/concepts/logical-analysis.md` — why this scales: 1 logical node, not N.
- `wiki/concepts/control-replication.md` — splits the launch's analysis across shards.
- `wiki/concepts/mapper.md` — `slice_task` carves point ownership across processors.

---
title: Projection Functor
slug: projection-functor
summary: A pure function that maps each point of an index-launch domain to a specific subregion of a logical partition; the bridge from "iteration domain" to "per-point data".
tags: [data-model, partitioning, execution, for-program-reasoning, for-perf-debug]
subsystem: legion
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/08_partitioning.md
  - raw/tutorials/03_index_tasks.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
related:
  - wiki/concepts/partition.md
  - wiki/concepts/index-space-launch.md
  - wiki/concepts/region-requirement.md
  - wiki/concepts/logical-region.md
  - wiki/concepts/non-interference.md
---

## TL;DR
A projection functor is a registered, deterministic, pure function `point → subregion-color` used by `index-space-launch.md`s to give each point task its own slice of a partition. The reserved projection ID **0** is the identity — point `i` gets subregion at color `i`. Custom projection functors implement stencils, halos, gather/scatter, and any other "point → which subregion" mapping. The confusion: a projection functor doesn't decide *which task* runs at each point — only *which subregion* that point task accesses. The task body still does the actual work.

## Mental model
A projection functor is the addressing scheme between an index launch's iteration domain and a partition's color space. It answers "given point P in my iteration domain, which subregion of partition LP should I access?". Where MPI codes write `data[rank]` to address per-rank state, Legion writes `lp[projection(point)]`.

## Mechanism & API
**The reserved identity functor** (ID = 0):
```cpp
IndexTaskLauncher launcher(TASK_ID, color_is, TaskArgument(), arg_map);
launcher.add_region_requirement(
    RegionRequirement(input_lp, /*projection_id=*/0, READ_ONLY, EXCLUSIVE, input_lr));
launcher.region_requirements[0].add_field(FID_X);
```
Each point task `i` receives `input_lp[i]` — the subregion at color `i`.

**A custom projection functor** for a 1D stencil with halo:
```cpp
class StencilProjectionFunctor : public ProjectionFunctor {
public:
  LogicalRegion project(const Mappable *m, unsigned idx,
                        LogicalPartition upper_bound,
                        const DomainPoint &p) override {
    // For point P, return the subregion that includes P's neighbors.
    return runtime->get_logical_subregion_by_color(upper_bound,
        DomainPoint(p[0] + 1));  // shift to the right neighbor
  }
  bool is_functional() const override { return true; }
};
Runtime::preregister_projection_functor(STENCIL_PROJ_ID, new StencilProjectionFunctor());
```

**Registration**:
```cpp
Runtime::preregister_projection_functor(MY_PROJ_ID, new MyProjectionFunctor());
```

Use it on the requirement:
```cpp
RegionRequirement(input_lp, /*projection_id=*/MY_PROJ_ID, ...);
```

**Built-in projection IDs** (current versions of Legion add a few):
- `0` — identity. Default and the most common.
- A handful of variants for affine shifts (used for multi-region stencils with halo regions).

## Invariants
- Projection functors are **pure**: same `(point, upper_bound)` input always produces the same output. The runtime caches and reuses results.
- The result must be a **subregion of the requirement's `upper_bound` partition** — otherwise it violates the privilege containment rule (`privilege.md`).
- Projection ID **0 is reserved** for the identity functor; do not register over it.
- A projection functor's `is_functional()` indicates whether it's mappable to a pure compile-time expression; the runtime can optimize functional projections.
- The functor's output **determines the per-point region** the task sees — combine with disjoint partition + per-field requirement for full non-interference (`non-interference.md`).

## Performance implications
- **The standard mechanism for ghost-cell / halo patterns**: aliased partitions + custom projection give stencils their natural data layout without manual neighbor-list construction.
- For data-parallel work where each point only touches "its own" subregion, identity (ID 0) is the right answer and is optimized.
- Complex projection functors that produce many cross-point conflicts can defeat `non-interference.md` — visible as edges between sibling point tasks in `legion-spy.md`'s dataflow graph.
- The functor itself runs in mapper-callback context style — keep it fast (no allocations, no expensive computation).

## Debug signals
- **Legion Spy dataflow graph**: an index launch with a custom projection shows up as one logical node, but its per-point fan-out can be inspected via the event graph (`event-graph.md`).
- **`-DBOUNDS_CHECKS`** catches a projection functor returning a subregion that doesn't include the point's actual accesses.
- **Slow point-task readiness** in `legion-prof.md`: if the functor produces an aliased pattern, point tasks serialize when you expected them to parallelize.

## Failure modes
- A projection functor that returns a region outside the partition's `upper_bound` → privilege containment error at submit time.
- Inconsistent results (functor not pure) → undefined behavior; the runtime caches and reuses.
- Wrong projection (e.g., off-by-one for stencil neighbors) → silent wrong data; catch with bounds checks + careful Spy review.

## Source pointers
- **Legion API**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Tutorial (partition + projection)**: https://legion.stanford.edu/tutorial/partitioning.html
- **Tutorial (index tasks)**: https://legion.stanford.edu/tutorial/index_tasks.html

## Related
- `wiki/concepts/partition.md` — what the functor's `upper_bound` is.
- `wiki/concepts/index-space-launch.md` — the launcher that uses the functor.
- `wiki/concepts/region-requirement.md` — where `projection_id` is set.
- `wiki/concepts/logical-region.md` — what the functor returns.
- `wiki/concepts/non-interference.md` — why projection choice matters for parallelism.

---
title: Physical Instance
slug: physical-instance
summary: A concrete, typed buffer in a specific memory that materializes some subset of fields of some subregion; the actual storage backing a logical region.
tags: [data-model, memory, instances, for-perf-debug, for-program-reasoning]
subsystem: legion
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/website-pages/mapper.md
  - raw/website-pages/profiling.md
  - raw/tutorials/06_physical_regions.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion_mapping.h
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
related:
  - wiki/concepts/logical-region.md
  - wiki/concepts/mapper.md
  - wiki/concepts/event.md
  - wiki/concepts/legion-prof.md
  - wiki/concepts/physical-analysis.md
  - wiki/concepts/equivalence-set.md
  - wiki/concepts/region-instance.md
  - wiki/concepts/dma-system.md
  - wiki/concepts/map-task.md
  - wiki/concepts/virtual-mapping.md
  - wiki/concepts/instance-layout.md
  - wiki/concepts/reduction-instance.md
  - wiki/concepts/garbage-collection.md
  - wiki/concepts/memory-manager.md
---

## TL;DR
A physical instance is the actual block of memory that stores data named by a logical region. The mapper chooses *which memory* (system RAM, GPU framebuffer, zero-copy, registered DMA), *which fields* it holds, *which subregion* it covers, and *what layout* (AOS/SOA, dimension order). The runtime then issues whatever copies are needed to keep the instance valid for the tasks that use it. The confusion: a logical region can be backed by zero, one, or many simultaneous physical instances — they appear/disappear as the mapper and the garbage collector decide.

## Mental model
Logical regions are like SQL views or table names; physical instances are like materialized views — concrete, typed buffers that exist (and consume memory) until the runtime collects them. The mapper is the storage planner: it decides whether the same data is mirrored on the CPU and the GPU, or kept only where the next task needs it.

## Mechanism & API
Instances are created by the mapper inside `map_task`, `premap_task`, or `map_inline` via:
```cpp
LayoutConstraintSet constraints;
constraints.add_constraint(SpecializedConstraint(AFFINE_SPECIALIZE));
constraints.add_constraint(FieldConstraint(field_set, /*contig=*/false));
constraints.add_constraint(OrderingConstraint(dims, /*contig=*/false));
constraints.add_constraint(MemoryConstraint(target_memory.kind()));

PhysicalInstance inst;
bool created;
runtime->find_or_create_physical_instance(ctx, target_memory, constraints,
    std::vector<LogicalRegion>{lr}, inst, created);
```

Inside a task body, instances are reached via the `regions` parameter — each `PhysicalRegion` exposes typed `FieldAccessor`s:
```cpp
const FieldAccessor<READ_ONLY, double, 1> acc(regions[0], FID_X);
acc[point]; // double load
```

Layouts:
- **AOS** (`AFFINE_SPECIALIZE` + interleaved field order): struct-of-arrays-of-points → array-of-structs.
- **SOA** (typical default): separate buffer per field, contiguous in point order.
- Dimension order: `OrderingConstraint({DIM_X, DIM_Y, DIM_Z, ...})`.

Other instance kinds:
- **Reduction instance**: holds partial sums for `REDUCE`-privilege accesses, folds at the end.
- **Virtual instance**: created when a task is *virtually mapped* — no buffer, only privilege transfer. Inner tasks can be virtually mapped to defer instance creation to their leaves.
- **External resource**: backed by an attached file (HDF5) or user-supplied pointer (paper `hipc2017.pdf`).

## Invariants
- A physical instance is tied to **one memory**; it cannot be moved (the runtime issues a copy to make another instance elsewhere).
- An instance covers some subregion and some field subset — the layout constraints determine both extent and order.
- The runtime **garbage-collects** instances whose region tree subtree has no future users (debug with `-DLEGION_GC`).
- `find_or_create_physical_instance` will reuse an existing valid instance with compatible constraints, or create a new one.
- A virtually mapped task does **not** materialize an instance — only its subtasks do, when they map.
- `WRITE_DISCARD` privilege lets the runtime **skip the init copy** from a prior valid instance — the instance is allocated fresh.

## Performance implications
- **Instance lifecycle dominates memory pressure**. Too many simultaneously-live instances → `-ll:csize`/`-ll:fsize` exhausted → out-of-memory mid-run.
- **Instance fragmentation**: many small instances per region cause GC churn and reduce DMA throughput. Reuse instances across tasks.
- Layout choice matters for kernels: AOS for irregular access, SOA for vectorized leaf tasks.
- Cross-memory placements (e.g., GPU task reading a SYSTEM_MEM instance) force a DMA visible as a channel-row bar in `legion-prof.md`.
- The `-DFULL_SIZE_INSTANCES` flag forces top-level-region-sized instances — useful for catching bounds bugs, terrible for perf.

## Debug signals
- **Legion Prof memory rows**: each row shows physical instances on that memory over time. Watch for many short-lived instances (churn) or one giant always-live instance (leak).
- **Legion Prof channel rows**: a bar means a copy was issued — every channel bar costs latency, and the source/target memories are visible.
- **`-DLEGION_GC`**: log every instance allocation/collection; analyze with `tools/legion_gc.py`.
- **`-DTRACE_ALLOCATION`**: log all runtime allocations including instance-side metadata.
- **Out-of-memory errors**: usually mean too-large or too-many instances; lower the partition count or raise `-ll:csize`/`-ll:fsize`.

## Failure modes
- [Instance fragmentation](../pitfalls/instance-fragmentation.md) — many small instances per region.
- [Excessive data movement](../pitfalls/excessive-data-movement.md) — instances placed far from compute.
- [GPU underutilization](../pitfalls/gpu-underutilization.md) — task is on GPU but instance is on CPU.

## Source pointers
- **Mapping API**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion_mapping.h
- **Accessors**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Tutorial (physical regions)**: https://legion.stanford.edu/tutorial/physical_regions.html
- **Paper (external resources)**: `raw/publications/pdfs/hipc2017.pdf`

## Related
- `wiki/concepts/logical-region.md` — the name that an instance materializes.
- `wiki/concepts/mapper.md` — creates and selects instances.
- `wiki/concepts/event.md` — the readiness signal for an instance.
- `wiki/concepts/legion-prof.md` — memory + channel rows show instance lifecycle.

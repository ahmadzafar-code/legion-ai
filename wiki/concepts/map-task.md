---
title: map_task
slug: map-task
summary: The mapper's most consequential callback; picks the task variant, the chosen physical instances for each region requirement, the target processor(s), task priority, and output instance targets.
tags: [mapping, instances, memory, for-perf-debug]
subsystem: legion
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/website-pages/mapper.md
  - raw/tutorials/10_custom_mappers.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion_mapping.h
related:
  - wiki/concepts/mapper.md
  - wiki/concepts/mapper-callback.md
  - wiki/concepts/mapper-context.md
  - wiki/concepts/physical-instance.md
  - wiki/concepts/task-variant.md
  - wiki/concepts/realm-machine-model.md
  - wiki/concepts/select-task-options.md
  - wiki/concepts/instance-layout.md
---

## TL;DR
`map_task` is *the* callback. The runtime gives the mapper a task that's ready to run, a list of currently-valid physical instances per region requirement, and the mapper returns: which variant to execute, which instances (existing or newly created) to use, which processor(s) the task can run on, what priority. Most custom mappers exist to override `map_task`. The confusion: `map_task` does not *create* instances directly — it calls `MapperRuntime::find_or_create_physical_instance` with layout constraints, and the runtime returns a handle. The mapper's job is choosing *constraints*, not allocating memory.

## Mental model
`map_task` is the storage-and-placement planner. Given: the chosen processor (from `select-task-options.md`), the currently-valid copies of each input, and the application's region requirements. Decide: which valid copies to reuse, which new instances to allocate (where and in what layout), which CPU/GPU variant to dispatch. The mapper makes the strategy; the runtime executes the mechanics (allocation, copies, dependencies).

## Mechanism & API
Signature:
```cpp
void map_task(const MapperContext ctx,
              const Task &task,
              const MapTaskInput &input,
              MapTaskOutput &output);
```

**Input** (`MapTaskInput`):
- `input.valid_instances` — per region requirement: the set of currently-valid physical instances. May be empty if `select_task_options::valid_instances` was `false`.
- `input.premapped_regions` — indices of region requirements already mapped by an earlier `premap_task`.

**Output** (`MapTaskOutput`):
- `output.chosen_instances` — per region requirement, the physical instances to use. Either reused from `input.valid_instances` or newly created via `MapperRuntime::find_or_create_physical_instance`.
- `output.chosen_variant` — the `VariantID` to execute on `target_procs`.
- `output.target_procs` — set of processors that can execute this task (the runtime picks one; the mapper can suggest several for load-balancing).
- `output.postmap_task` — set to `true` to receive a `postmap_task` callback (useful for prefetching copies into other memories).
- `output.task_priority` — scheduling priority (higher = sooner).
- `output.output_targets` — for output regions, the memory in which to create instances.

**Creating an instance** (the body of most custom `map_task` overrides):
```cpp
LayoutConstraintSet constraints;
constraints.add_constraint(SpecializedConstraint(AFFINE_SPECIALIZE));
constraints.add_constraint(FieldConstraint(task.regions[0].privilege_fields, /*contig=*/false));
constraints.add_constraint(OrderingConstraint(dims, /*contig=*/false));
constraints.add_constraint(MemoryConstraint(target_memory.kind()));

PhysicalInstance inst;
bool created;
runtime->find_or_create_physical_instance(
    ctx, target_memory, constraints,
    std::vector<LogicalRegion>{task.regions[0].region},
    inst, created);

output.chosen_instances[0].push_back(inst);
```

**Selecting a variant compatible with the target processor**:
```cpp
std::vector<VariantID> variants;
runtime->find_valid_variants(ctx, task.task_id, variants);
for (auto vid : variants) {
  const ExecutionConstraintSet &c = runtime->find_execution_constraints(ctx, task.task_id, vid);
  if (c.processor_constraint.can_use(target_proc.kind())) {
    output.chosen_variant = vid;
    break;
  }
}
```

## Invariants
- The chosen `target_procs` must contain at least one processor; **all chosen processors must be compatible** with the chosen variant's `ProcessorConstraint`.
- Every region requirement must have at least one `chosen_instance` (unless `virtual_map` was set on the requirement).
- Reusing an instance from `input.valid_instances` is safe and preferred; creating a new one when a compatible valid one exists wastes memory and adds allocation cost.
- `find_or_create_physical_instance` returns a handle even when allocation **fails** (returns `false` for success); always check the return.
- The chosen instances must satisfy the region requirement's privilege at access time — the runtime checks under `-DPRIVILEGE_CHECKS`.
- All `mapper-callback.md` rules apply: non-blocking, non-reentrant by default, `mapper-context.md` valid only inside this call.

## Performance implications
- **The mapper's single biggest perf influence.** Instance placement (`target_memory`) and variant selection (`chosen_variant`) directly drive `legion-prof.md` channel-row activity and processor-row utilization.
- **Reuse valid instances** from `input.valid_instances` aggressively — fresh allocations are expensive and trigger DMA to populate.
- **Memory choice matters**: use `proc_mem_affinity` queries to pick a memory close to `target_proc`. Cross-memory placements force copies.
- **Variant choice matters**: pick a GPU variant for `TOC_PROC` placement, CPU variant for `LOC_PROC`. Missing variants → silent CPU fallback → `pitfalls/gpu-underutilization.md`.
- **Layout choice matters**: AOS for "per-point all-fields" iterations, SOA for "per-field across-points". The wrong layout makes leaf kernels slow.

## Debug signals
- **`LoggingWrapper` + `-level mapper=2`**: every `map_task` call logs `chosen_instances`, `chosen_variant`, `target_procs`. First place to look when output doesn't match expectations.
- **Legion Prof channel rows**: heavy activity = `map_task` is choosing memories far from compute. Inspect `target_memory` selection.
- **Legion Prof GPU rows empty**: variant selection sent the task to CPU; check the variant-filtering loop.
- **Out-of-memory in `find_or_create_physical_instance`**: bump `-ll:csize`/`-ll:fsize`, or reuse instances more aggressively, or coarsen partitions.

## Failure modes
- [GPU underutilization](../pitfalls/gpu-underutilization.md) — variant or instance memory chose the wrong kind.
- [Excessive data movement](../pitfalls/excessive-data-movement.md) — instances placed in memories far from compute.
- [Mapper bouncing](../pitfalls/mapper-bouncing.md) — unstable `target_procs` across iterations.
- [Instance fragmentation](../pitfalls/instance-fragmentation.md) — fresh allocations per call instead of reuse.

## Source pointers
- **Header**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion_mapping.h
- **Reference**: `raw/website-pages/mapper.md`
- **Tutorial**: `raw/tutorials/10_custom_mappers.md`

## Related
- `wiki/concepts/mapper.md` — host.
- `wiki/concepts/mapper-callback.md` — callback model.
- `wiki/concepts/mapper-context.md` — `ctx`.
- `wiki/concepts/physical-instance.md` — what this callback creates/selects.
- `wiki/concepts/task-variant.md` — what `chosen_variant` selects from.
- `wiki/concepts/realm-machine-model.md` — what processors/memories the mapper queries.
- `wiki/concepts/select-task-options.md` — the prior callback.

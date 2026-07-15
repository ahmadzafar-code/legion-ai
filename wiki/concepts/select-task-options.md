---
title: select_task_options
slug: select-task-options
summary: The first mapper callback for any task; runs before dependence analysis and sets initial properties (target processor, inline/stealable/replicate/memoize flags) that shape the task's path through the pipeline.
tags: [mapping, execution, for-perf-debug, for-program-reasoning]
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
  - wiki/concepts/slice-task.md
  - wiki/concepts/map-task.md
  - wiki/concepts/tracing.md
---

## TL;DR
`select_task_options` is the **first** callback the mapper sees for every task — before dependence analysis even runs. The output sets initial processor, inline/stealable/replicate flags, and crucially the **`memoize` flag** that opts the task into trace memoization. Get this wrong and tracing never kicks in. The confusion: this is the only callback fired before stage 2 of the operation pipeline, so it cannot consult information that only emerges later (which instances are valid, what regions look like in physical form). It's purely "what kind of task is this and where should I send it first?"

## Mental model
`select_task_options` is the dispatcher's first triage: "look at the task ID and the launcher tag, decide whether to send this to a GPU, replicate it, allow stealing, and most importantly whether to enable memoization." Everything else is shaped by what this callback says.

## Mechanism & API
Signature:
```cpp
void select_task_options(const MapperContext ctx,
                         const Task &task,
                         SelectTaskOptionsOutput &output);
```

**Input** (`const Task &task`):
- `task.task_id` — registered task ID.
- `task.regions` — region requirements (raw, pre-mapping).
- `task.arglen`, `task.args` — launcher payload.
- `task.tag` — mapper-visible application hint.
- `task.index_point` — for index tasks, this point.
- `task.parent_task` — the parent context.

**Output fields**:
- `output.initial_proc` — processor to send the task to for mapping. Default: local processor.
- `output.inline_task` — inline into the parent (skip launching). Default: `false`.
- `output.stealable` — other mappers may steal this task. Default: `false`.
- `output.map_locally` — perform mapping on the current node even if `initial_proc` is remote. Default: `false`.
- `output.valid_instances` — set to `false` to skip the "valid instances" preamble in `map_task`. Default: `true`.
- `output.memoize` — **enable tracing memoization for this task**. Default: `false` for stock `Mapper`, **`true` for `DefaultMapper`**. This is the critical knob for `tracing.md`.
- `output.replicate` — enable replication for resilience. Default: `false`.
- `output.parent_priority` — initial scheduling priority.

**Common pattern** — steer GPU-eligible tasks to GPU processors:
```cpp
void MyMapper::select_task_options(const MapperContext ctx,
                                    const Task &task,
                                    SelectTaskOptionsOutput &out) {
  DefaultMapper::select_task_options(ctx, task, out);   // inherit defaults
  if (task.task_id == GPU_TASK_ID) {
    Machine::ProcessorQuery pq(machine); pq.only_kind(Processor::TOC_PROC);
    out.initial_proc = pq.first();
  }
}
```

## Invariants
- `select_task_options` runs **before logical analysis (stage 2)** — region instances do not exist yet; the callback cannot query physical state.
- The callback runs **exactly once per task** (including each point of an index launch — though typically `slice_task` is the per-point hook).
- The `initial_proc` is a **hint**: subsequent callbacks may relocate the task. It controls where mapping happens, not necessarily where execution happens.
- `memoize = true` is **required** for the task to participate in any form of tracing — explicit, dynamic, or automatic.
- All callbacks remain **non-blocking and non-reentrant** per default mapper rules (`mapper.md`).

## Performance implications
- **The #1 perf knob in this callback is `memoize`.** Forgetting it defeats tracing entirely; see `pitfalls/missed-tracing-opportunity.md`.
- `initial_proc` steers GPU-eligible work to GPU processors — a common fix for `pitfalls/gpu-underutilization.md`.
- `map_locally = true` reduces inter-node messaging for tasks where mapping decisions are cheap and the data is local.
- `inline_task = true` is rarely beneficial; it eliminates parallelism for that task. Use only for trivial dispatcher tasks.

## Debug signals
- **`LoggingWrapper`** shows the `output` of every `select_task_options` call. Mismatches between expected and actual `initial_proc` / `memoize` are the most common debugging finding here.
- **Tracing not working** despite `begin_trace`/`end_trace` markers → check `memoize` is `true` in the mapper log.
- **GPU rows empty in Legion Prof** despite GPU variants existing → `initial_proc` probably points at a CPU.

## Failure modes
- [Missed tracing opportunity](../pitfalls/missed-tracing-opportunity.md) — `memoize=false`.
- [GPU underutilization](../pitfalls/gpu-underutilization.md) — `initial_proc` set wrong.

## Source pointers
- **Header**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion_mapping.h
- **Reference**: `raw/website-pages/mapper.md`
- **Tutorial**: `raw/tutorials/10_custom_mappers.md`

## Related
- `wiki/concepts/mapper.md` — host concept.
- `wiki/concepts/mapper-callback.md` — callback model.
- `wiki/concepts/mapper-context.md` — the `ctx` argument.
- `wiki/concepts/slice-task.md` — next callback for index launches.
- `wiki/concepts/map-task.md` — the next callback for single tasks.
- `wiki/concepts/tracing.md` — gated by `memoize`.

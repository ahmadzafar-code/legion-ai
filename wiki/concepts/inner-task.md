---
title: Inner Task
slug: inner-task
summary: A task variant declared with `set_inner(true)`; promises that the task launches subtasks/sub-operations but does NOT read or write region-instance data directly, enabling the runtime to defer physical-instance creation via virtual mapping.
tags: [execution, configuration, for-perf-debug]
subsystem: legion
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/04_hybrid_model.md
  - raw/website-pages/mapper.md
  - raw/youtube_transcripts/runtime_school_2023/transcripts/002_Legion_Runtime_Internals_-_Lesson_2_-_Tasks_Context_and_Forward_Progress.txt
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
related:
  - wiki/concepts/task.md
  - wiki/concepts/task-variant.md
  - wiki/concepts/leaf-task.md
  - wiki/concepts/replicable-task.md
  - wiki/concepts/virtual-mapping.md
  - wiki/concepts/operation-pipeline.md
---

## TL;DR
An inner task is a task variant declared with `set_inner(true)`. It promises that the body **launches subtasks (or other sub-operations) but does not directly read or write the data inside any of its region requirements** — all data access happens in subtasks. In return the runtime allows the mapper to **virtually map** the inner task's regions: no physical instance is created at the inner-task's level, only at its subtasks'. The confusion: inner is the dual of `leaf-task.md` (leaf = no sub-ops, full data access; inner = sub-ops only, no direct data access). The "regular" task — one that both reads data *and* launches subtasks — has neither flag.

## Mental model
Inner tasks are *dispatchers* — control-flow tasks that decide what to launch next based on tunable variables, futures, or partition shapes, but never look at the contents of a region. Where `leaf-task.md` is the "do the work" leaf of a Legion call tree, inner is the branch nodes. The optimization is straightforward: the mapper doesn't need to materialize an instance just for the inner task to pass it through to subtasks; the subtasks will materialize when they actually need it.

## Mechanism & API
Set on the registrar:
```cpp
TaskVariantRegistrar reg(STENCIL_DRIVER_TASK_ID, "stencil_driver");
reg.add_constraint(ProcessorConstraint(Processor::LOC_PROC));
reg.set_inner(true);
Runtime::preregister_task_variant<stencil_driver>(reg, "stencil_driver");
```

What the runtime allows when `inner = true`:
- The inner task may launch any sub-operations (`execute_task`, `execute_index_space`, copies, fills, partitioning operations).
- The mapper may set the inner task's region requirements to **virtually mapped** (no physical instance backing). See `virtual-mapping.md`.

What the inner task **must not** do:
- Construct a `FieldAccessor` on any `regions[i]` — there's no instance to access.
- Pass instance pointers to native code that dereferences them.

The Runtime School Lesson 2 transcript distinguishes the **inner context** (the context kind for non-leaf tasks) from the **leaf context** (the optimized variant). An inner task with `set_inner(true)` runs under an inner context, just like a regular task — the flag is additional information about *what the body does*, not a different context class.

## Invariants
- An inner task's body **must not** access region-instance data through any accessor.
- The runtime does **not** dynamically check inner-violation; misuse is undefined behavior in release builds.
- Inner and leaf are **mutually exclusive** flags on a single variant.
- An inner task **can launch subtasks of any kind**: leaf, inner, replicable, or regular.
- A region requirement marked virtual on an inner task's launch produces **no instance at this level** — subtasks see the same region but must map it themselves.
- Privileges still propagate normally: subtasks of an inner task have their privileges checked against the inner task's declared privileges (subset rule, `privilege.md`).

## Performance implications
- **The big win is virtual mapping.** An inner driver task with virtually-mapped regions costs essentially zero allocation; its subtasks make placement decisions in context of where they will actually run.
- Inner tasks are the natural form for top-level orchestrators that read tunable values, dispatch index launches, and pass futures around without ever touching field data.
- The leaf-context fast path (`leaf-task.md`) does **not** apply — inner tasks pay normal-context overhead.
- Mapping an inner task is **cheaper** than a regular task: with virtual mapping there are no instances to create and no copies to schedule.

## Debug signals
- **`LoggingWrapper`** shows `set_inner` in registered variants; mapper logs show `virtual_map = true` on requirements.
- **Crash inside an inner-task body** with a `FieldAccessor` constructor is the typical "I added `set_inner` but my code still reads data" mistake.
- **Legion Prof**: an inner task's bar should be brief (just launching) and instance/channel rows during it should be quiet.

## Failure modes
- Calling a `FieldAccessor` inside an `inner` task → undefined behavior in release builds; debug builds may catch via assertion.
- Setting both `set_leaf(true)` and `set_inner(true)` → registration error.

## Source pointers
- **Legion API**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Reference**: `raw/website-pages/mapper.md` (virtual mapping section)
- **Lecture**: `raw/youtube_transcripts/runtime_school_2023/transcripts/002_..._Tasks_Context_and_Forward_Progress.txt`

## Related
- `wiki/concepts/task.md` — host concept.
- `wiki/concepts/task-variant.md` — where `set_inner` is set.
- `wiki/concepts/leaf-task.md` — the dual.
- `wiki/concepts/replicable-task.md` — the third task-property flag.
- `wiki/concepts/virtual-mapping.md` — what `set_inner` unlocks.
- `wiki/concepts/operation-pipeline.md` — inner tasks run under inner contexts.

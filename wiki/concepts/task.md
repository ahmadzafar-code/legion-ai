---
title: Task
slug: task
summary: The unit of compute in Legion; a functionally-pure procedure declaring its region/field access via region requirements, asynchronously dispatched by the runtime.
tags: [execution, for-program-reasoning]
subsystem: legion
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/website-pages/overview.md
  - raw/tutorials/02_tasks_and_futures.md
  - raw/tutorials/07_privileges.md
  - raw/youtube_transcripts/runtime_school_2023/transcripts/001_Legion_Runtime_Internals_-_Lesson_1_-_The_Operation_Pipeline.txt
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
related:
  - wiki/concepts/logical-region.md
  - wiki/concepts/privilege.md
  - wiki/concepts/mapper.md
  - wiki/concepts/operation-pipeline.md
  - wiki/concepts/event.md
  - wiki/concepts/index-space-launch.md
  - wiki/concepts/future.md
  - wiki/concepts/future-map.md
  - wiki/concepts/regent-language.md
  - wiki/concepts/pygion.md
  - wiki/concepts/task-variant.md
  - wiki/concepts/task-launcher.md
  - wiki/concepts/region-requirement.md
  - wiki/concepts/leaf-task.md
  - wiki/concepts/inner-task.md
  - wiki/concepts/replicable-task.md
---

## TL;DR
A task is a registered C++ function the Legion runtime can execute. It takes by-value arguments, declares which logical regions it touches with what privileges, and returns either a `Future` or a `FutureMap` (for index-space launches). Tasks are *implicitly parallel*: the runtime infers dependencies from region requirements and runs non-interfering tasks concurrently. The confusion: writing tasks that *look* sequential is normal — Legion only serializes pairs whose region requirements actually conflict.

## Mental model
Tasks are like instructions in an out-of-order CPU. Each task carries its own "operand set" (region requirements with privileges) instead of register names. The runtime is the scheduler: it inspects the operand set, identifies real (RAW/WAR/WAW) hazards, and dispatches independent tasks to processors. The application's job is to launch every task it would *like* to run; the runtime decides what order is *actually safe*. The 7-stage operation pipeline (see `operation-pipeline.md`) is where that decision happens.

## Mechanism & API
- **Registration** (once per task ID): `TaskVariantRegistrar` + `Runtime::preregister_task_variant<...>(registrar, name)`. Register multiple variants per `TaskID` (CPU, GPU, leaf) to give the mapper choices.
- **Launch**: `TaskLauncher` (single task) or `IndexLauncher` (Cartesian-launch over a color/index space, scales to billions of points — see paper `idx2021.pdf`). Both take `RegionRequirement`s, `TaskArgument` payloads, futures to wait on, and a `MapperID`.
- **Execute**: `runtime->execute_task(ctx, launcher)` → returns a `Future`. `runtime->execute_index_space(ctx, launcher)` → returns a `FutureMap`.
- **Inside the task**: arguments via `task->args`/`task->arglen`; per-region instances via the `regions` vector; runtime + context for launching subtasks.

Task properties set on the registrar:
- `set_leaf(true)` — task launches no subtasks. Enables an optimized leaf-context path.
- `set_inner(true)` — task launches subtasks but doesn't directly read instance data.
- `ProcessorConstraint(LOC_PROC|TOC_PROC|...)` — pins variants to processor kinds.

## Invariants
- A subtask's region requirements **must be a subset** of its parent task's privileges; privileges flow only through region requirements (no global ambient privilege).
- A task sees a **consistent snapshot** of its requested regions at execution time, regardless of how reorderings shake out elsewhere.
- `Future` is **not permitted** as a task return type. Pass futures by adding them with `launcher.add_future(f)` instead.
- A leaf task **must not** launch subtasks; if it tries, the runtime errors (`raw/youtube_transcripts/runtime_school_2023/.../Lesson_1_...txt`).
- Blocking on a future (`future.get_result<T>()`) **does not stall the processor** — other mapped tasks continue running on it.

## Performance implications
- **Index launches scale**; iterated `TaskLauncher`s in a loop create one runtime operation per call. For data-parallel work always prefer `IndexLauncher`.
- **Task granularity matters**: too fine → runtime overhead dominates; too coarse → no parallelism. Profile with `legion-prof.md` to see where you sit.
- **Maximize launches before blocking** (`get_result`). Each blocking call pauses *only* the issuing task; pull all blockers to the end of the parent task body.
- Task variants can target multiple processor kinds; missing GPU variants force CPU placement and explain "GPU sat idle" surprises.

## Debug signals
- **Legion Prof**: each task instance is a colored bar on its processor row. Gaps before a task = data movement or unsatisfied dependency.
- **Legion Spy** (`-lg:spy` → `legion_spy.py -dez`): the dataflow graph shows tasks as nodes, region requirements as edges; trace upward to find what triggered each scheduling decision.
- **Mapper log** (`LoggingWrapper` + `-level mapper=2`): every callback the mapper made for this task, including the variant chosen and the chosen processor.

## Failure modes
- [Long dependence chains](../pitfalls/long-dependence-chains.md) — sequential `execute_task` loops without index launches.
- [Missed tracing opportunity](../pitfalls/missed-tracing-opportunity.md) — repeated loop bodies not wrapped in `begin_trace`/`end_trace`.

## Source pointers
- **Header**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Tutorial (single tasks + futures)**: https://legion.stanford.edu/tutorial/tasks_and_futures.html
- **Tutorial (index launches)**: https://legion.stanford.edu/tutorial/index_tasks.html
- **Paper (index launches)**: `raw/publications/pdfs/idx2021.pdf`

## Related
- `wiki/concepts/logical-region.md` — what tasks reach into.
- `wiki/concepts/privilege.md` — how the runtime extracts parallelism between tasks.
- `wiki/concepts/mapper.md` — where each task instance ends up running.
- `wiki/concepts/operation-pipeline.md` — the 7 stages every task flows through.
- `wiki/concepts/event.md` — what `Future` is built on at the Realm layer.

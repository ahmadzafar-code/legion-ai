---
title: Leaf Task
slug: leaf-task
summary: A task variant declared with `set_leaf(true)`; promises to launch no subtasks or sub-operations, in exchange for the runtime's optimized leaf-context fast path.
tags: [execution, configuration, for-perf-debug]
subsystem: legion
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/02_tasks_and_futures.md
  - raw/tutorials/04_hybrid_model.md
  - raw/youtube_transcripts/runtime_school_2023/transcripts/002_Legion_Runtime_Internals_-_Lesson_2_-_Tasks_Context_and_Forward_Progress.txt
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
related:
  - wiki/concepts/task.md
  - wiki/concepts/task-variant.md
  - wiki/concepts/operation-pipeline.md
  - wiki/concepts/regent-demand-directive.md
  - wiki/concepts/inner-task.md
  - wiki/concepts/replicable-task.md
---

## TL;DR
A leaf task is a task variant whose registrar carries `set_leaf(true)`. It promises the runtime that the task body **does not launch subtasks, copies, or any other operations** — it just computes and returns. In exchange, the runtime executes the task under a **leaf context**, a simplified per-task state object that skips bookkeeping for sub-ops. Two sibling annotations: `set_inner(true)` (the task launches subtasks but doesn't read region instances directly, enabling virtual mapping) and `set_replicable(true)` (the task is safe for control replication). The confusion: leaf is purely a performance hint backed by a runtime check — if a leaf task ever launches a sub-op, the runtime errors out. There's no silent fallback.

## Mental model
Leaf is `__attribute__((leaf))` for tasks — same idea as the GCC function attribute: "this function makes no calls back into the caller's framework". The runtime can then strip the framework's per-call state. Inner is the dual: "this is just a dispatcher, the work happens in my children, so don't bother materializing my data". Replicable is the opt-in for control replication's SPMD execution.

## Mechanism & API
Set on the registrar at task registration time:
```cpp
TaskVariantRegistrar reg(TASK_ID, "stencil");
reg.add_constraint(ProcessorConstraint(Processor::LOC_PROC));
reg.set_leaf(true);     // leaf task: no sub-ops
Runtime::preregister_task_variant<stencil_impl>(reg, "stencil");
```

Other relevant setters:
- `set_inner(true)` — inner task. May launch subtasks/sub-ops but cannot directly read or write region-instance data. Enables **virtual mapping** for its region requirements: the runtime defers physical-instance creation to the subtasks. The Runtime School Lesson 2 distinguishes **inner context** (the normal kind, capable of launching sub-ops) from **leaf context** (the optimized variant).
- `set_replicable(true)` — the task is suitable for control replication: deterministic given inputs, no side effects that would diverge across shards. Required for top-level tasks that scale across nodes.
- `set_idempotent(true)` — the task can be re-executed safely without observable side effects; enables certain resiliency optimizations.

A task may be **both leaf and replicable** (a leaf kernel re-run on every shard). It cannot be both leaf and inner (mutually exclusive).

Inside a leaf task, the `Context` is a leaf context. Operations that try to launch sub-ops (`runtime->execute_task`, `runtime->create_index_space`, etc.) error out at runtime with messages like "cannot launch sub task in leaf context".

## Invariants
- A `set_leaf(true)` task **must not** launch sub-operations at runtime. The runtime checks at every API call inside the leaf context and errors if it does.
- A `set_inner(true)` task **must not** directly access region-instance data (no `FieldAccessor` use in its body); it can only delegate to subtasks.
- Leaf and inner are **mutually exclusive** on a single variant.
- All variants of a task ID **should agree** on leaf/inner properties — though Legion technically allows mixing, the convention is that all variants of a task are either leaf or non-leaf.
- A replicable task **must be deterministic** given its logical inputs across all shards (`control-replication.md`).
- Properties are **per-variant**, set on `TaskVariantRegistrar` — different variants of the same TaskID can have different leaf flags (rare, but legal).

## Performance implications
- **Leaf is a real perf win for fine-grained tasks.** The optimized leaf-context path skips much of the per-task bookkeeping that the normal inner context maintains. For tasks that execute in microseconds, this can be 2-3× the wall-clock improvement.
- **Inner unlocks virtual mapping.** An inner task can have region requirements with no physical instance — the subtasks materialize instances when they actually need them. Useful for "control flow" tasks that just decide what to launch.
- **Replicable is required for multi-node scaling** of the top-level task; without it, control replication can't be applied.
- Forgetting `set_leaf` on an obvious leaf task is a common silent perf loss; the program still runs, just slower.

## Debug signals
- **Runtime error "cannot launch sub task in leaf context"** = a `set_leaf(true)` task tried to call `execute_task` or similar. Either remove `set_leaf` or refactor the body.
- **Legion Prof**: leaf-context tasks have measurably less per-call overhead than non-leaf siblings; comparing profiles before/after adding `set_leaf` quantifies the win.
- **Mapper logs**: variant selection logs include the leaf/inner flags of the chosen variant.

## Failure modes
- A leaf task that "needs to" launch a subtask in one rare branch can't easily be retrofitted; remove `set_leaf` and accept the overhead.
- An inner task accidentally reading region-instance data → undefined behavior (no runtime check).

## Source pointers
- **Legion API**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Tutorial (set_leaf example)**: https://legion.stanford.edu/tutorial/tasks_and_futures.html
- **Lecture**: `raw/youtube_transcripts/runtime_school_2023/transcripts/002_..._Tasks_Context_and_Forward_Progress.txt`

## Related
- `wiki/concepts/task.md` — host concept.
- `wiki/concepts/task-variant.md` — where the flag is set.
- `wiki/concepts/operation-pipeline.md` — the leaf-context fast path lives here.
- `wiki/concepts/regent-demand-directive.md` — `__demand(__leaf)` is Regent's enforcer.

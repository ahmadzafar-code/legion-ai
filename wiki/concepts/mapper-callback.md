---
title: Mapper Callback
slug: mapper-callback
summary: The unifying interface for all mapper methods; non-blocking, default-serialized per mapper instance, receives an opaque `MapperContext` plus per-callback input/output structs.
tags: [mapping, configuration, for-program-reasoning]
subsystem: legion
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/website-pages/mapper.md
  - raw/youtube_transcripts/runtime_school_2023/transcripts/003_Legion_Runtime_Internals_-_Lesson_3_-_Scheduling_and_Mapper_Calls.txt
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion_mapping.h
related:
  - wiki/concepts/mapper.md
  - wiki/concepts/mapper-context.md
  - wiki/concepts/select-task-options.md
  - wiki/concepts/slice-task.md
  - wiki/concepts/map-task.md
  - wiki/concepts/default-mapper.md
---

## TL;DR
Every mapper method is a callback the runtime invokes at a specific pipeline point. All callbacks share a consistent shape: `void callback(const MapperContext ctx, const InputStruct &in, OutputStruct &out)`. By default the runtime **serializes callbacks per mapper instance** — only one callback runs at a time on a given mapper. Callbacks **must not block** on long-running work. Mapper events let a callback defer a decision pending another callback's result without blocking. The confusion: "callback" suggests the mapper is reactive, but several callbacks **also produce active side-effects** — `map_task` creates instances; `select_steal_targets` initiates steal requests.

## Mental model
The mapper-callback interface is like the OS scheduler's hook table: the kernel (runtime) calls the policy (mapper) at specific decision points, the policy fills in the answer, the kernel proceeds. The contract is "answer fast, don't block, leave shared state consistent."

## Mechanism & API
Generic callback signature:
```cpp
void MyMapper::callback_name(const MapperContext ctx,
                             const CallbackInput &input,
                             CallbackOutput &output);
```

Three universal pieces:
- `ctx` — opaque handle, valid only inside this callback. See `mapper-context.md`.
- `input` — `const` reference to the runtime's per-callback information bundle.
- `output` — mutable reference; the mapper fills it in with its decisions.

**Callback categories**:
- **Task launch lifecycle**: `select_task_options` → `slice_task` (index) / `premap_task` → `select_tasks_to_map` → `map_task` → `postmap_task`. See the named callback pages.
- **Other operations**: `map_inline` (explicit `InlineMapping`), `map_copy` (explicit copy operations), `map_must_epoch` (must-epoch launches).
- **Variants and tunables**: `select_variant`, `select_tunable_value`.
- **Load balancing**: `select_steal_targets`, `permit_steal_request`.
- **Replication**: `select_sharding_functor`.
- **Profiling**: `report_profiling`.
- **Speculation**: `speculate`.
- **Cross-mapper communication**: `handle_message`.

**Concurrency model**:
- By default `concurrent = false`: the runtime serializes callbacks per mapper instance. No application-level synchronization needed for mapper-private state.
- Set `concurrent = true` in the constructor to opt into concurrent callbacks. Requires application-side locking of shared state via `MapperLock`.
- **Mapper instances are per-processor.** Different processors' mappers run independently; cross-mapper state requires explicit message passing.

**Synchronization primitives**:
- `MapperLock` — created via `runtime->create_mapper_lock(ctx)`. Used between concurrent callbacks on the same mapper instance.
- `MapperEvent` — for one callback to wait on another callback's completion: `create_mapper_event` → some other callback triggers it → `wait_on_mapper_event`.
- `runtime->send_message(ctx, target_proc, msg, size)` + `handle_message` — inter-mapper RPC.

**Non-blocking rule**: callbacks must not perform long-running operations. The runtime relies on prompt responses; a slow callback stalls the pipeline (`pitfalls/mapper-stalls.md`). Mapper events are the right way to defer a decision pending future information without blocking.

## Invariants
- Every callback receives a fresh `MapperContext`; reusing it across callbacks is undefined.
- Callbacks **must not block** on I/O, sleep, or other long operations. `wait_on_mapper_event` is the only legitimate cross-callback wait.
- Default-concurrency mappers process callbacks one at a time per mapper instance.
- Callbacks may call into the `MapperRuntime` (instance queries, locks, variant lookup) using `ctx`.
- Mappers may have **multiple instances per node** (one per processor); cross-instance state requires explicit synchronization or messaging.
- A callback that fills `output` incompletely → runtime asserts under debug; release builds may have undefined behavior.

## Performance implications
- **Slow callbacks are the most common mapper perf bug.** Cache machine-model query results in the constructor; avoid per-callback allocation; pre-sort processor/memory lists.
- Reentrant callbacks (`concurrent = true`) buy parallelism inside the mapper but require careful locking. Most production mappers stay non-concurrent.
- **Mapper events** are cheaper than reactive re-querying; use them when one callback's answer depends on another's result.
- `report_profiling` is the only callback that does *not* affect a task's execution — it's called after completion to feed profiling data back. Cheap to ignore; expensive to populate fully.

## Debug signals
- **`LoggingWrapper`** wraps any mapper and logs every callback's input/output. Strongly recommended during development.
- **`-level mapper=2`** turns on internal callback timing logs.
- **Long callback duration in `LoggingWrapper` logs** = inefficient callback body; usually heavy `MapperRuntime` queries.
- **Mapper-stall symptoms in Legion Prof**: busy utility rows, idle app rows. See `pitfalls/mapper-stalls.md`.

## Failure modes
- [Mapper stalls](../pitfalls/mapper-stalls.md) — slow callbacks.
- [Mapper bouncing](../pitfalls/mapper-bouncing.md) — unstable callback outputs across iterations.

## Source pointers
- **Mapper API header**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion_mapping.h
- **Reference**: `raw/website-pages/mapper.md`
- **Lecture (scheduler/mapper calls)**: `raw/youtube_transcripts/runtime_school_2023/transcripts/003_..._Scheduling_and_Mapper_Calls.txt`

## Related
- `wiki/concepts/mapper.md` — host concept.
- `wiki/concepts/mapper-context.md` — the universal first argument.
- `wiki/concepts/select-task-options.md`, `wiki/concepts/slice-task.md`, `wiki/concepts/map-task.md` — the most-overridden callbacks.
- `wiki/concepts/default-mapper.md` — the reference implementation of all callbacks.

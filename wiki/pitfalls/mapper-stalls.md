---
title: Mapper Stalls
slug: mapper-stalls
summary: Slow mapper callbacks block the runtime pipeline at stage 4 (mapping); application processors idle while utility processors are busy running mapper code.
tags: [for-perf-debug, mapping, configuration]
status: draft
created: 2026-05-15
updated: 2026-05-15
related:
  - wiki/concepts/mapper.md
  - wiki/concepts/mapper-callback.md
  - wiki/concepts/operation-pipeline.md
  - wiki/concepts/legion-prof.md
  - wiki/concepts/mapper-logging.md
---

## Symptom

- **Utility-processor rows** (`UTIL_PROC`) in Legion Prof are saturated; **application processor rows** (`LOC_PROC`/`TOC_PROC`) show large idle gaps before each task.
- The **`critical-path.md`** runs through utility rows — the bottleneck is the runtime making mapping decisions, not the app doing work.
- Wall-clock scales with the number of operations more than with the work-per-operation; coarsening tasks helps disproportionately.
- **`LoggingWrapper`** + `-level mapper=2` shows individual callbacks taking milliseconds when they should take microseconds.

## Cause

Mapper callbacks must be **non-blocking and fast** (`mapper-callback.md`). When they aren't, the runtime stalls at stage 4 of the operation pipeline (`operation-pipeline.md`) because every operation needs to be mapped before it can execute. Common offenders inside `map_task.md` and related callbacks:

1. **Allocations on the hot path**: `new`/`malloc` per call inside `map_task`. Even small allocations add up over millions of operations.
2. **Sorting a large candidate list per call**: e.g., re-sorting all processors by current load on every `map_task`. Sort once in the constructor.
3. **Expensive `Machine` queries that could be cached**: `Machine::ProcessorQuery` walks every processor; doing it once per callback is wasteful. Cache in the constructor.
4. **String operations / `printf` / formatting**: log-heavy mappers shipped to production. `LoggingWrapper` adds known overhead; custom logging can add much more.
5. **Synchronization primitives**: a custom mapper using `MapperLock` heavily, or worse `wait_on_mapper_event` in a tight loop.

A subtler cause: **too few utility processors** for the operation rate. By default Realm creates 1 utility processor per node (`-ll:util 1`); fast mappers and huge operation rates can saturate it.

## Fix

- **Cache machine-model queries in the mapper constructor**:
  ```cpp
  MyMapper::MyMapper(...) : DefaultMapper(...) {
    Machine::ProcessorQuery pq(machine);
    pq.only_kind(Processor::TOC_PROC);
    gpus_.assign(pq.begin(), pq.end());  // cached
  }
  ```
  Use `gpus_` inside callbacks instead of re-querying.

- **Move heavy work out of callbacks**: any "compute the optimal placement" logic that depends only on `Machine` state should run once at startup, not per-task.
- **Profile mapper logic separately**: use `LoggingWrapper`'s per-callback timing in `-level mapper=2` logs. Find the outliers.
- **Increase utility-processor count**: `-ll:util 4` gives the runtime more parallel mapping bandwidth. A band-aid, not a substitute for fast callbacks, but useful when you can't refactor the mapper immediately.
- **Reduce the operation rate**: use index launches (`index-space-launch.md`) instead of loops of `execute_task`. Each index launch is one mapper invocation per slice, not per point.
- **Enable tracing**: once mapping decisions are recorded in a trace, replay (`trace-replay.md`) skips the mapper callbacks for cached templates.

After the fix, the utility-processor row should be sparse and the application rows continuous, with the critical path running through application work — not runtime overhead.

## Underlying concepts

- `wiki/concepts/mapper.md` — the surface being slow.
- `wiki/concepts/mapper-callback.md` — the contract callbacks must obey.
- `wiki/concepts/operation-pipeline.md` — where stalls form (stage 4).
- `wiki/concepts/legion-prof.md` — where the symptom is visible (utility-row activity, gaps on app rows).
- `wiki/concepts/mapper-logging.md` — the diagnostic tool.

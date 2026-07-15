---
title: Future
slug: future
summary: A lightweight, reference-counted handle to a pending task's return value; the Legion-level wrapper around a Realm completion event plus a typed result buffer.
tags: [execution, synchronization, for-program-reasoning]
subsystem: legion
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/02_tasks_and_futures.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
related:
  - wiki/concepts/task.md
  - wiki/concepts/event.md
  - wiki/concepts/future-map.md
---

## TL;DR
A `Future` is what `runtime->execute_task` hands back: a typed, reference-counted handle that *will* hold the task's return value once it completes. You can pass futures into other launchers as data dependencies (`launcher.add_future(f)`) or block on them via `future.get_result<T>()`. Internally a future is a Realm `Event` (`event.md`) plus a buffer that holds the marshalled return value. The confusion: blocking on a future inside a task does not stall the processor — the runtime is free to execute other mapped tasks while the call waits, which is why eager `get_result` calls early in a parent task don't kill parallelism the way they would in OpenMP.

## Mental model
A `Future<T>` is a `std::future<T>` for tasks instead of threads — same handle-and-promise pattern, but with deferred-execution semantics built into the substrate. Where a C++ future maps onto a thread, a Legion future maps onto a Realm event: the typing and reference counting are Legion's; the "is it done?" bit is Realm's.

## Mechanism & API
- **Production**: every `execute_task` returns one. Tasks return values normally via C++ `return`; the runtime marshals the bytes.
  ```cpp
  Future f = runtime->execute_task(ctx, launcher);
  ```
- **Consumption (blocking)**: `f.get_result<T>()` waits and returns the typed value. Use sparingly — pull every blocking call to the end of the parent task body so launches can pipeline.
- **Consumption (non-blocking, preferred)**: pass the future into another launcher.
  ```cpp
  launcher2.add_future(f);
  runtime->execute_task(ctx, launcher2);
  // Inside that task: task->futures[i].get_result<T>()
  ```
- **Empty future**: `Future()` is the default-constructed handle that is already "ready" with no value.
- **Reference counting**: futures are copyable; copies are cheap (refcount bump). Drop the last copy and the runtime can collect the buffer.
- `Future` is **not permitted as a task return type**. Pass futures via `add_future`, never via the typed return.

Serialization: arbitrary trivially-copyable return types are auto-marshalled. Non-trivial types need a custom serializer registered with the runtime.

## Invariants
- A future fires (its underlying event triggers) **exactly once** when its producing task completes.
- A future's value, once available, is immutable.
- Reference-counted: holders extend lifetime. The runtime cannot collect a future while any holder remains.
- Blocking on a future via `get_result` **does not block the underlying processor**; the runtime context-switches and runs other tasks. This is fundamental to Legion's deferred-execution model.
- A future passed into a launcher via `add_future` becomes an explicit task dependency: the consumer cannot map until the future is ready.

## Performance implications
- Cheap to create (one Realm event + one allocation for the buffer).
- **Eager `get_result` calls in a parent task delay subsequent launches** if structured as "launch, wait, launch, wait". Restructure to "launch all, then wait" so the analysis pipeline stays fed (see `operation-pipeline.md`).
- Passing futures into launchers (`add_future`) is the preferred composition: the runtime can begin dependence analysis on the consumer immediately without the parent task ever blocking.

## Debug signals
- **Legion Prof**: a "wait" gap on a processor row preceding a task starting → that task was blocked on a future's result.
- **Legion Spy** event graph: futures appear as edges from producer task to consumer task.
- **`REALM_SHOW_EVENT_WAITERS`**: if the app hangs, the dump shows the future-backing event in the cycle.

## Failure modes
- Returning a `Future` from a task → runtime error.
- Forgetting `add_future` and reading a stale buffer → undefined behavior in release builds.

## Source pointers
- **Legion API**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Tutorial**: https://legion.stanford.edu/tutorial/tasks_and_futures.html (mirrored at `raw/tutorials/02_tasks_and_futures.md`)

## Related
- `wiki/concepts/task.md` — what produces futures.
- `wiki/concepts/event.md` — what a future is built on at the Realm layer.
- `wiki/concepts/future-map.md` — the per-point analogue for index launches.

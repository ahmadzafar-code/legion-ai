---
title: Argument Map
slug: argument-map
summary: A point-to-buffer map attached to an IndexTaskLauncher; provides each point task with its own private `TaskArgument` payload while the launcher's global argument is shared across all points.
tags: [execution, for-program-reasoning]
subsystem: legion
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/03_index_tasks.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
related:
  - wiki/concepts/index-space-launch.md
  - wiki/concepts/task-launcher.md
  - wiki/concepts/task.md
  - wiki/concepts/future-map.md
---

## TL;DR
An `ArgumentMap` is a typed `DomainPoint → TaskArgument` map you attach to an `IndexTaskLauncher`. Each point task receives the launcher's *global* `TaskArgument` (via `task->args`) plus its own private bytes from the map (via `task->local_args` / `task->local_arglen`). The map is the standard way to give index-launch point tasks per-point inputs (a chunk ID, an offset, a per-point hyperparameter) without packing everything into futures. The confusion: the *global* `TaskArgument` is the same bytes broadcast to every point; the *map* is the only per-point input mechanism.

## Mental model
Picture an `ArgumentMap` like the `argv` of an MPI program — each rank gets its own slice of input parameters. Where MPI distributes via `MPI_Scatter`, Legion attaches the per-point payload to the launcher and the runtime hands the right slice to each point at execution.

## Mechanism & API
**Building**:
```cpp
ArgumentMap arg_map;
for (int i = 0; i < num_points; i++) {
  int value = i + 10;
  arg_map.set_point(/*point=*/i, TaskArgument(&value, sizeof(value)));
}
```

**Attaching to the launcher**:
```cpp
IndexTaskLauncher launcher(TASK_ID, launch_bounds, /*global=*/TaskArgument(NULL, 0), arg_map);
```

**Inside the point task**:
```cpp
int index_space_task(const Task *task, ...) {
  assert(task->local_arglen == sizeof(int));
  int my_value = *((const int*)task->local_args);
  // task->args is the launcher's global TaskArgument (shared across all points)
  // task->local_args is this point's private payload from the ArgumentMap
}
```

**ArgumentMap with FutureMap inputs**:
```cpp
// Build an ArgumentMap whose per-point values come from a prior FutureMap:
ArgumentMap arg_map2(prev_future_map);
```
This produces a per-point input where each point's bytes are the prior `FutureMap`'s point result — the standard pipelining pattern between index launches.

## Invariants
- An `ArgumentMap` is **point-indexed by `DomainPoint`**; points not in the launch domain are ignored.
- A point with no entry in the map gets **`local_arglen == 0` and `local_args == nullptr`**.
- The runtime **copies** the per-point bytes at launch time; the caller's buffer may be freed afterwards.
- An `ArgumentMap` is **mutable** between launches; clear/replace per-point entries before reusing.
- Like other launcher fields, `ArgumentMap` is value-typed — modifications after `execute_index_space` don't affect in-flight launches.

## Performance implications
- Cheap for small per-point payloads (a few bytes to a few KB).
- For large per-point inputs prefer **per-point regions** (point `i` accesses subregion `i` via a projection functor); the runtime can co-locate data with compute.
- Pipelined index-launch chains via `ArgumentMap(future_map)` let the runtime schedule downstream points as upstream completes — better than blocking on `wait_all_results` between launches.

## Debug signals
- **Mismatched `local_arglen`** asserts inside the point task usually mean a `set_point` was missed or the map has the wrong point indexing.
- **Legion Prof**: a `wait_all_results` fence followed by a downstream `IndexLauncher` shows up as a serial barrier; replacing it with `ArgumentMap(prev_fm)` should make the downstream points overlap.

## Failure modes
- Point task assumes a per-point payload but the launcher's `ArgumentMap` is missing — `local_arglen == 0`, the task reads garbage.

## Source pointers
- **Legion API**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Tutorial**: https://legion.stanford.edu/tutorial/index_tasks.html (mirrored at `raw/tutorials/03_index_tasks.md`)

## Related
- `wiki/concepts/index-space-launch.md` — the launch context for `ArgumentMap`.
- `wiki/concepts/task-launcher.md` — `IndexTaskLauncher` carries one.
- `wiki/concepts/task.md` — `task->local_args` / `task->local_arglen` accessors.
- `wiki/concepts/future-map.md` — the pipelining counterpart at the result side.

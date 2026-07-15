---
title: Task Launcher
slug: task-launcher
summary: The C++ struct (`TaskLauncher` for single tasks, `IndexTaskLauncher` for index launches) that bundles a task ID with arguments, region requirements, futures, predicates, and a mapper ID; the input to `execute_task` / `execute_index_space`.
tags: [execution, for-program-reasoning]
subsystem: legion
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/02_tasks_and_futures.md
  - raw/tutorials/03_index_tasks.md
  - raw/tutorials/07_privileges.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
related:
  - wiki/concepts/task.md
  - wiki/concepts/task-variant.md
  - wiki/concepts/region-requirement.md
  - wiki/concepts/argument-map.md
  - wiki/concepts/future.md
  - wiki/concepts/index-space-launch.md
---

## TL;DR
A `TaskLauncher` is the structured input to `runtime->execute_task` — it carries a `TaskID`, a `TaskArgument` payload, a list of `RegionRequirement`s (with field sets), a list of futures the task depends on, an optional `Predicate`, an optional `MapperID`, and a mapper-readable `tag`. `IndexTaskLauncher` is the index-space analogue, additionally carrying an iteration `Domain` and an `ArgumentMap` for per-point arguments. The confusion: launchers are *plain structs*, not RPC channels — you fill in the fields, hand to `execute_task`, and the runtime owns the rest. Reusing a launcher across calls is fine, but mutate fields explicitly (`region_requirements[0].privilege_fields.clear()` etc.) so state doesn't leak.

## Mental model
A launcher is `argv` for a Legion task: a fully-described intent to execute. Where `argv` says "run program X with these strings", a launcher says "run task X with these arguments, these regions and privileges, these future dependencies, under this mapper". The runtime takes it from there.

## Mechanism & API

**TaskLauncher (single task)**:
```cpp
TaskLauncher launcher(TASK_ID, TaskArgument(&payload, sizeof(payload)));
launcher.add_region_requirement(
    RegionRequirement(lr, READ_WRITE, EXCLUSIVE, lr));
launcher.add_field(0, FID_X);
launcher.add_future(prev_future);
launcher.map_id = MY_MAPPER_ID;     // optional, defaults to 0 (DefaultMapper)
launcher.tag = MY_HINT_TAG;         // arbitrary mapper-visible bits
launcher.predicate = pred;          // optional Legion predicate
Future f = runtime->execute_task(ctx, launcher);
```

**IndexTaskLauncher (index space)**:
```cpp
IndexTaskLauncher launcher(TASK_ID, color_is, TaskArgument(NULL, 0), arg_map);
launcher.add_region_requirement(
    RegionRequirement(lp, /*proj_id=*/0, RO, EXCLUSIVE, lr));
launcher.region_requirements[0].add_field(FID_X);
FutureMap fm = runtime->execute_index_space(ctx, launcher);
```

**Key fields on `TaskLauncher`** (also on `IndexTaskLauncher` unless noted):
- `task_id` — which `TaskID`.
- `argument` — by-value `TaskArgument` (pointer + size; runtime copies).
- `region_requirements` — `std::vector<RegionRequirement>`; mutate directly between launches.
- `futures` — `std::vector<Future>` added via `add_future(f)`. Inside the task, `task->futures[i]`.
- `map_id` — which registered mapper handles this launch (default 0).
- `tag` — application-defined `MappingTagID` for hints to the mapper without touching its interface.
- `predicate` — Legion `Predicate` for conditional execution (`Predicate::TRUE_PRED` if always run).
- `priority` — scheduler priority hint.

**IndexTaskLauncher additions**:
- `launch_space` / `launch_domain` — the iteration domain.
- `argument_map` — `ArgumentMap` of per-point inputs (`argument-map.md`).
- The region requirements typically use a `LogicalPartition` + a projection ID so each point sees its own subregion.

## Invariants
- Launchers are **value types**; the runtime copies what it needs at `execute_*` time. Mutating the launcher after the call doesn't affect the in-flight task.
- A `RegionRequirement` added to a launcher must reference a region the **parent task has privileges on** (subset rule, `privilege.md`).
- `add_future(f)` makes the future a real dependency; the task cannot map until `f` is ready.
- The `tag` is **opaque to the runtime**; only the mapper interprets it.
- An `IndexTaskLauncher` with a projection-functor-bearing region requirement projects each point's subregion independently — see `index-space-launch.md`.
- Reusing a launcher across iterations works, but **field sets persist** — call `region_requirements[i].privilege_fields.clear()` and re-add fields before reusing if the field set should change.

## Performance implications
- The launcher itself is cheap; cost comes from the dependence + physical analyses (`operation-pipeline.md`).
- **Reusing a launcher inside a `begin_trace`/`end_trace` body** with identical content lets the runtime memoize analysis. Inadvertently varying any launcher field invalidates the trace.
- For data-parallel work, **prefer `IndexTaskLauncher` over a loop of `TaskLauncher`s** — see `index-space-launch.md` for why.
- `priority` hints can move latency-critical tasks ahead in the ready queue but don't change correctness.

## Debug signals
- **Legion Spy** shows the per-launcher region requirements as edges; a missing field on a requirement = forgotten `add_field`.
- **Privilege-mismatch errors** at runtime (debug build): launcher requested a privilege the parent doesn't hold.
- **"No valid variant" errors**: launcher's region-requirement layout doesn't match any registered task variant's constraints.

## Failure modes
- Forgetting to clear `privilege_fields` between reuses → stale fields requested unintentionally.
- Passing a `TaskArgument` whose pointer doesn't outlive the call → runtime copies at launch, but accessing the original buffer inside the task is undefined.

## Source pointers
- **Legion API**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Tutorial (TaskLauncher)**: https://legion.stanford.edu/tutorial/tasks_and_futures.html
- **Tutorial (IndexTaskLauncher)**: https://legion.stanford.edu/tutorial/index_tasks.html

## Related
- `wiki/concepts/task.md` — what the launcher launches.
- `wiki/concepts/task-variant.md` — what the mapper resolves to.
- `wiki/concepts/region-requirement.md` — primary launcher payload.
- `wiki/concepts/argument-map.md` — per-point args for `IndexTaskLauncher`.
- `wiki/concepts/future.md` — `add_future` inputs.
- `wiki/concepts/index-space-launch.md` — the `IndexTaskLauncher` flow end-to-end.

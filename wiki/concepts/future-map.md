---
title: Future Map
slug: future-map
summary: The Future returned by an index-space launch; a mapping from each point in the launch domain to a typed result, with a fence operation to wait for all points.
tags: [execution, synchronization, for-program-reasoning]
subsystem: legion
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/03_index_tasks.md
  - raw/tutorials/02_tasks_and_futures.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
related:
  - wiki/concepts/future.md
  - wiki/concepts/index-space-launch.md
  - wiki/concepts/task.md
---

## TL;DR
A `FutureMap` is the result of `runtime->execute_index_space(ctx, launcher)`: one logical handle indexed by point that yields a `Future` per point task. `fm.wait_all_results()` is the standard fence on the entire launch; `fm.get_result<T>(point)` extracts a single point's typed return. The confusion: a `FutureMap` is *not* `std::vector<Future>` — it's one runtime object representing all the per-point completions collectively, so the runtime can fold-reduce them (when reductions are configured) or sample any subset without forcing the whole launch to complete.

## Mental model
If `Future` is `std::future<T>`, `FutureMap` is `std::unordered_map<Point, std::future<T>>` — but with collective operations the standard library can't express (per-point sample, all-points fence, optional reduction folding). Useful image: a 2D Excel sheet of pending values, where each cell becomes filled in independently as its point task completes.

## Mechanism & API
- **Production**: from an index launch.
  ```cpp
  FutureMap fm = runtime->execute_index_space(ctx, launcher);
  ```
- **Per-point read**: `fm.get_result<T>(point)` (blocks until that point completes).
- **Full fence**: `fm.wait_all_results()` (blocks until every point completes).
- **Reduction-style fold**: if the launcher's `IndexLauncher::predicate_false_future` or reduction operator is set, the runtime can produce a single `Future` from the map.
- **Passing forward**: like `Future`, a `FutureMap` is reference-counted and can be handed into a downstream `IndexLauncher` (`add_future_map(fm)`).

Tutorial-style usage (DAXPY-like check pass):
```cpp
FutureMap fm = runtime->execute_index_space(ctx, launcher);
fm.wait_all_results();
for (int i = 0; i < num_points; i++)
  total += fm.get_result<int>(i);
```

## Invariants
- One `FutureMap` corresponds to **one index launch**. There's no abstraction-level merge across launches.
- Per-point futures inside a `FutureMap` fire independently as their point tasks complete; the map is "fully ready" only after the last point.
- Per-point lookup with a `DomainPoint` not in the launch's domain is an error (debug build: assertion; release: undefined).
- The map's reference count keeps the per-point result buffers alive — drop all holders and the runtime collects the buffers.

## Performance implications
- `wait_all_results()` is the most common fence pattern; it's effectively a barrier across the whole index launch.
- **Avoid per-point `get_result` in a loop** if you can pass the entire map to a downstream consumer instead — the runtime can pipeline that, while the loop forces serial collection.
- For pure reduction-style aggregations (sum, max, etc.), configure the launcher with a `ReductionOpID` so the runtime folds in place. This avoids materializing N per-point buffers.

## Debug signals
- **Legion Prof**: per-point bars of the index launch should appear roughly concurrently; if they staircase, the partition is aliased or privileges interfere.
- **`fm.wait_all_results()`** appears in Prof as a fence point — every point's completion is observed before subsequent work.
- **Legion Spy**: each point's future is one edge from the point task to its consumer in the event graph.

## Failure modes
- Reading a point not present in the launch domain → undefined behavior in release builds.
- Confusing `FutureMap` reference-count drops with point-buffer freedom: the map keeps all points alive until destroyed.

## Source pointers
- **Legion API**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Tutorial**: https://legion.stanford.edu/tutorial/index_tasks.html

## Related
- `wiki/concepts/future.md` — single-task counterpart.
- `wiki/concepts/index-space-launch.md` — what produces a `FutureMap`.
- `wiki/concepts/task.md` — the per-point task whose results populate the map.

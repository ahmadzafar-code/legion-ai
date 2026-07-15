---
title: Runtime Overhead Dominates
slug: runtime-overhead-dominates
summary: Pipeline stages 1-3 (dispatch, dependence analysis, ready) cost more than the actual task execution; the runtime processes more "work" than the application does.
tags: [for-perf-debug, execution, configuration]
status: draft
created: 2026-05-15
updated: 2026-05-15
related:
  - wiki/concepts/operation-pipeline.md
  - wiki/concepts/task.md
  - wiki/concepts/index-space-launch.md
  - wiki/concepts/tracing.md
  - wiki/concepts/control-replication.md
  - wiki/concepts/legion-prof.md
  - wiki/workflows/enable-tracing.md
  - wiki/workflows/move-from-single-node-to-distributed.md
---

## Symptom

- **Application processor rows** in Legion Prof show **many tiny task bars** — bars whose width is on the order of microseconds, separated by gaps.
- **Utility processor rows** (`UTIL_PROC`) are saturated; the runtime is doing more work than the application.
- Wall-clock throughput is far lower than back-of-envelope estimates from the per-task kernel cost.
- The `critical-path.md` (press `a`) runs through utility rows and gaps between application tasks, not through application work.
- Distinguish from `mapper-stalls`: mapper-stalls have utility-row activity *between* tasks but the mapper callbacks are slow; runtime-overhead has many small utility events from the *operation pipeline itself*.

## Cause

The Legion runtime pays a fixed per-operation cost — pipeline stages 1 (dispatch), 2 (dependence analysis), and 3 (ready queue management). If task granularity is so small that **per-task runtime cost exceeds per-task application work**, the runtime is the bottleneck. Common contributors:

1. **Iterated `execute_task` calls instead of `execute_index_space`**: a `for` loop of N single-task launches creates N pipeline operations. An equivalent `IndexLauncher` creates **one** operation node that expands to N points — N× cheaper at stages 2-3.
2. **Tasks that complete in microseconds**: fine-grained tasking with leaf-task bodies smaller than the runtime's pipeline overhead. The runtime can't process operations faster than ~100k/sec per utility processor; tasks faster than that bottleneck on the pipeline.
3. **Missing tracing**: on repeating loop bodies, the runtime re-runs full dep-analysis every iteration. See `pitfalls/missed-tracing-opportunity.md`.
4. **No control replication on multi-node runs**: the top-level task runs on one node and all dep-analysis happens there. See `control-replication.md`.

This pitfall is most visible at scale (large iteration counts, many small tasks) and on multi-node runs where centralized analysis can't keep up.

## Fix

- **Use `IndexLauncher` for data-parallel work**: replace `for (int i=0; i<N; i++) execute_task(...)` with one `execute_index_space(...)`. The runtime sees one operation, not N. Verify in `legion-spy.md`'s dataflow graph — it should show one index-launch node instead of N task nodes.
- **Coarsen tasks**: merge several small kernel calls into one task body. The application loses some parallelism at the task level but recovers it at the kernel level, with much less runtime cost.
- **Wrap repeating loop bodies in `begin_trace`/`end_trace`** — see `tracing.md` and `pitfalls/missed-tracing-opportunity.md`. On replay, stages 2 and 5 are skipped.
- **Enable control replication** on the top-level task: `set_replicable(true)` on its variant, and run with multiple shards. Per-shard analysis cost becomes 1/N of total.
- **Tune `-lg:width`**: increase the scheduling-pass batch size so each pass amortizes overhead over more operations.
- **Add more utility processors**: `-ll:util 4` (or higher) gives the runtime more parallel pipeline bandwidth. Cap at the available cores; usually 2-4 is enough.

After the fix, application rows should be **continuously busy** with the critical path running through them, not through utility rows or gaps.

## Underlying concepts

- `wiki/concepts/operation-pipeline.md` — the stages whose overhead is the bottleneck.
- `wiki/concepts/task.md` / `wiki/concepts/index-space-launch.md` — the unit-of-work decision.
- `wiki/concepts/tracing.md` — the primary fix for repeating patterns.
- `wiki/concepts/control-replication.md` — the multi-node fix.
- `wiki/concepts/legion-prof.md` — where the symptom is visible.

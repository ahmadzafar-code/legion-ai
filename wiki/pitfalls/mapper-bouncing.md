---
title: Mapper Bouncing
slug: mapper-bouncing
summary: The mapper assigns the same logical task to different processors (or processor kinds) on successive iterations, forcing the runtime to copy instances back and forth between memories.
tags: [for-perf-debug, mapping]
status: draft
created: 2026-05-15
updated: 2026-05-15
related:
  - wiki/concepts/mapper.md
  - wiki/concepts/map-task.md
  - wiki/concepts/physical-instance.md
  - wiki/concepts/dma-system.md
  - wiki/concepts/legion-prof.md
  - wiki/concepts/mapper-logging.md
---

## Symptom

- The **same logical task** appears on **different processor rows** across iterations in Legion Prof's `timeline-view.md`. One iteration the task ran on CPU, the next iteration on GPU, the next back on CPU.
- **Channel rows show oscillating copies** between two memories — `SYSTEM_MEM → GPU_FB_MEM` followed by `GPU_FB_MEM → SYSTEM_MEM` on the next iteration.
- The application gets slower over time without obvious cause; no individual task is slow, but each iteration pays setup cost.
- **`-level trace=2`** shows `trace invalidated` messages around the affected task IDs — because the mapper's placement changed, the trace can't replay.

## Cause

A custom mapper's `map_task.md` (or `select_task_options.md`) callback returns **unstable choices** for the same logical task across iterations. Common patterns that cause this:

1. **Load-balancing heuristic with no hysteresis**: the mapper picks "the least-loaded processor right now" and the least-loaded one varies between iterations. Each switch forces a data migration.
2. **Stateful cost model with stale data**: the mapper tracks per-processor latencies or queue depths and reads from a model that hasn't converged. Each call uses different numbers and gets a different answer.
3. **Random or fuzz-test mappers** (e.g., the `AdversarialMapper` from the tutorial) used by mistake in production code.
4. **Conflicting `select_task_options::initial_proc` and `map_task::target_procs`**: the first callback picks one processor, the second changes its mind. The runtime relocates the task, which can leave instances in the wrong memory.

Crucially, the runtime **cannot tell** that the mapper's decision is unstable — it executes whatever the mapper returns. The cost shows up as: (a) DMAs to migrate instances, (b) invalidated traces that re-record, (c) cold caches at the destination.

## Fix

- **Pin tasks to processor kinds** via `ProcessorConstraint` on the task variant (`task-variant.md`). If a task must run on GPU, register only a GPU variant. The mapper has no choice but to put it on a `TOC_PROC`.
- **Add hysteresis to the cost model**: don't move a task off its previous processor unless the alternative is significantly (e.g., 20%+) better. Standard scheduler trick.
- **Cache placement decisions per task ID + index point**: once you decide where `STENCIL_TASK_ID[0,0]` runs, keep that decision. `runtime->set_mapper_data(ctx, task, ...)` stores per-task mapper state.
- **Confirm with `mapper-logging.md`**: wrap the mapper in `LoggingWrapper`, run with `-level mapper=2`, and grep the `target_procs` field across iterations. If the same task lands on different processors, you've confirmed the bouncing.
- **Disable the buggy mapper temporarily** by reverting to `default-mapper.md` for the affected tasks — usually a quick perf win and a sanity check that the mapper was the cause.

After the fix, expect the iteration-time staircase to flatten in `legion-prof.md`, and trace replay to start working again (`-level trace=2` should show `trace replayed` for the affected sequence).

## Underlying concepts

- `wiki/concepts/mapper.md` — where placement is decided.
- `wiki/concepts/map-task.md` — the specific callback responsible for `target_procs` / `chosen_instances`.
- `wiki/concepts/physical-instance.md` — the buffers that bounce.
- `wiki/concepts/dma-system.md` — what produces the oscillating channel-row activity.
- `wiki/concepts/legion-prof.md` — the timeline where the symptom is visible.
- `wiki/concepts/mapper-logging.md` — the tool that confirms the diagnosis.

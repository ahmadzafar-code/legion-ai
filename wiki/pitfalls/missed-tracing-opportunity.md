---
title: Missed Tracing Opportunity
slug: missed-tracing-opportunity
summary: A repeating loop body (time-step, training iteration) is not wrapped in begin_trace/end_trace, so the runtime re-runs dependence analysis every iteration instead of memoizing it.
tags: [for-perf-debug, tracing, execution]
status: draft
created: 2026-05-15
updated: 2026-05-15
related:
  - wiki/concepts/tracing.md
  - wiki/concepts/operation-pipeline.md
  - wiki/concepts/control-replication.md
  - wiki/concepts/legion-prof.md
  - wiki/workflows/enable-tracing.md
---

## Symptom
- An iterative program (stencil solver, training loop, time-step) is slower than expected.
- **Legion Prof** shows persistent utility-processor (UTIL_PROC) activity *every* iteration, not just the first.
- Adding processors helps less than expected; the critical path is gated by analysis time, not compute.
- Profile of the same code with `-lg:no_tracing` (or equivalent) is roughly the same speed — meaning tracing isn't doing anything because it isn't enabled.

## Cause
By default, Legion re-runs **stage 2 (dependence analysis)** and **stage 5 (physical analysis)** of the operation pipeline (`operation-pipeline.md`) for every operation. If the same sequence of operations is issued every iteration, this is repeated work that scales with iteration count.

Three flavors of this bug:
1. **No trace markers** around the loop body. The runtime cannot know the structure repeats.
2. **Trace markers but `memoize = false`** in the mapper (default mapper sets `memoize = true`, but custom mappers sometimes forget). Without memoization, the trace records but never replays.
3. **Trace invalidation.** Markers are present and memoization is on, but the operation sequence isn't actually identical iteration-to-iteration (mapper chose differently, a branch went the other way, a different region got created). Each entry to the trace ID re-records.

## Fix
- **Wrap the loop body**:
  ```cpp
  for (int step = 0; step < num_steps; step++) {
    runtime->begin_trace(ctx, /*trace_id=*/0);
    // identical-structure launches go here
    runtime->end_trace(ctx, /*trace_id=*/0);
  }
  ```
- **Use a stable `trace_id`** — picking a fresh ID per iteration defeats memoization.
- **Confirm in the mapper** that `select_task_options::memoize = true` for tasks inside the trace. `DefaultMapper` does this by default.
- **Investigate invalidation** by running with `-level trace=2`; look for "trace invalidated" messages and the reason given.
- For pattern-detection without manual markers, opt into **automatic tracing** (paper `autotrace2025.pdf`).
- Combined with **control replication** (`control-replication.md`), the per-shard analysis collapse multiplies the win on multi-node runs.

## Underlying concepts
- `wiki/concepts/tracing.md` — what is being memoized.
- `wiki/concepts/operation-pipeline.md` — the stages being skipped.
- `wiki/concepts/control-replication.md` — why this matters more at scale.
- `wiki/concepts/legion-prof.md` — UTIL-row activity as the signal.

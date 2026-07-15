---
title: Enable Tracing
slug: enable-tracing
summary: A recipe for adding dynamic tracing (`begin_trace`/`end_trace`) to an iterative Legion application; the single biggest perf win for most time-stepped / training-loop codes.
tags: [for-perf-debug, tracing, execution]
status: draft
created: 2026-05-15
updated: 2026-05-15
related:
  - wiki/concepts/tracing.md
  - wiki/concepts/dynamic-tracing.md
  - wiki/concepts/automatic-tracing.md
  - wiki/concepts/trace-recording.md
  - wiki/concepts/trace-replay.md
  - wiki/concepts/select-task-options.md
  - wiki/concepts/legion-prof.md
  - wiki/pitfalls/missed-tracing-opportunity.md
---

## Inputs

- A Legion (or Regent) application with a repeating loop whose body issues an identical sequence of operations each iteration — stencil time-stepping, training iterations, iterative solvers.
- A profile from `legion-prof.md` showing `pitfalls/missed-tracing-opportunity.md` symptoms (persistent utility-row activity every iteration).

## Steps

1. **Identify the repeating loop**. The trace must cover a sequence of operations that's structurally identical across iterations — same launchers, same region requirements, same field sets. Changing-shape sequences invalidate the trace.

2. **Pick a stable `trace_id`**. Any unsigned integer; the rule is to use the **same ID** across all iterations of the loop. Don't pick a fresh ID per iteration (that defeats memoization).

3. **Wrap the loop body**:
   ```cpp
   for (int step = 0; step < num_steps; step++) {
     runtime->begin_trace(ctx, /*trace_id=*/0);

     runtime->execute_index_space(ctx, stencil_launcher);
     runtime->execute_index_space(ctx, exchange_launcher);
     // ... whatever the iteration does ...

     runtime->end_trace(ctx, /*trace_id=*/0);
   }
   ```

4. **Verify the mapper opts tasks into memoization**. `select_task_options::memoize = true` is required for tasks inside the trace to participate. `default-mapper.md` sets this by default; custom mappers sometimes forget. Check with `mapper-logging.md`.

5. **Run with `-level trace=2`** to see record/replay events:
   ```bash
   ./app -level trace=2 -logfile trace_%.log
   ```
   You should see one `trace recorded` line per `(ctx, trace_id)` cold path, then `trace replayed` for subsequent iterations. If you see repeated `trace invalidated` messages, the operation stream is varying — fix the structural variance.

6. **Profile and verify the win**:
   ```bash
   DEBUG=0 make
   ./app -lg:prof N -lg:prof_logfile prof_%.gz
   legion_prof --view prof_*.gz
   ```
   On a successful trace, utility-row activity should drop sharply after the first iteration (`pitfalls/missed-tracing-opportunity.md` symptom resolved). Per-iteration time should plateau.

7. **(Optional) Try logical-only tracing** if physical templates won't converge:
   ```cpp
   runtime->begin_trace(ctx, /*trace_id=*/0, /*logical_only=*/true);
   ```
   Memoizes only stage 2 (logical analysis) — useful when mapping varies but logical structure is stable.

8. **(Optional) Try automatic tracing** if you don't want to add markers manually. See `automatic-tracing.md` for the runtime flag (current version's `-help` will show it).

9. **(Multi-node) Combine with control replication** for compound wins. Replicable top-level task + per-shard tracing collapses analysis cost across both axes. See `wiki/workflows/move-from-single-node-to-distributed.md`.

## Outputs

- A traced loop body with `begin_trace`/`end_trace` markers.
- A re-profiled run showing the iteration-time plateau and reduced utility-row activity.
- A quantified speedup vs. the untraced baseline.

## When to use

- Any iterative Legion application with a repeating loop, even if it "feels fast enough" — tracing is essentially free when the structure is stable.
- After capturing a profile that shows `pitfalls/missed-tracing-opportunity.md` (persistent UTIL activity).
- Before scaling to multi-node — tracing's benefit multiplies with control replication.

## Related

- `wiki/concepts/tracing.md` — umbrella concept.
- `wiki/concepts/dynamic-tracing.md` — the API used here.
- `wiki/concepts/automatic-tracing.md` — alternative for un-bracketed patterns.
- `wiki/concepts/trace-recording.md` / `wiki/concepts/trace-replay.md` — what happens internally.
- `wiki/concepts/select-task-options.md` — `memoize` gates participation.
- `wiki/concepts/legion-prof.md` — how to see the win.
- `wiki/pitfalls/missed-tracing-opportunity.md` — symptom this workflow resolves.

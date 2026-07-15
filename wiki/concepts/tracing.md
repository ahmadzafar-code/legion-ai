---
title: Tracing
slug: tracing
summary: A memoization mechanism that records the result of dependence analysis and physical analysis on the first execution of a marked region, then replays it on subsequent iterations to skip the runtime overhead.
tags: [tracing, execution, for-perf-debug]
subsystem: legion
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/youtube_transcripts/runtime_school_2023/transcripts/021_Legion_Runtime_Internals_-_Lesson_22_-_Tracing_Part_1.txt
  - raw/publications/publications.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
related:
  - wiki/concepts/operation-pipeline.md
  - wiki/concepts/task.md
  - wiki/concepts/control-replication.md
  - wiki/concepts/mapper.md
  - wiki/concepts/dependence-analysis.md
  - wiki/concepts/logical-analysis.md
  - wiki/concepts/physical-analysis.md
  - wiki/concepts/automatic-tracing.md
  - wiki/concepts/dynamic-tracing.md
  - wiki/concepts/static-tracing.md
  - wiki/concepts/trace-recording.md
  - wiki/concepts/trace-replay.md
  - wiki/concepts/select-task-options.md
  - wiki/concepts/task-fusion.md
---

## TL;DR
Tracing tells the runtime "this sequence of operations will repeat with the same structure"; the first pass records the logical and physical analysis results, and subsequent passes replay them, skipping pipeline stages 2 and 5. Three variants: **static tracing** (compiler-asserted), **dynamic tracing** (application-bracketed with `begin_trace`/`end_trace`), and **automatic tracing** (the runtime detects repetition; paper `autotrace2025.pdf`). The confusion: tracing is *enabled* by an `unsigned trace_id` (or by the mapper setting `memoize = true`) — without it, the runtime re-analyzes every iteration even of an obviously-cyclic loop.

## Mental model
> "It's like having a little mini-jitting compiler inside of your runtime system to capture some of these things and optimize traces and replay them." — Runtime School 2023, Lesson 22.

The expensive parts of stages 2 and 5 are equivalent to instruction decode + register rename in an OOO CPU. Tracing is the trace cache. The first iteration is cold; subsequent iterations skip directly to "issue the cached operation packet to the execution units (events)". When the loop pattern changes (different launches, different region sets), the trace **invalidates** and the runtime falls back to full analysis on the next entry.

## Mechanism & API
- **Dynamic tracing** (most common):
  ```cpp
  for (int step = 0; step < num_steps; step++) {
    runtime->begin_trace(ctx, /*trace_id=*/0);
    runtime->execute_index_space(ctx, init_launcher);
    runtime->execute_index_space(ctx, stencil_launcher);
    runtime->execute_index_space(ctx, exchange_launcher);
    runtime->end_trace(ctx, /*trace_id=*/0);
  }
  ```
  Today the application is **trusted**: if the sequence inside `begin_trace`/`end_trace` ever differs across iterations (different launchers, different region requirements, different field sets), behavior is undefined.

- **Logical-only vs full tracing**: `begin_trace(ctx, id, /*logical_only=*/true)` skips physical analysis memoization; useful if mappings change but logical dependencies are stable.

- **Static tracing**: marks a trace as known to be invariant at compile time (used by Regent). Largely deprecated in favor of dynamic + automatic.

- **Automatic tracing** (`autotrace2025.pdf`, ASPLOS 2025): the runtime watches the operation stream, detects repeated patterns via suffix-array-like analysis, and creates traces without application markers. Enabled via runtime flags.

- The mapper gates trace replay: `select_task_options::memoize = true` opts a task into being part of a memoizable trace. Default mapper sets this for most tasks.

## Invariants
- A trace is identified by a `(ctx, trace_id)` pair within a context.
- The runtime **trusts** the application that successive entries to the same trace ID are structurally identical (same launches, same region requirements, same dependencies). Today this is not checked except in extra-paranoid configurations.
- Replay regenerates **events**, not data: the trace records *what* operations to issue and *how* they depend, not their results.
- If the mapper makes a different decision on a replayed iteration, the trace invalidates and re-records.
- Logical tracing memoizes pipeline stage 2; physical tracing memoizes stage 5. Both together yield the biggest replay speedup.

## Performance implications
- Often the **single biggest perf win** for iterative codes — stencil time-loops, training loops, etc.
- Without tracing, a 1000-step simulation pays the dep-analysis + physical-analysis cost 1000×; with tracing, it pays it once (cold) and replays 999×.
- Trace **invalidation** is expensive: pick trace IDs carefully so the structure inside a trace really is stable.
- Automatic tracing eliminates the application-side bookkeeping at the cost of a small detection overhead.
- See paper `dcr2021.pdf` for tracing under control replication.

## Debug signals
- **Legion Prof**: utility-processor activity should drop dramatically after the first traced iteration. If it doesn't, the trace isn't replaying.
- **`-level trace=2`**: logs trace recording and replay events; "trace replayed" vs "trace invalidated" lines tell you the cache state.
- **`-lg:no_tracing`** (or app-level flag): disables tracing entirely; useful for A/B perf comparison.
- **Legion Spy**: the operation DAG should be identical between traced iterations.

## Failure modes
- [Missed tracing opportunity](../pitfalls/missed-tracing-opportunity.md) — a loop body isn't wrapped in `begin_trace`/`end_trace` and the runtime re-analyzes every iteration.
- [Long dependence chains](../pitfalls/long-dependence-chains.md) — a trace can collapse them per replay.

## Source pointers
- **Legion API** (`begin_trace`/`end_trace`): https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Paper (dynamic tracing)**: `raw/publications/pdfs/trace2018.pdf`
- **Paper (DCR + tracing)**: `raw/publications/pdfs/dcr2021.pdf`
- **Paper (automatic tracing)**: `raw/publications/pdfs/autotrace2025.pdf`
- **Lectures**: `raw/youtube_transcripts/runtime_school_2023/` (Lessons 22–24)

## Related
- `wiki/concepts/operation-pipeline.md` — what tracing skips.
- `wiki/concepts/task.md` — the units that flow through a trace.
- `wiki/concepts/control-replication.md` — replicated tracing.
- `wiki/concepts/mapper.md` — `memoize` opts tasks into tracing.

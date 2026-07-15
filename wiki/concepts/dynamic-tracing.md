---
title: Dynamic Tracing
slug: dynamic-tracing
summary: The application-bracketed form of tracing (`runtime->begin_trace`/`end_trace`) where the runtime records the first iteration's analysis and replays it on subsequent iterations; comes in logical-only and full (logical + physical) variants.
tags: [tracing, execution, for-perf-debug]
subsystem: legion
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/youtube_transcripts/runtime_school_2023/transcripts/021_Legion_Runtime_Internals_-_Lesson_22_-_Tracing_Part_1.txt
  - raw/youtube_transcripts/runtime_school_2023/transcripts/022_Legion_Runtime_Internals_-_Lesson_23_-_Tracing_Part_2.txt
  - raw/youtube_transcripts/runtime_school_2023/transcripts/023_Legion_Runtime_Internals_-_Lesson_24_-_Tracing_Part_3.txt
  - raw/publications/publications.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion_trace.h
related:
  - wiki/concepts/tracing.md
  - wiki/concepts/trace-recording.md
  - wiki/concepts/trace-replay.md
  - wiki/concepts/static-tracing.md
  - wiki/concepts/automatic-tracing.md
  - wiki/concepts/operation-pipeline.md
  - wiki/applications/pennant.md
  - wiki/applications/circuit.md
---

## TL;DR
Dynamic tracing is the standard tracing API: the application brackets a sequence of operations with `runtime->begin_trace(ctx, id)` and `runtime->end_trace(ctx, id)`, and the runtime memoizes the dependence-analysis result on the first pass through the trace, replaying it cheaply on subsequent passes. Two flavors: **logical-only tracing** (memoizes stage 2 of the pipeline) and **full dynamic tracing** (memoizes stages 2 and 5). The confusion: even within dynamic tracing, the runtime keeps **multiple templates per trace ID** because the mapper's decisions can change between iterations — each template covers one set of mapping preconditions.

## Mental model
Dynamic tracing is the trace cache of a hardware JIT. First entry to a `(ctx, trace_id)` pair is a cold pass — record. Subsequent entries are warm — check preconditions, replay. The runtime trusts the application's promise that the operation stream inside the brackets is structurally identical across passes; if it isn't, behavior is undefined. The mapper's freedom to vary decisions across passes is why a single trace ID needs *several* templates.

## Mechanism & API
```cpp
for (int step = 0; step < num_steps; step++) {
  runtime->begin_trace(ctx, /*trace_id=*/0);
  runtime->execute_index_space(ctx, stencil_launcher);
  runtime->execute_index_space(ctx, exchange_launcher);
  runtime->end_trace(ctx, /*trace_id=*/0);
}
```

**Two flavors**, controlled at `begin_trace`:
- **Full dynamic tracing**: `runtime->begin_trace(ctx, id)` — memoizes both logical analysis (stage 2) and physical analysis (stage 5). The standard form.
- **Logical-only tracing**: `runtime->begin_trace(ctx, id, /*logical_only=*/true)` — memoizes only stage 2. Use when mapping decisions vary too much for physical-tracing templates to converge, but the logical operation graph is stable.

**Internal structure** (per Lesson 22–24):
- One `LogicalTrace` per `(parent_task_context, trace_id)` pair — captures the application's logical operation sequence.
- Optionally points to a `PhysicalTrace` — captures physical-analysis state.
- Each `PhysicalTrace` holds a **vector of templates** (`std::vector<PhysicalTemplate>`); bounded at ~5–10 to cap memory.
- Each template is a recorded capture with preconditions, postconditions, and anticonditions on equivalence sets, plus a list of "instructions" (Realm operations to replay) — see `trace-recording.md`.

**Pipeline interaction**:
- First pass to a trace ID: record the operation stream + mapping decisions + physical-analysis results.
- Subsequent passes: test the templates' preconditions against current equivalence-set state. If one matches, replay (see `trace-replay.md`). If none matches, fall back to record a new template (up to the cap; LRU thereafter).

**Mapper interaction**: `select_task_options::memoize = true` opts each task into tracing. `DefaultMapper` sets this by default.

## Invariants
- The operation stream between `begin_trace` and `end_trace` must be **structurally identical across passes** (same launchers, region requirements, fields, dependencies). The runtime trusts this; violation = UB.
- Logical tracing assumes only application-side identity; physical tracing additionally requires **the same mapping decisions or a precondition match**.
- A `(ctx, trace_id)` pair has exactly one `LogicalTrace`; a `LogicalTrace` has at most one `PhysicalTrace`; a `PhysicalTrace` has many templates.
- Templates are bounded; evicted templates require re-recording if their pattern recurs.
- `-lg:no_tracing` (or equivalent flag) disables tracing entirely — useful for A/B testing.

## Performance implications
- Often the single biggest perf win for iterative codes (stencils, training loops, time-stepping). Without tracing, dep-analysis cost scales linearly with iteration count.
- **Logical-only** is cheaper to record but skips less work on replay. Use it when physical-trace templates won't converge.
- The **bounded template count** means workloads with N>>10 distinct mapping configurations thrash. Diagnose via `-level trace=2` showing many "trace invalidated" entries.
- Combined with **control replication** (`control-replication.md`), per-shard analysis cost collapses near-zero on replay.

## Debug signals
- **Legion Prof**: utility-processor activity should drop dramatically after the first traced iteration. If it doesn't, the trace isn't replaying (precondition mismatch or memoize=false).
- **`-level trace=2`**: logs each capture, each replay attempt, each invalidation with reason.
- **A/B run** with `-lg:no_tracing` (or `runtime->disable_tracing()` API) quantifies the tracing speedup.

## Failure modes
- [Missed tracing opportunity](../pitfalls/missed-tracing-opportunity.md) — loop body isn't bracketed.
- Trace invalidation thrash — many distinct mapping configurations exceed the template cap.

## Source pointers
- **Header**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion_trace.h
- **Paper (dynamic tracing)**: `raw/publications/pdfs/trace2018.pdf` (SC 2018) — *Dynamic Tracing: Memoization of Task Graphs for Dynamic Task-Based Runtimes*
- **Paper (DCR + tracing)**: `raw/publications/pdfs/dcr2021.pdf`
- **Lectures**: `raw/youtube_transcripts/runtime_school_2023/transcripts/021..023_...Tracing_Part_1..3.txt`

## Related
- `wiki/concepts/tracing.md` — umbrella concept.
- `wiki/concepts/trace-recording.md` — first-pass mechanism.
- `wiki/concepts/trace-replay.md` — subsequent-pass mechanism.
- `wiki/concepts/static-tracing.md` — compiler-asserted sibling, mostly deprecated.
- `wiki/concepts/automatic-tracing.md` — runtime-detected sibling, no application markers.
- `wiki/concepts/operation-pipeline.md` — which stages get memoized.

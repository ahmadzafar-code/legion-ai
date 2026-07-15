---
title: Trace Recording
slug: trace-recording
summary: The first-pass mechanism inside a trace; captures the physical analysis as a sequence of Realm-operation "instructions" plus preconditions/postconditions/anticonditions on equivalence sets, packaged as a physical template.
tags: [tracing, dependence-analysis, instances, for-perf-debug]
subsystem: legion
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/youtube_transcripts/runtime_school_2023/transcripts/022_Legion_Runtime_Internals_-_Lesson_23_-_Tracing_Part_2.txt
  - raw/youtube_transcripts/runtime_school_2023/transcripts/023_Legion_Runtime_Internals_-_Lesson_24_-_Tracing_Part_3.txt
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion_trace.h
related:
  - wiki/concepts/dynamic-tracing.md
  - wiki/concepts/trace-replay.md
  - wiki/concepts/tracing.md
  - wiki/concepts/physical-analysis.md
  - wiki/concepts/equivalence-set.md
---

## TL;DR
Trace recording is what happens on the first pass through a `begin_trace`/`end_trace` block (or after invalidation): the runtime instruments physical analysis to *record* the Realm operations being issued and to *capture* the preconditions, postconditions, and anticonditions on every equivalence set the trace touches. The output is a **physical template** — a list of "instructions" (Realm operations) plus three frontier sets per equivalence set. The confusion: recording is *not free*; it runs alongside the full physical analysis. The savings only show up on replay.

## Mental model
Trace recording is a video recorder running while a live performance happens. The performance (full physical analysis + Realm event-graph construction) is unaltered; the recorder writes a script (instructions) and a list of stage props that have to be in the right state (preconditions) for a replay to work. On replay, the runtime reads the script and skips re-staging.

## Mechanism & API
**Trigger**: the application opens a trace with `runtime->begin_trace(ctx, id)`; the runtime detects no replayable template covers the current state and switches the trace into **record mode**.

**What's captured** (per Lesson 23–24):

1. **Instructions** — a `std::vector<Instruction*>` inside the `PhysicalTemplate` object. Each instruction is an abstract base class whose `execute()` method, when called on replay, issues a specific Realm operation (copy, fill, reduction, task spawn). The transcript calls this "almost like a very, very simple intermediate representation".

2. **Per-equivalence-set frontiers** stored as **`TraceViewsSet`** objects (`runtime/legion/legion_trace.h`). For each equivalence set the trace touches:
   - **Preconditions** — instances that must already hold valid data on replay (e.g., the first read of a field that wasn't first written inside the trace).
   - **Postconditions** — instances that the trace produces with valid data (effects visible to operations after `end_trace`).
   - **Anticonditions** — instances that must be *invalid* on entry (used for reduction patterns: the trace fills a reduction instance, so it must not already be valid).

3. **Tracing recording happens inside `update_set`** on each equivalence set — the same path normal physical analysis goes through; tracing adds a step that updates the frontier sets after each operation's instances are computed (Lesson 24).

**Reduction-aware recording**:
- A `READ_ONLY` access to a field with no prior in-trace write contributes to **preconditions** for the non-dominated subset (whatever the prior in-trace writes did not cover).
- A `WRITE_DISCARD`/`READ_WRITE` access updates **postconditions**; a write that fully dominates prior writes can **invalidate all but itself**.
- A reduction read of an instance produced by an in-trace fill triggers an **anticondition** (the reduction instance must be invalid on entry to replay correctly).

**Template management**:
- The `PhysicalTrace` holds a bounded vector of templates (~5–10). New records evict the LRU one.
- Records are kept across `begin_trace` invocations of the same `trace_id`; each represents a different precondition set.

**Exit**: `end_trace` closes recording. The frontier sets are now the trace's "interface" — they're what `trace-replay.md` checks against.

## Invariants
- Recording **does not change semantics** — physical analysis runs normally; recording is observation only.
- Per-equivalence-set frontiers are computed **incrementally** during the trace's operations; the final frontier on `end_trace` is the trace's externally-visible postcondition/anticondition set.
- An instruction's `execute()` is required to issue exactly the same Realm operation that physical analysis issued during recording.
- **Tracing post-conditions invalidate all prior copies** when the in-trace write dominates them (Lesson 24).
- The template-vector cap means very-variant workloads will not converge on a stable replay set.

## Performance implications
- Recording **costs more than untraced analysis** — same physical analysis plus instrumentation. Acceptable because it's one-shot.
- Frontier-set updates are per-equivalence-set; many-equivalence-set workloads pay more per recording.
- Recording **memory** is bounded by the template cap × per-template instructions and frontier sets. The lecture notes this can be "somewhat memory intensive".

## Debug signals
- **`-level trace=2`** logs each recording start and which template index was filled.
- **First iteration noticeably slower than steady state** is the visible recording cost.
- **Memory growth proportional to template count** can be tracked with `-DLEGION_GC` + `tools/legion_gc.py`.

## Failure modes
- Template-vector cap exceeded → LRU eviction → potential re-record thrash for highly-variant patterns.

## Source pointers
- **Header**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion_trace.h
- **Implementation**: `runtime/legion/legion_trace.cc`
- **Lectures**: `raw/youtube_transcripts/runtime_school_2023/transcripts/022_..._Tracing_Part_2.txt`, `023_..._Tracing_Part_3.txt`

## Related
- `wiki/concepts/dynamic-tracing.md` — what triggers recording.
- `wiki/concepts/trace-replay.md` — what replays the recorded template.
- `wiki/concepts/tracing.md` — umbrella.
- `wiki/concepts/physical-analysis.md` — recording instruments this pass.
- `wiki/concepts/equivalence-set.md` — frontier sets live per equivalence set.

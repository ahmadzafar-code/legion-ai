---
title: Trace Replay
slug: trace-replay
summary: The fast-path execution of a recorded physical template; checks the template's preconditions against current equivalence-set state, and on a match, issues the recorded Realm-operation "instructions" directly without redoing physical analysis.
tags: [tracing, execution, for-perf-debug]
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
  - wiki/concepts/trace-recording.md
  - wiki/concepts/dynamic-tracing.md
  - wiki/concepts/tracing.md
  - wiki/concepts/physical-analysis.md
  - wiki/concepts/equivalence-set.md
---

## TL;DR
Trace replay is the fast path: when the application re-enters a traced block, the runtime tests each cached **physical template**'s preconditions against the current equivalence-set state. The first matching template "wins" — the runtime walks its instruction list and issues the recorded Realm operations directly, skipping logical analysis (stage 2) and physical analysis (stage 5) entirely. The confusion: a template that *doesn't* match its preconditions doesn't disqualify the trace — the runtime tries the next template, and only falls back to recording a new one when none of the cached templates fits.

## Mental model
Trace replay is a JIT cache lookup: the runtime computes a fingerprint (the trace's preconditions) and tries to find a matching cached compiled artifact (template). On hit, it runs the cached code (instructions). On miss across all templates, it falls back to the slow path (full analysis + record a new template).

## Mechanism & API
**Trigger**: `runtime->begin_trace(ctx, trace_id)` looks up the `LogicalTrace` and its optional `PhysicalTrace`. If templates exist, the runtime enters **replay-attempt mode**.

**Precondition check**:
- For each template in the `PhysicalTrace::templates` vector, test whether the current equivalence-set frontier matches the template's recorded preconditions and anticonditions.
- A match means: all instances the template lists as **preconditions** are currently valid for those points/fields; all instances the template lists as **anticonditions** are currently invalid.
- The runtime iterates templates in some priority order (typically most-recently-replayed first).

**Replay execution**:
- On precondition match, the runtime walks `template->instructions` and calls `instruction->execute()` on each. Each call issues a Realm operation (copy, fill, reduction, task spawn) using cached Realm event handles — no per-op physical-analysis recomputation.
- The result is the same Realm event graph the recording produced, populated with fresh event triggers for this iteration.
- On `end_trace`, the runtime updates the equivalence-set frontiers to the template's **postconditions**.

**Failure to match**:
- If no template's preconditions match, the runtime falls back to **trace-recording.md** for the current iteration, producing a new template (subject to the cap).
- Repeated mismatches with many distinct precondition sets → template thrash; visible as no perf improvement despite tracing being on.

## Invariants
- Replay is **semantics-preserving** by construction: the recorded instructions issue the same Realm operations the recording observed, against equivalence sets in a verified-compatible state.
- The mapper is **not consulted** during replay (no `map_task`, no `select_instance`); the recording's mapping decisions are reused.
- Replay produces fresh Realm events per iteration — only the *structure* of the event graph is cached, not the events themselves.
- Postconditions stamped on equivalence sets after replay must be consistent with the trace's effect — if they aren't, the next operation outside the trace sees stale data (UB).
- The `LRU template eviction` policy means rarely-matched templates may disappear; the next match becomes a re-record.

## Performance implications
- **Replay is dramatically faster than untraced analysis** — typically 10×+ on the dep-analysis cost; on heavily traced loops the whole iteration drops to Realm execution time.
- Under control replication, replay is per-shard — multiplies the win across nodes (paper `dcr2021.pdf`).
- **Precondition-check cost is real** but bounded by template count × per-template precondition complexity. With ~5–10 templates and modest precondition sets, this is small.
- Thrash (no template matches consistently) is a worst-case where tracing **adds overhead** with no replay savings.

## Debug signals
- **`-level trace=2`** logs each replay attempt: "matched template N" / "no match, recording new template" / "invalidating template M".
- **Legion Prof**: traced iterations after the first should show near-zero utility-row activity for the trace's operations. If utility rows stay busy, replay isn't happening.
- **Per-iteration time** should plateau after the first iteration once a template matches. A staircase shape suggests thrash.

## Failure modes
- Template thrash from highly-variant mapping → re-record per iteration; no replay savings. Diagnose via `-level trace=2`.
- Postcondition inconsistency (recording bug) → next operation after the trace reads stale data; visible as wrong output.

## Source pointers
- **Header**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion_trace.h
- **Implementation**: `runtime/legion/legion_trace.cc`
- **Lectures**: `raw/youtube_transcripts/runtime_school_2023/transcripts/022_..._Tracing_Part_2.txt`, `023_..._Tracing_Part_3.txt`

## Related
- `wiki/concepts/trace-recording.md` — what replay consumes.
- `wiki/concepts/dynamic-tracing.md` — the API that triggers replay.
- `wiki/concepts/tracing.md` — umbrella.
- `wiki/concepts/physical-analysis.md` — the stage replay skips.
- `wiki/concepts/equivalence-set.md` — where preconditions/postconditions are matched.

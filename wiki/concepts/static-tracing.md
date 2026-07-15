---
title: Static Tracing
slug: static-tracing
summary: A compiler-asserted form of tracing where a Regent program annotates that a code region's operation sequence is invariant; the runtime accepts the assertion and replays without per-iteration checks.
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
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion_trace.h
related:
  - wiki/concepts/tracing.md
  - wiki/concepts/dynamic-tracing.md
  - wiki/concepts/automatic-tracing.md
  - wiki/concepts/regent-language.md
---

## TL;DR
Static tracing is the compiler-asserted variant of Legion tracing: the application (typically Regent code emitting C++) marks a code region as having a statically-known invariant operation sequence, and the runtime accepts the assertion without re-checking on each iteration. It's the predecessor to `dynamic-tracing.md`. Today static tracing is **mostly deprecated** in favor of dynamic tracing under dynamic control replication (`control-replication.md`); the bootcamp 2017 / retreat 2024 transcripts note Regent has removed its static-CR implementation, and dynamic tracing now covers the same workloads. The confusion: "static tracing" doesn't mean the trace itself is statically compiled — it means the *assertion that the trace is invariant* comes from a compiler, not the runtime.

## Mental model
Static tracing is the compile-time-asserted version of dynamic tracing: the same memoization machinery, but with the validity check moved to compile time. Where dynamic tracing trusts the user's runtime promise ("this sequence will repeat"), static tracing trusts a compiler's static proof. In practice the runtime treats it as a more aggressive variant of dynamic tracing.

## Mechanism & API
The `begin_trace` API accepts a flag designating the trace as static:
```cpp
// Conceptual; check the runtime header for the exact signature.
runtime->begin_trace(ctx, trace_id, /*logical_only=*/false, /*static=*/true,
                     /*set_of_static_traces=*/...);
```

Static traces additionally take a `set_of_static_traces` pointer in the deprecated API; the Lesson 22 transcript flags this as deprecated and recommends "assume everything we're doing is dynamic tracing today".

**Why deprecated**:
- Static control replication (paper `cr2017.pdf`) was Regent's compile-time SPMD compiler, which paired with static traces to assert SPMD-invariant sequences. Dynamic control replication (`dcr2021.pdf`) supersedes it: the runtime does the same work without compile-time hand-off.
- The bootcamp 2017 / retreat 2024 transcripts both note that Regent has **removed its static control replication** in favor of using dynamic CR from the runtime.
- The static-tracing code path remains in the runtime header but is rarely exercised; new code should use `dynamic-tracing.md`.

**When you'd still encounter it**:
- Legacy Regent or C++ Legion programs that pre-date dynamic CR.
- Reading runtime source code — the static-tracing branches are still present.

## Invariants
- A static trace's invariance is **claimed by the compiler**; the runtime does not check.
- Misuse (a static trace whose sequence actually varies) is undefined behavior with no diagnostic.
- All other tracing invariants from `dynamic-tracing.md` apply.
- The mapper's `memoize` flag still gates participation.

## Performance implications
- **Theoretically** slightly cheaper than dynamic tracing because the runtime skips one layer of checking. In practice the win is marginal vs dynamic tracing today.
- Operates on the same physical-template machinery; same memory cost.

## Debug signals
- **`-level trace=2`** logs identify static-trace branches when active.
- Source code in `runtime/legion/legion_trace.cc` flagged by the Lesson 22 lecturer as messy and likely to change.

## Failure modes
- Using static tracing on a sequence that isn't actually invariant → undefined behavior, no diagnostic.

## Source pointers
- **Header**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion_trace.h
- **Paper (static CR, Regent compiler)**: `raw/publications/pdfs/cr2017.pdf`
- **Paper (DCR, supersedes static path)**: `raw/publications/pdfs/dcr2021.pdf`
- **Lecture (deprecation note)**: `raw/youtube_transcripts/runtime_school_2023/transcripts/021_..._Tracing_Part_1.txt`

## Related
- `wiki/concepts/tracing.md` — umbrella.
- `wiki/concepts/dynamic-tracing.md` — the supported alternative.
- `wiki/concepts/automatic-tracing.md` — runtime-detected variant.
- `wiki/concepts/regent-language.md` — the compiler that historically emitted static-trace calls.

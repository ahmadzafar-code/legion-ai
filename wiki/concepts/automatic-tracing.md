---
title: Automatic Tracing
slug: automatic-tracing
summary: A runtime system (Apophenia, paper `autotrace2025.pdf`) that detects repeating operation patterns in the task stream via online string analysis (suffix arrays + tries) and triggers Legion's tracing engine without application-level `begin_trace`/`end_trace` markers.
tags: [tracing, execution, for-perf-debug]
subsystem: legion
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/publications/pdfs/autotrace2025.pdf
  - raw/publications/publications.md
github:
  - https://github.com/StanfordLegion/legion/tree/master/runtime/legion
related:
  - wiki/concepts/tracing.md
  - wiki/concepts/dynamic-tracing.md
  - wiki/concepts/trace-recording.md
  - wiki/concepts/trace-replay.md
  - wiki/concepts/operation-pipeline.md
  - wiki/pitfalls/missed-tracing-opportunity.md
---

## TL;DR
Automatic tracing is the system **Apophenia** introduced by `autotrace2025.pdf` (ASPLOS 2025). It sits between the application and Legion's dependence analysis, watching the task stream, identifying **repeated sub-strings** via online string analysis (suffix arrays + tries), and wrapping them in `tbegin(id)`/`tend(id)` for Legion's existing tracing engine (`dynamic-tracing.md`) to memoize. Applications get the perf benefit of tracing without writing `begin_trace`/`end_trace` markers. The confusion: this is research-level functionality but **production-tested at scale** — the paper evaluates on S3D, HTR, CFD, TorchSWE, FlexFlow on Perlmutter + Eos supercomputers up to 64 GPUs. Apophenia achieves 0.92-1.03× the perf of manually-traced code (essentially matches), and 0.91-2.82× speedup over previously-untraced applications.

## Mental model
Apophenia is a **JIT compiler for task streams**. JIT compilers watch bytecode, find hot loops, and compile them. Apophenia watches Legion's task stream, finds repeated sub-sequences, and replaces them with traced replay. The same architecture: slow general path (full dep-analysis) running by default; fast specialized path (replay) for sequences identified as repeating; an online analysis component (the "compiler") that decides what to specialize.

## Mechanism & API

Per `autotrace2025.pdf` §3-4:

**Token stream**: each Legion task call is hashed to a fixed-size token (the task ID + region arguments + privileges + sharding metadata, all hash to a 128-bit token). The application's runtime behavior is reduced to a stream of tokens; trace identification is sub-string matching on this stream.

**Two architectural components**:

1. **TraceFinder**: buffers incoming tokens. Periodically, asynchronously runs the **non-overlapping repeated sub-string algorithm** (Algorithm 2 in the paper) over a slice of the buffer. The algorithm:
   - Builds a **suffix array** + LCP array for the buffer slice.
   - Iterates adjacent suffix-array entries; pairs without overlap become **candidate repeats**.
   - Sorts candidates by length (longest first) and greedily selects non-overlapping candidates.
   - Output: a set of repeated sub-strings with high coverage. Time: O(n log n).
   The candidates feed a **trie** of currently-tracked patterns.

2. **TraceReplayer**: as each new token arrives, walks the trie of candidates. If the stream matches a candidate, dispatches `tbegin(id)` + the recorded tasks + `tend(id)` to Legion's tracing engine instead of the raw tasks. Includes a **scoring function** balancing: trace length, repeat count, time-since-last-seen, with bias toward already-replayed traces.

**Buffer sampling — the ruler function** (`autotrace2025.pdf` §4.4): the candidate-finding pass runs over sub-slices of the token buffer, not the whole buffer. The choice of slice follows the **ruler sequence** (the count of times each integer divides by 2: `0, 1, 0, 2, 0, 1, 0, 3, ...`). This gives a sampling pattern that is fast-to-respond for short repeats and still able to detect long repeats periodically. Time complexity: O(n log² n) total.

**Distributed operation** (`autotrace2025.pdf` §5.1): Apophenia uses Legion's `control-replication.md` so each shard runs an instance of Apophenia. To maintain control determinism, all shards must agree on which traces to replay — Apophenia includes a coordination protocol where nodes agree on a count of processed operations before issuing replay results.

**Validation** (`autotrace2025.pdf` §6): Apophenia is correct because Legion's tracing engine validates trace replays itself — Apophenia just *proposes* traces; the runtime rejects invalid ones and falls back to full analysis. There's no application correctness risk.

## Invariants
- Apophenia is **correct by construction**: it only invokes Legion's existing tracing engine. If the proposed trace is invalid (sequence diverges from the recorded one), Legion's engine detects it and falls back to full analysis.
- Apophenia's analysis is **asynchronous and non-blocking** — it runs on Legion's background worker threads and never stalls the application's task launch path.
- The token-buffer size + sampling strategy is fixed at startup; no application tuning needed.
- Under DCR, all shards run Apophenia locally; the cross-shard agreement protocol ensures replicated programs make the same trace decisions.
- Traces Apophenia issues are **subject to the same invariants as manual traces** (`dynamic-tracing.md`): structurally identical sequences, stable region requirements.

## Performance implications
Per the paper's evaluation (`autotrace2025.pdf` §6):

- **Previously-traced applications** (S3D, HTR, FlexFlow): Apophenia matches manual tracing within 0.92-1.03×. The cost of automatic identification is roughly offset by occasional better trace selection.
- **Previously-untraced applications** (CFD, TorchSWE — cuPyNumeric applications): Apophenia provides 0.91-2.82× speedup over untraced. These applications couldn't be manually traced because their loop structure didn't match Legion's trace requirements.
- **Per-task overhead**: 7 µs without Apophenia, 12 µs with — negligible relative to the 100 µs cost of replaying a single task.
- **Warmup**: 30-300 iterations for Apophenia to find a steady state (problem-dependent). For long-running workloads this is negligible; for short benchmarks it dominates measurement.
- **Memory**: bounded by the token buffer + trie size — small relative to application data.

## Debug signals
- **Iteration time stabilizes after a warmup period** = Apophenia found a steady state. The number of warmup iterations is workload-dependent (S3D: 50, HTR: 50, CFD: 300, TorchSWE: 300, FlexFlow: 30).
- **`-level trace=2`** logs Apophenia's identification + replay decisions.
- **`legion-prof.md`** utility-row activity should drop after warmup; automated traces show up like manual ones.
- **Enable / disable**: Apophenia is opt-in via a runtime flag (check `-help` on your Legion version for the current spelling).

## Failure modes
- Application's task stream lacks any repeated sub-strings → Apophenia finds nothing; no perf change vs. untraced.
- Region allocator churn (e.g., cuPyNumeric arrays rebound to different region IDs each iteration) → tokens vary and repeats aren't found. Steady state requires application cooperation (stable region IDs across iterations).

## Source pointers
- **Paper**: `raw/publications/pdfs/autotrace2025.pdf` — ASPLOS 2025, *Automatic Tracing in Task-Based Runtime Systems* (Yadav, Bauer, Broman, Garland, Aiken, Kjolstad).
- **String-matching reference impl**: https://github.com/david-broman/matching-substrings (cited in §4.2).
- **Runtime tree**: https://github.com/StanfordLegion/legion/tree/master/runtime/legion

## Related
- `wiki/concepts/tracing.md` — umbrella.
- `wiki/concepts/dynamic-tracing.md` — the engine Apophenia targets.
- `wiki/concepts/trace-recording.md` / `wiki/concepts/trace-replay.md` — what happens internally on each trace.
- `wiki/concepts/operation-pipeline.md` — Apophenia sits at the front of stage 1.
- `wiki/pitfalls/missed-tracing-opportunity.md` — the pitfall this system aims to eliminate.

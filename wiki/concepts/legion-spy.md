---
title: Legion Spy
slug: legion-spy
summary: Legion's dependence-graph visualizer and correctness verifier; renders the logical operation DAG and the post-mapping Realm event graph, and optionally re-checks the runtime's analyses.
tags: [debugging, profiling, tooling, for-correctness-debug, for-perf-debug]
subsystem: cross
layer: tooling
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/website-pages/debugging.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/tools/legion_spy.py
related:
  - wiki/concepts/operation-pipeline.md
  - wiki/concepts/privilege.md
  - wiki/concepts/event.md
  - wiki/concepts/legion-prof.md
  - wiki/concepts/freeze-on-error.md
  - wiki/concepts/partition-checks.md
---

## TL;DR
Legion Spy answers "why did the runtime decide *that*?". It ingests `-lg:spy` logs and produces (a) the dataflow graph of logical operations and the privileges connecting them, (b) the Realm event graph after mapping, and (c) optionally re-runs the runtime's logical/physical/mapping analyses for correctness verification. Use Legion Prof for *when*; use Legion Spy for *why*. The confusion: visualization mode (`-dez`) is cheap and always-on for debugging; checking mode (`-lpa` with `-DLEGION_SPY`) is slow and only for correctness deep-dives.

## Mental model
If Legion Prof is the timeline view of an OOO processor, Legion Spy is the data-flow / dependence-graph view. Each node is an operation (task, copy, fill, fence); each edge is a dependence the runtime detected via `privilege.md` + coherence + field analysis. Edges that "shouldn't be there" are false dependencies — i.e., your over-broad privileges or aliased partitions made the runtime serialize work it could have parallelized.

## Mechanism & API
**Visualization mode (cheap):**
```bash
./app -lg:spy -logfile spy_%.log
legion/tools/legion_spy.py -dez spy_*.log
```
Flags:
- `-d` — emit the **dataflow graph** (logical operations).
- `-e` — emit the **event graph** (post-mapping Realm structure).
- `-z` — read gzipped logs.

Output is a directory of `.pdf`/`.png` graphs viewable in any image viewer.

**Checking mode (expensive, correctness-only):**
```bash
CC_FLAGS=-DLEGION_SPY make
./app -lg:spy -logfile spy_%.log
legion/tools/legion_spy.py -lpa spy_*.log    # then -dez for visuals
```
Flags:
- `-l` — re-run **logical analysis**; verifies dep-analysis matches the runtime's.
- `-p` — re-run **physical analysis**; verifies instance and copy decisions.
- `-a` — verify **mapping dependence** analysis.

Compiling with `-DLEGION_SPY` enables extra logging needed for checking mode; do not use in production builds.

**Naming objects for readable graphs:**
```cpp
runtime->attach_name(is, "input_is");
runtime->attach_name(lr, "input_lr");
runtime->attach_name(ip, "ip");
```

## Invariants
- The dataflow graph reflects the **logical** structure (what the runtime saw at stage 2 of the pipeline); the event graph reflects the **physical** structure after stage 5.
- Visualization mode requires only the `-lg:spy` runtime flag.
- Checking mode requires recompilation with `-DLEGION_SPY` and runs slowly (often 10–100× slower).
- Each spy log file corresponds to one node; pass them all to `legion_spy.py` together.
- Naming objects (`attach_name`) only changes the rendered labels; it has no semantic effect.

## Performance implications
- `-lg:spy` adds non-trivial runtime overhead (it logs every operation); do not pair with Legion Prof when measuring perf — measure separately.
- `-DLEGION_SPY` compile path is much slower than regular debug; only use for correctness investigations.
- `tools/legion_spy.py` itself is single-threaded Python; large logs (millions of ops) can take minutes.

## Debug signals (what to look for)
- **Excess edges** between two tasks that should be parallel → over-broad privileges (`privilege.md`) or aliased partitions (`partition.md`).
- **Missing edges** where you expect dependence → wrong region requirement or wrong field set.
- **Critical path** drawn by `-d` matches Legion Prof's critical-path view; mismatch indicates physical analysis added dependencies the logical view didn't predict.
- In **checking mode**: any error printed by `-l`/`-p`/`-a` is a runtime correctness bug or an application invariant violation worth reporting.

## Failure modes
- [False dependencies from over-broad privileges](../pitfalls/false-dependencies-overbroad-privileges.md) — Spy's dataflow graph is how you confirm.
- [Non-disjoint "disjoint" partition](../pitfalls/non-disjoint-disjoint-partition.md) — visible as cross-edges between sibling point tasks.

## Source pointers
- **Tool**: https://github.com/StanfordLegion/legion/blob/master/tools/legion_spy.py
- **Reference**: https://legion.stanford.edu/debugging/ (mirrored at `raw/website-pages/debugging.md`)

## Related
- `wiki/concepts/operation-pipeline.md` — what Spy is visualizing.
- `wiki/concepts/privilege.md` — what the dataflow edges encode.
- `wiki/concepts/event.md` — what the event graph nodes are.
- `wiki/concepts/legion-prof.md` — sibling tool; Prof is timeline, Spy is causality.
- `wiki/workflows/debug-perf-bottleneck.md` — when to reach for Spy.

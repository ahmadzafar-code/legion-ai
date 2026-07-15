---
title: Dataflow Graph
slug: dataflow-graph
summary: Legion Spy's `-d` output; renders the post-logical-analysis operation DAG with privilege-annotated edges; the operation-level causality view used for diagnosing false dependencies.
tags: [profiling, debugging, tooling, for-perf-debug, for-correctness-debug]
subsystem: cross
layer: tooling
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/website-pages/debugging.md
  - raw/youtube_transcripts/runtime_school_2023/transcripts/007_Legion_Runtime_Internals_-_Lesson_7_-_Logical_Dependence_Analysis.txt
github:
  - https://github.com/StanfordLegion/legion/blob/master/tools/legion_spy.py
related:
  - wiki/concepts/legion-spy.md
  - wiki/concepts/logical-analysis.md
  - wiki/concepts/event-graph.md
  - wiki/concepts/privilege.md
  - wiki/concepts/non-interference.md
---

## TL;DR
The dataflow graph is Legion Spy's `-d` output: a `dot` rendering of the **logical operation DAG** that pipeline-stage-2 produced. Each node is one *logical* operation (a task launch, a copy, a fill, an index launch — even with N points, it's one node), and each edge is a logical dependence with its privilege/field reason. The confusion: index launches show as **one node** here, not N. To see per-point precision you need `event-graph.md` (Spy's `-e`). The dataflow graph is where you go to confirm false dependencies — every edge says "these two ops conflict per logical analysis", and edges that shouldn't exist are the bug.

## Mental model
The dataflow graph is the program's *intent* through Legion's eyes — what the source code's launch order plus region requirements look like after logical analysis has computed non-interference. Where `event-graph.md` is what Realm actually executed, the dataflow graph is the operation-level shape the runtime decided. Optimizing perf almost always means making this graph wider (more parallel edges from one node) and shorter (fewer chained edges).

## Mechanism & API
**Capture** (per `raw/website-pages/debugging.md`):
```bash
./app -lg:spy -logfile spy_%.log
legion/tools/legion_spy.py -dz spy_*.log
```

Flags:
- `-d` — emit the dataflow graph.
- `-z` — read gzipped logs.
- Combine with `-e` (`-dez`) to also emit `event-graph.md`.

Output: `dot` file + rendered `.pdf`/`.png` in the output directory.

**Reading the graph** (Runtime School Lesson 7 walks an example):
- Nodes are *logical* operations: tasks, index-launch operations (one node, not N), explicit copies, fills, fences, close ops.
- Edges are dependencies the runtime added during logical analysis. Edge labels typically include the conflicting field set and privilege.
- An index-launch operation is one node — but it can have multiple incoming edges representing dependencies from multiple prior writers covering different points.
- Close operations and inter-close ops show up where the runtime had to insert them; see Lesson 7 for the example.

**What conclusions you draw from the graph**:
- An **edge between two tasks you expected to be parallel** = a false dependence. Trace the labelled privilege/field to find the over-broad declaration.
- A **long chain of single-edge dependencies** = `pitfalls/long-dependence-chains.md`.
- **Inter-close ops between launches** = the runtime is forcing a privilege boundary; check if it was intentional.

**Pairing with the event graph**:
- A clean dataflow graph and a noisy event graph = physical-analysis-level cost (instance placement, copies). Investigate `mapper.md`.
- Both graphs heavy = real serialization in the program; restructure or improve privileges.

## Invariants
- The dataflow graph reflects **logical analysis output** (`logical-analysis.md`) — the same thing the runtime checks during stage 2 of the operation pipeline.
- Logical analysis is sound but imprecise (Lesson 7); the dataflow graph may show edges that physical analysis later refines away per-point. But it never *misses* a real dependency.
- Index launches: **one node**, regardless of point count. To see per-point structure, use the event graph.
- Edge labels show the conflicting **privilege + coherence + field set** — directly usable for fixing.
- Naming via `attach_name` improves readability; otherwise nodes show runtime-generated IDs.

## Performance implications
- The dataflow graph is the **first place to confirm false dependencies** suspected from Legion Prof. Each unexpected edge is a specific fix opportunity.
- **`WRITE_DISCARD`** eliminates RAW edges to prior writers (the canonical perf-improving change).
- **Disjoint partitions** eliminate edges between point tasks; aliased ones add them.
- **`REDUCE` with same op** eliminates edges between concurrent reducers.

## Debug signals
- **Unexpected edge** between two operations → over-broad privilege, over-broad region, or unintended coherence. Cross-reference with `privilege.md` and `non-interference.md`.
- **Sibling point tasks of an index launch appear as one node but their downstream edges fan out** → use the event graph for per-point precision.
- **Inter-close ops appearing between launches** → privilege transitions you may not have realized you triggered; see Lesson 7 for examples.

## Failure modes
- Building large graphs for long-running programs → render times grow super-linearly. Constrain.
- Reading the graph without `attach_name` calls → nodes labelled with cryptic IDs; add naming and re-run.

## Source pointers
- **Tool**: https://github.com/StanfordLegion/legion/blob/master/tools/legion_spy.py
- **Reference**: `raw/website-pages/debugging.md`
- **Lecture (graph walkthrough)**: `raw/youtube_transcripts/runtime_school_2023/transcripts/007_..._Logical_Dependence_Analysis.txt`

## Related
- `wiki/concepts/legion-spy.md` — host tool.
- `wiki/concepts/logical-analysis.md` — the stage whose output this is.
- `wiki/concepts/event-graph.md` — Spy's per-point complement.
- `wiki/concepts/privilege.md` — what edge labels carry.
- `wiki/concepts/non-interference.md` — the predicate behind every edge's presence/absence.

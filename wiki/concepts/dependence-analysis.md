---
title: Dependence Analysis
slug: dependence-analysis
summary: The runtime's process for computing which operations conflict and must be ordered; split into a fast operation-granularity logical pass (stage 2) and a precise point-granularity physical pass (stage 5).
tags: [dependence-analysis, execution, for-program-reasoning, for-perf-debug]
subsystem: legion
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/youtube_transcripts/runtime_school_2023/transcripts/007_Legion_Runtime_Internals_-_Lesson_7_-_Logical_Dependence_Analysis.txt
  - raw/youtube_transcripts/runtime_school_2023/transcripts/009_Legion_Runtime_Internals_-_Lesson_9_-_Physical_Analysis_Part_1.txt
  - raw/publications/publications.md
github:
  - https://github.com/StanfordLegion/legion/tree/master/runtime/legion
related:
  - wiki/concepts/operation-pipeline.md
  - wiki/concepts/logical-analysis.md
  - wiki/concepts/physical-analysis.md
  - wiki/concepts/privilege.md
  - wiki/concepts/coherence-mode.md
  - wiki/concepts/tracing.md
  - wiki/concepts/visibility-algorithm.md
  - wiki/concepts/non-interference.md
---

## TL;DR
Dependence analysis is what makes Legion programs implicitly parallel. The runtime examines every operation's region requirements and decides which prior operations it conflicts with. There are two passes: **logical analysis** (pipeline stage 2) runs in program order, is operation-granular, and is sound-but-imprecise; **physical analysis** (stage 5) runs after mapping, is point-and-instance precise, and computes the actual Realm event graph. The confusion: an "edge" in the Legion Spy dataflow graph reflects logical analysis; the per-point dependencies happen later and only show up in the physical (event) graph.

## Mental model
Two-pass compilation: logical analysis is the front-end pass that decides *which operations must wait for which* without yet knowing where instances live; physical analysis is the back-end pass that, given the mapper's placement, computes the precise copies and event chains. The two-pass structure is critical to performance: logical analysis runs cheaply in program order and unlocks parallel execution of physical analysis (and mapping) for independent operations.

## Mechanism & API
**Pass 1 — Logical analysis (stage 2)**
- Operates on the region tree (`runtime/legion/region_tree.cc`).
- Walks operation's region requirements; for each, checks against the tree of currently-outstanding operations.
- Compares by region overlap, field overlap, privilege, and coherence to compute non-interference.
- Runs operations in **program order within a parent context** (a hard invariant).
- Output: a partial order over operations (the "operation DAG") consumed by stages 3–4.

**Pass 2 — Physical analysis (stage 5)**
- After the mapper has chosen instances, the runtime calls `perform_versioning_analysis` to find the **equivalence sets** for the operation's regions. See `wiki/concepts/physical-analysis.md` for what an equivalence set is.
- Then `physical_perform_updates_and_registration` issues the copies/reductions/fills needed to make the chosen instances valid.
- Returns Realm events representing per-point preconditions.

Both passes can be **memoized** by tracing (`wiki/concepts/tracing.md`): logical analysis (stage 2) and physical analysis (stage 5) are exactly the stages a dynamic trace caches.

## Invariants
- Logical analysis is **sound but imprecise**. Two operations marked dependent may turn out to not actually conflict at the per-point level; the precise check happens in physical analysis. Logical never falsely says "independent" — that direction would break correctness.
- Logical analysis runs in **program order** within a context; no reordering, no concurrency. Different contexts (e.g., subtask launches under different parents) can run logical analysis concurrently.
- Logical analysis is at **operation granularity**: an `IndexLauncher` is one logical node, even though it expands to N point tasks. Per-point dependencies emerge only in physical analysis.
- Physical analysis is **deferred and asynchronous**: it can begin only once mapping has chosen instances; multiple operations' physical analyses run in parallel when their logical predecessors permit.
- Both passes consume `privilege.md` + `coherence-mode.md` + the region/field set. Change any of them and you change the analysis result.

## Performance implications
- Logical analysis cost scales with `#operations × #region requirements × tree depth`. Over-launching tiny tasks here costs more than executing them. Use `IndexLauncher` (see `index-space-launch.md`) — one logical node, not N.
- Logical analysis being *imprecise* means false dependences are real performance bugs — see `pitfalls/false-dependencies-overbroad-privileges.md`.
- Physical analysis cost scales with `#equivalence sets touched × #fields × #valid instances`. Hierarchical and dependent partitioning can blow this up if the equivalence-set partition is fragmented.
- **Tracing collapses both passes** on repeated patterns — often the single biggest perf win in iterative codes. See `tracing.md`.

## Debug signals
- **Legion Spy dataflow graph** (`-lg:spy` → `legion_spy.py -d`): shows the logical-analysis output. Edges between supposedly-parallel operations indicate false dependences.
- **Legion Spy event graph** (`-lg:spy` → `legion_spy.py -e`): shows the physical-analysis output. This is the precise per-point view.
- **`-level legion=2`**: logs per-operation transitions through stages 2 and 5.
- **Legion Prof utility-processor rows**: heavy activity here, especially before stage 4 (mapping), points at logical-analysis cost.

## Failure modes
- [Long dependence chains](../pitfalls/long-dependence-chains.md) — usually a logical-analysis-visible problem.
- [Runtime overhead dominates](../pitfalls/runtime-overhead-dominates.md) — dependence analysis runs more often than execution.

## Source pointers
- **Runtime (analysis lives here)**: https://github.com/StanfordLegion/legion/tree/master/runtime/legion
- **Lectures (deep dive)**: `raw/youtube_transcripts/runtime_school_2023/` Lessons 7–14
- **Paper (correctness)**: `raw/publications/pdfs/dep2018.pdf` — *Correctness of Dynamic Dependence Analysis*
- **Paper (visibility algorithms)**: `raw/publications/pdfs/visibility2023.pdf`

## Related
- `wiki/concepts/operation-pipeline.md` — where the two passes sit in the 7-stage pipeline.
- `wiki/concepts/logical-analysis.md` — pass 1 deep dive.
- `wiki/concepts/physical-analysis.md` — pass 2 deep dive.
- `wiki/concepts/privilege.md` — the dominant input.
- `wiki/concepts/coherence-mode.md` — the secondary input.
- `wiki/concepts/tracing.md` — how both passes get memoized.

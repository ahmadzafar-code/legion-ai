---
title: Logical Analysis
slug: logical-analysis
summary: Pipeline stage 2; the operation-granularity, program-ordered, sound-but-imprecise pass that computes which Legion operations must be ordered relative to each other.
tags: [dependence-analysis, execution, for-perf-debug, for-program-reasoning]
subsystem: legion
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/youtube_transcripts/runtime_school_2023/transcripts/007_Legion_Runtime_Internals_-_Lesson_7_-_Logical_Dependence_Analysis.txt
  - raw/youtube_transcripts/runtime_school_2023/transcripts/008_Legion_Runtime_Internals_-_Lesson_8_-_Logical_Dependence_Analysis_Part_2.txt
  - raw/publications/publications.md
github:
  - https://github.com/StanfordLegion/legion/tree/master/runtime/legion
related:
  - wiki/concepts/operation-pipeline.md
  - wiki/concepts/dependence-analysis.md
  - wiki/concepts/physical-analysis.md
  - wiki/concepts/privilege.md
  - wiki/concepts/tracing.md
  - wiki/concepts/region-tree.md
  - wiki/concepts/non-interference.md
---

## TL;DR
Logical analysis is the first dependence-analysis pass — pipeline stage 2. It runs in program order within a parent context, walks each operation's region requirements through the region tree, and records dependencies on currently-outstanding operations whose requirements interfere. The output is a partial order over operations that controls when each may proceed to mapping (stage 4). The confusion: it works at **operation granularity** — a single `IndexLauncher` is one logical node regardless of how many points it will expand into.

## Mental model
> "Tasks that potentially have things happening in parallel here, could potentially run through this mapping stage in parallel. So these dependencies are actually important — we want to find some parallelism in there." — Runtime School 2023, Lesson 7.

Picture logical analysis as the decode/rename stage of an OOO processor: take an instruction (operation), look at its operands (region requirements), check against the in-flight set, mark RAW/WAR/WAW hazards. It's program-order in, partial-order out.

## Mechanism & API
- Triggered when an operation arrives at stage 2 of `operation-pipeline.md`.
- Walks each region requirement through the region-tree node for the named (sub)region.
- At each node, checks the operation's privilege + coherence + field set against the recorded "epoch" of prior users at that node.
- Computes **interference** by ANDing region overlap, field overlap, privilege/coherence conflict.
- Records a dependency edge to each interfering prior operation; the new operation cannot proceed past stage 3 until those predecessors have themselves been mapped.
- Output is consumed by stages 3–7.

Critical structural fact: logical analysis processes operations **strictly in program order** within a parent context. The runtime guarantees this for correctness — it's how the application's sequential-program illusion is preserved.

## Invariants
- **Program order within a context.** Subtasks issued from the same parent run through logical analysis in launch order.
- **Operation granularity.** An `IndexLauncher` is one node in the logical graph, not N. Per-point dependencies are computed later in `physical-analysis.md`.
- **Sound but imprecise.** Logical analysis may report two operations as dependent when no per-point conflict actually exists. The reverse direction never holds — logical never says "independent" when there's a real per-point conflict. (Lesson 7.)
- Logical analysis output is what `legion_spy.py -d` renders as the dataflow graph.
- Multiple contexts run logical analysis concurrently — only the same-parent ordering is sequential.

## Performance implications
- **Linear in `#operations × #region requirements × region-tree depth`**. The dominant runtime overhead in fine-grained Legion programs.
- Use `IndexLauncher` to collapse N point tasks into 1 logical operation (see `index-space-launch.md`).
- False dependencies from over-broad privileges (`pitfalls/false-dependencies-overbroad-privileges.md`) inflate the logical graph; narrow privileges to keep it sparse.
- **Tracing memoizes this pass** for repeated patterns. Without tracing, an iterative code re-runs logical analysis on every iteration. See `tracing.md`.
- Under control replication, logical analysis is **partitioned across shards** — each shard runs only its slice of the analysis. See `control-replication.md`.

## Debug signals
- **Legion Spy `-d`**: the dataflow graph IS the logical-analysis output. Edges you don't expect = over-broad privileges or aliased partitions.
- **Legion Prof utility rows** active before stage 4 (mapping) starts → logical analysis is the bottleneck.
- **`-level legion=2`**: per-op stage transitions logged; "logical_analysis done" timestamps reveal where time goes.

## Failure modes
- [False dependencies from over-broad privileges](../pitfalls/false-dependencies-overbroad-privileges.md) — the canonical logical-analysis pitfall.
- [Long dependence chains](../pitfalls/long-dependence-chains.md) — visible in the dataflow graph.
- [Runtime overhead dominates](../pitfalls/runtime-overhead-dominates.md) — logical analysis cost exceeds execution.

## Source pointers
- **Runtime tree (region_tree, legion_ops)**: https://github.com/StanfordLegion/legion/tree/master/runtime/legion
- **Lectures**: `raw/youtube_transcripts/runtime_school_2023/` Lessons 7–8
- **Paper (correctness)**: `raw/publications/pdfs/dep2018.pdf`

## Related
- `wiki/concepts/operation-pipeline.md` — stage 2 is logical analysis.
- `wiki/concepts/dependence-analysis.md` — the umbrella concept.
- `wiki/concepts/physical-analysis.md` — stage 5; the precise per-point counterpart.
- `wiki/concepts/privilege.md` — the dominant interference input.
- `wiki/concepts/tracing.md` — how this pass gets cached.

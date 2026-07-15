---
title: Critical Path
slug: critical-path
summary: Legion Prof's overlay (press `a`) that draws the longest chain of dependent operations through the application; the timeline you have to shorten to make the program faster regardless of how many processors you add.
tags: [profiling, tooling, for-perf-debug]
subsystem: cross
layer: tooling
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/website-pages/profiling.md
  - raw/youtube_transcripts/bootcamp_2017/transcripts/008_Advanced_Profiling_-_Legion_Bootcamp_2017_8_of_10.txt
github:
  - https://github.com/StanfordLegion/legion/tree/master/tools/legion_prof_rs
related:
  - wiki/concepts/legion-prof.md
  - wiki/concepts/legion-spy.md
  - wiki/concepts/operation-pipeline.md
  - wiki/workflows/debug-perf-bottleneck.md
  - wiki/concepts/timeline-view.md
  - wiki/concepts/event-graph.md
---

## TL;DR
The critical path is the longest dependent chain of operations through a Legion application's execution. Press `a` in Legion Prof and the viewer overlays this chain on the timeline — the path along which every gap and every long task directly extends wall-clock time. The confusion: critical path is not the longest *task* — it's the longest *chain*. A long task off the critical path doesn't matter to total runtime; a short task on the critical path with a long dependency-wait gap before it does.

## Mental model
The critical path is the slack-free chain in your task DAG — the operations whose completion event triggers the next without any wait. Where Gantt-chart project planning has "tasks on the critical path", Legion Prof's critical path is the same idea applied to the operation DAG: shortening any task or gap *on* the chain reduces total runtime by exactly that amount.

## Mechanism & API
In Legion Prof:
- **Press `a`** in the UI to toggle the critical-path overlay. The chain is highlighted on the relevant processor/channel/memory rows.
- Critical path is computed from the **post-mapping event graph** — i.e., what Realm actually executed, including the copies physical analysis inserted.
- Pairing with **Legion Spy** annotates the critical path with dependence reasons: `-lg:spy` + `legion_spy.py -dez` then load both into the profiler.

**Critical-path elements** (per `raw/website-pages/profiling.md`):
- A long task **on** the chain → optimize that kernel.
- A long copy bar **on** the chain → revisit instance placement (`mapper.md`).
- An idle gap **before** a task → upstream took too long to ready it; trace upstream.
- A short task with no preceding gap → no win available there.

The runtime exposes three perf factors the critical path measures (`raw/website-pages/profiling.md`):
1. **Overall throughput** — sum of work across all paths; not the critical path's concern.
2. **Critical-path latency** — what you see when you press `a`. This is the floor on total time given unlimited processors.
3. **Runtime overhead** — utility-processor activity. Often appears as gaps along the critical path if mapping is slow.

## Invariants
- Critical path is **derived from the actual run** — it reflects this specific execution's mapping, tracing, and event ordering.
- Critical path is the **post-mapping** chain; logical dependence analysis can predict some of it but not all (mapping decisions, instance placement, and copy emission shape the actual chain).
- Reducing critical-path length **always** reduces total runtime. Reducing off-critical work does not, until enough off-critical work is reduced to expose a new shorter critical path.
- Adding processors **cannot help** a critical path that's already serialized — only restructuring the DAG (index launches, tracing, reduced false dependencies) helps.
- Critical path can change between runs if mapping, tracing, or non-determinism varies; rerun to confirm.

## Performance implications
- **First thing to look at in any Legion Prof profile.** Identifies the floor on wall-clock improvement.
- Each element on the critical path is a tractable target — a specific task, copy, or gap with a specific cause.
- Critical-path elements on **channel rows** point at `pitfalls/excessive-data-movement.md`; on **utility rows** at `pitfalls/mapper-stalls.md` or `pitfalls/runtime-overhead-dominates.md`; as **long chains of tasks** at `pitfalls/long-dependence-chains.md`.
- Pairing with Legion Spy turns each edge into a "why does B wait for A?" question with an answer.

## Debug signals
- **Critical path drawn as a single chain through one row** → no parallelism; investigate index launches and false dependencies.
- **Critical path zigzags between rows** = normal; reflects cross-processor dependencies (data movement or sync).
- **Critical path dominated by utility rows** → runtime overhead exceeds compute; coarsen tasks or enable tracing.
- **Critical path includes a long copy bar** → mapper placed instances suboptimally.

## Failure modes
- Looking only at *busy* rows and ignoring the critical-path overlay is the most common Legion Prof mistake — busy rows say "where work happened", critical path says "what determined total time".

## Source pointers
- **Profiler source**: https://github.com/StanfordLegion/legion/tree/master/tools/legion_prof_rs
- **Reference**: https://legion.stanford.edu/profiling/ (mirrored at `raw/website-pages/profiling.md`)
- **Demo**: `raw/youtube_transcripts/bootcamp_2017/transcripts/008_Advanced_Profiling_-_Legion_Bootcamp_2017_8_of_10.txt`

## Related
- `wiki/concepts/legion-prof.md` — host tool.
- `wiki/concepts/legion-spy.md` — pair for dependence-reason annotations.
- `wiki/concepts/operation-pipeline.md` — what the chain runs through.
- `wiki/workflows/debug-perf-bottleneck.md` — decision tree starting from "look at the critical path".

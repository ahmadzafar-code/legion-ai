---
title: Long Dependence Chains
slug: long-dependence-chains
summary: The critical path is dominated by a long sequential chain of dependent tasks; the application has no fan-out, an aliased partition, false dependencies, or unused tracing.
tags: [for-perf-debug, dependence-analysis, parallelism]
status: draft
created: 2026-05-15
updated: 2026-05-15
related:
  - wiki/concepts/operation-pipeline.md
  - wiki/concepts/privilege.md
  - wiki/concepts/partition.md
  - wiki/concepts/tracing.md
  - wiki/concepts/legion-prof.md
  - wiki/concepts/legion-spy.md
---

## Symptom
- Press `a` in **Legion Prof** to view the critical path: it is a single long chain instead of a wide fan-out.
- Adding processors doesn't reduce wall-clock time.
- The **Legion Spy** dataflow graph is mostly a left-to-right chain rather than a layered DAG.

## Cause
Five common causes — diagnose by reading the chain in the dataflow graph and asking why each edge exists:

1. **No index launch.** A `for` loop calling `execute_task` in series produces N operations with no implicit parallelism. Each launch costs runtime overhead and they serialize on whatever shared state they touch.
2. **False dependencies from over-broad privileges.** See [false-dependencies-overbroad-privileges](false-dependencies-overbroad-privileges.md). The dataflow graph shows edges between tasks that should be parallel.
3. **Aliased partition where a disjoint one was intended.** Subregions overlap → point tasks of an index launch serialize. See [non-disjoint-disjoint-partition](non-disjoint-disjoint-partition.md).
4. **Repeated loop body without tracing.** Even when the structure is parallel within an iteration, repeated re-analysis of each iteration extends the critical path. See [missed-tracing-opportunity](missed-tracing-opportunity.md).
5. **Logically sequential algorithm.** Sometimes the program really is sequential; algorithmic change is the only fix (e.g., tree-reduce instead of left-fold).

## Fix
- **Replace sequential launches with `IndexLauncher`.** Define a color space, partition the regions, launch the work as one index task. The N operations collapse to 1 operation with N points.
- **Narrow privileges and split fields** where false edges appear in the Spy graph. See `privilege.md`.
- **Run with `-lg:partcheck`** to confirm disjointness when an "obvious" partition still serializes.
- **Wrap repeating loop bodies in `runtime->begin_trace`/`end_trace`** with a stable `trace_id`. See `tracing.md`.
- **Increase `-lg:window`** to let more operations issue ahead, but understand that this expands the *issue* window without changing the *execution* DAG — it helps overlap analysis with execution, not parallelism.

## Underlying concepts
- `wiki/concepts/operation-pipeline.md` — where the chain forms in the runtime.
- `wiki/concepts/privilege.md` — the dominant source of false edges.
- `wiki/concepts/partition.md` — how to express data-parallelism.
- `wiki/concepts/tracing.md` — how to collapse repeated analysis cost.
- `wiki/concepts/legion-prof.md` — critical-path view.
- `wiki/concepts/legion-spy.md` — dataflow-graph confirmation.

---
title: Automated Mapping
slug: automated-mapping
summary: A research-level technique (paper automap2023.pdf) that infers task placement and instance memory automatically from a cost model + machine topology; an alternative to hand-written custom mappers for typical workloads.
tags: [mapping, for-perf-debug]
subsystem: legion
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/publications/publications.md
github:
  - https://github.com/StanfordLegion/legion/tree/master/runtime/mappers
related:
  - wiki/concepts/mapper.md
  - wiki/concepts/default-mapper.md
  - wiki/concepts/sharding-functor.md
  - wiki/concepts/control-replication.md
  - wiki/concepts/realm-machine-model.md
---

## TL;DR
Automated mapping is a technique introduced in the paper *Automated Mapping of Task-Based Programs onto Distributed and Heterogeneous Machines* (`automap2023.pdf`, SC 2023). It takes the manual mapper-writing problem (`mapper.md`, `wiki/workflows/write-a-custom-mapper.md`) and replaces it with a cost-model-driven search over the placement space: given the task DAG, the `realm-machine-model.md`, and a model of per-task costs, the system infers a mapping. The confusion: automated mapping is **not a runtime feature out-of-the-box**; it's a separate tool/system layered on top of Legion. `default-mapper.md` is still the default; automated mapping is the alternative for users who want better-than-default placement without hand-writing a mapper.

## Mental model
Automated mapping is the compiler-style approach to a problem that's traditionally hand-tuned. Where `default-mapper.md` uses general heuristics and `wiki/workflows/write-a-custom-mapper.md` is application-specific code, automated mapping is "the system figures it out from data". Comparable to how a compiler's register allocator replaces hand-rolled assembly: the user expresses intent (the task graph) and a cost-aware solver produces the schedule.

## Mechanism & API
The paper's approach (per `raw/publications/publications.md` entry):
- Inputs: the task DAG (logical operations + region requirements), the target machine's processors/memories/affinities (`realm-machine-model.md`), per-task cost estimates (compute, memory).
- Output: an assignment of tasks to processors and instances to memories.
- Implemented as a **mapper subclass** that overrides `map_task.md`, `slice-task.md`, and `select_sharding_functor` to use the inferred placement instead of heuristics.

In practice, automated mapping is typically distributed as an additional Legion-tools package or as a custom mapper a user clones from the paper's repo; it's not enabled by default. Profile any application that uses it: the wins are workload-dependent, and the inference cost shows up as `mapper-stalls`-like activity at startup or between major program phases.

## Invariants
- Automated mapping produces a **valid Legion mapping** — same correctness guarantees as any custom mapper.
- It does not change application semantics; like all mappers, it controls performance, not correctness (`mapper.md`).
- Inference cost may be non-trivial — runtime spent solving the placement problem is **not** application work.
- Cost models vary in fidelity; for unusual workloads the inferred mapping may be worse than `default-mapper.md`.
- The technique combines with `control-replication.md` and `tracing.md` in the standard way; downstream perf knobs still apply.

## Performance implications
- **Best for users without mapping expertise** who want better-than-default placement without writing a custom mapper.
- **Worst when the cost model doesn't match the application** — bizarre placement decisions, mapper-stalls during inference.
- Profile with `legion-prof.md` to confirm the inferred placement matches your intuition; `mapper-logging.md` shows the decisions.
- For repeated workloads, the inference results can be cached across runs; some implementations expose this.

## Debug signals
- **`legion-prof.md`** shows a clear "mapper inference" phase at startup or before major regions — busy utility rows, idle app rows for a measurable duration.
- **`mapper-logging.md`** identifies which inferred decision was made for each task.
- **`pitfalls/mapper-stalls.md`** symptoms during inference are normal; if they persist into steady-state, the cost model is too expensive.

## Failure modes
- The cost model produces poor placement for an unusual workload → worse perf than `default-mapper.md`. Fall back to default + targeted overrides.
- Inference cost itself dominates → cache results across runs or only re-run inference when the task graph changes.

## Source pointers
- **Paper**: `raw/publications/pdfs/automap2023.pdf` — *Automated Mapping of Task-Based Programs onto Distributed and Heterogeneous Machines* (SC 2023).
- **Mappers tree**: https://github.com/StanfordLegion/legion/tree/master/runtime/mappers — the standard mapper code; reference implementations and integrations evolve here.

## Related
- `wiki/concepts/mapper.md` — what automated mapping subclasses or replaces.
- `wiki/concepts/default-mapper.md` — the alternative for users not using automated mapping.
- `wiki/concepts/sharding-functor.md` — automated mapping typically infers these too under control replication.
- `wiki/concepts/control-replication.md` — combines with automated mapping.
- `wiki/concepts/realm-machine-model.md` — the input topology.

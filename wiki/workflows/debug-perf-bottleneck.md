---
title: Debug a Performance Bottleneck
slug: debug-perf-bottleneck
summary: A decision tree that maps Legion Prof observations to pitfall pages, so you walk from "the program is slow" to a specific named cause and fix.
tags: [for-perf-debug, profiling, debugging]
status: draft
created: 2026-05-15
updated: 2026-05-15
related:
  - wiki/workflows/profile-an-app.md
  - wiki/concepts/legion-prof.md
  - wiki/concepts/legion-spy.md
  - wiki/concepts/operation-pipeline.md
  - wiki/concepts/critical-path.md
  - wiki/concepts/mapper-logging.md
---

## Inputs
- A captured Legion Prof profile (see `profile-an-app.md`).
- (Optional) the application source for the hot section.

## Steps

1. **Press `a` in Legion Prof to draw the critical path.** Identify which row contributes the most time.

2. **Critical path is mostly on application processor rows** (CPU or GPU executing tasks):
   - **Single long chain of tasks** → [long-dependence-chains](../pitfalls/long-dependence-chains.md). Investigate why: no index launch, false dependence, aliased partition, or missing trace.
   - **GPU rows mostly idle while CPU rows busy** → [gpu-underutilization](../pitfalls/gpu-underutilization.md). Investigate mapper placement and instance memory.

3. **Critical path is mostly on utility processor rows** (the runtime is busier than the app):
   - Same activity every iteration → [missed-tracing-opportunity](../pitfalls/missed-tracing-opportunity.md). Wrap loop body in `begin_trace`/`end_trace`.
   - Long single mapper-callback bars → [mapper-stalls](../pitfalls/mapper-stalls.md). Make callbacks faster / cache `Machine` queries.
   - Many tiny operations per iteration → [runtime-overhead-dominates](../pitfalls/runtime-overhead-dominates.md). Coarsen tasks; use `IndexLauncher`.

4. **Critical path is mostly on channel rows** (DMA between memories):
   - Persistent host↔device copies → [excessive-data-movement](../pitfalls/excessive-data-movement.md). Co-locate instance with consumer; consider `Z_COPY_MEM`.
   - Specifically GPU starving on data → [gpu-underutilization](../pitfalls/gpu-underutilization.md).

5. **Memory rows show churn** (many short instance slabs):
   - [instance-fragmentation](../pitfalls/instance-fragmentation.md). Stabilize layout constraints; use `find_or_create_physical_instance`.

6. **Same task bouncing between processor rows across iterations**:
   - [mapper-bouncing](../pitfalls/mapper-bouncing.md). Pin with `ProcessorConstraint`; add hysteresis to mapper.

7. **Two "independent" tasks serialize** (according to Spy dataflow):
   - [false-dependencies-overbroad-privileges](../pitfalls/false-dependencies-overbroad-privileges.md). Narrow privileges; use disjoint partitions; prefer `WRITE_DISCARD` / `REDUCE`.

8. **Sibling point tasks of an index launch serialize**:
   - [non-disjoint-disjoint-partition](../pitfalls/non-disjoint-disjoint-partition.md). Run with `-lg:partcheck` to confirm.

9. **Apply the fix, re-run `profile-an-app.md`, and verify** the symptom is gone *and* a new one hasn't taken its place.

## Outputs
- A named pitfall that matches your profile.
- A concrete fix to attempt.
- A re-measurement that confirms (or refutes) the hypothesis.

## When to use
- After capturing a profile with `wiki/workflows/profile-an-app.md` and finding the application slower than expected.
- Whenever you observe a symptom but don't know which Legion concept explains it.

## Related
- `wiki/concepts/legion-prof.md` — every row and signal mentioned here.
- `wiki/concepts/legion-spy.md` — for confirming false dependencies and partition disjointness.
- `wiki/concepts/operation-pipeline.md` — the mental model for which stage each symptom lives in.
- `wiki/workflows/profile-an-app.md` — how to get the profile.

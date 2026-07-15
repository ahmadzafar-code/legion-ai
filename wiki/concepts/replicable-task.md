---
title: Replicable Task
slug: replicable-task
summary: A task variant declared with `set_replicable(true)`; promises determinism given inputs across all shards, making the task eligible for control replication (SPMD execution of one logical task as N copies).
tags: [execution, replication, distributed, for-perf-debug]
subsystem: legion
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/youtube_transcripts/runtime_school_2023/transcripts/002_Legion_Runtime_Internals_-_Lesson_2_-_Tasks_Context_and_Forward_Progress.txt
  - raw/youtube_transcripts/runtime_school_2023/transcripts/016_Legion_Runtime_Internals_-_Lesson_17_-_Control_Replication_Part_1.txt
  - raw/publications/publications.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
related:
  - wiki/concepts/task.md
  - wiki/concepts/task-variant.md
  - wiki/concepts/leaf-task.md
  - wiki/concepts/inner-task.md
  - wiki/concepts/control-replication.md
  - wiki/concepts/sharding-functor.md
---

## TL;DR
A replicable task variant is one the application has declared safe to execute as N replicated copies under control replication. Set via `TaskVariantRegistrar::set_replicable(true)`. The application promises that the task body is **deterministic given the same logical inputs across all shards** — same control flow, same launch sequence, same effects (modulo the sharding functor's per-point ownership). The confusion: most top-level tasks in modern Legion are replicable; the flag is what *opts in*. Without it, the top-level task runs on one processor and dependence analysis becomes the bottleneck on multi-node runs (`pitfalls/runtime-overhead-dominates.md`).

## Mental model
Replicable is the SPMD opt-in for a task. The flag tells the runtime "feel free to run me N times in lockstep on N shards, each handling its share of the work, agreeing on operation order, exchanging cross-shard data via collectives." Where `leaf-task.md` and `inner-task.md` are about *what kind of body this task has*, replicable is about *how the runtime is allowed to compose copies of this task across processors*.

## Mechanism & API
Set on the registrar:
```cpp
TaskVariantRegistrar reg(TOP_LEVEL_TASK_ID, "top_level");
reg.add_constraint(ProcessorConstraint(Processor::LOC_PROC));
reg.set_replicable(true);
Runtime::preregister_task_variant<top_level_task>(reg, "top_level");
```

When a replicable task is launched and control replication is enabled (`-lg:control_replication` or via mapper hints), the runtime executes the task as **N shards** — one per chosen processor. Each shard executes the same C++ task body but the sharding functor (`sharding-functor.md`) decides which shard owns each point of any index launches that shard's body issues. See `control-replication.md` for the broader picture.

The replicable variant runs under a **replicated context** (`ReplicateContext`), a derived class of `InnerContext` (from Runtime School Lesson 2) that adds per-shard state and inter-shard collective machinery.

Compatibility with other flags:
- `set_replicable(true) + set_leaf(true)` — legal. A leaf replicable task is fully data-touching but executes one copy per shard; used for replicated leaves of a SPMD computation.
- `set_replicable(true) + set_inner(true)` — legal and common for orchestrator-style top-level tasks under replication.

## Invariants
- A replicable task's body must be **deterministic given identical logical inputs across all shards**: the sequence of `execute_task` / `execute_index_space` / future reads / etc. must match on every shard.
- Non-determinism that affects the operation stream (random number sampling, system clock reads, file IO, external mutable state) is **forbidden** — must be routed through Legion (futures, regions, tunables) so all shards see the same inputs.
- The number of shards is decided at launch and stable for the task's lifetime.
- The sharding functor decides which points of each index launch this shard "owns"; non-owned points still participate in dependence analysis but do not execute.
- Output regions reduce across shards via Legion's normal reduction-instance machinery when appropriate.
- A replicable variant that **silently uses non-deterministic input** → undefined behavior (hangs, wrong results, shard disagreement).

## Performance implications
- **Required for multi-node scaling.** Without `set_replicable`, the top-level task is single-processor and per-iteration dep-analysis cost becomes the bottleneck.
- Combined with **tracing** (`tracing.md`), per-shard analysis collapses after the first iteration → near-linear scaling on stencil-style workloads.
- Replicable leaf tasks are useful when each shard needs its own copy of "scaffolding" data to compute on — paired with a sharding functor that gives each shard its share of points.
- The sharding-functor choice matters: a balanced functor keeps shard load even (`pitfalls/mapper-bouncing.md` is the per-iteration unstable-placement variant of this).

## Debug signals
- **printfs in a replicable top-level task appear N times** in a multi-node run — that's the visible "you have N shards" signal.
- **`-level replication=2`** logs each shard's operation stream and collective transfers.
- **Legion Spy under replication** generates N copies of the logical-analysis graph (one per shard) on the same shared Realm event graph. If they don't match in structure, your task is not deterministic.
- **Shard load skew** in Legion Prof's per-shard utility rows → bad sharding functor or non-deterministic load distribution.

## Failure modes
- Top-level task without `set_replicable(true)` running on multi-node → all dep-analysis on one node → [runtime-overhead-dominates](../pitfalls/runtime-overhead-dominates.md) at scale.
- Replicable task that uses `rand()` per shard → shards disagree → hangs or wrong results.

## Source pointers
- **Legion API**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Paper (DCR)**: `raw/publications/pdfs/dcr2021.pdf`
- **Paper (static CR, predecessor)**: `raw/publications/pdfs/cr2017.pdf`
- **Lectures**: `raw/youtube_transcripts/runtime_school_2023/transcripts/002_..._Tasks_Context.txt`, `016_..._Control_Replication_Part_1.txt` through `Part_5`

## Related
- `wiki/concepts/task.md` — host.
- `wiki/concepts/task-variant.md` — where `set_replicable` is set.
- `wiki/concepts/leaf-task.md`, `wiki/concepts/inner-task.md` — the other two task-property flags.
- `wiki/concepts/control-replication.md` — what replicable opts into.
- `wiki/concepts/sharding-functor.md` — what decides per-shard ownership.

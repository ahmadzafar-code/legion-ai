---
title: Task Fusion
slug: task-fusion
summary: A runtime optimization that merges two or more sequential tasks operating on the same data into a single fused task; eliminates the runtime overhead between them and the intermediate-copy cost (papers pawatm2022 + fusion2025).
tags: [execution, replication, for-perf-debug]
subsystem: legion
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/publications/publications.md
github:
  - https://github.com/StanfordLegion/legion/tree/master/runtime/legion
related:
  - wiki/concepts/operation-pipeline.md
  - wiki/concepts/tracing.md
  - wiki/concepts/control-replication.md
  - wiki/concepts/kernel-fusion.md
  - wiki/concepts/mapper.md
---

## TL;DR
Task fusion is a runtime optimization that takes two or more sequential tasks operating on the same data and **merges them into one fused task**. The runtime saves the per-task overhead (one trip through the operation pipeline instead of N) and eliminates the intermediate-copy materialization between them. Subject of the papers *Task Fusion in Distributed Runtimes* (`pawatm2022.pdf`, PAW-ATM 2022) and *Composing Distributed Computations Through Task and Kernel Fusion* (`fusion2025.pdf`, ASPLOS 2025). The confusion: task fusion is not the same as `kernel-fusion.md`. Task fusion is **at the runtime level** (merging Legion operations); kernel fusion is **at the GPU code level** (merging the actual compiled kernels).

## Mental model
Task fusion is like compiler **basic-block merging**: two adjacent tasks that read each other's outputs become one task whose body does both pieces of work. Where compiler IR has SSA form letting the optimizer fuse passes, Legion's task graph lets the runtime fuse compatible launches. The benefit compounds with `kernel-fusion.md` — once the *tasks* are fused, the actual GPU/CPU kernel code inside can be merged too.

## Mechanism & API
Task fusion is **runtime-driven, opted in by the mapper or compiler**. The runtime inspects adjacent tasks for fusion candidates:
- Same processor placement.
- Compatible region requirements (one task's output is the next's input).
- Both fusable (the task variant doesn't have side effects the runtime can't replicate).

When the runtime fuses, it produces a single Realm task that runs both bodies sequentially with the intermediate data held in registers / cache rather than materialized as a full instance.

**Where fusion is decided**:
- Static fusion: Regent's compiler (`regent-language.md`) can emit fused tasks where it sees the opportunity.
- Dynamic fusion: the Legion runtime can fuse at mapping time given mapper hints — the user typically signals via task tags or via a custom mapper.
- Per the ASPLOS 2025 paper, the technique covers both task and kernel level in a unified framework.

## Invariants
- Fused tasks **preserve correctness** — the runtime guarantees the result is observationally equivalent to running the unfused tasks sequentially.
- Fusion is **transparent to the application**: the launcher API is unchanged.
- The fused task inherits the union of the constituent tasks' region requirements and privileges.
- Fused tasks **may not run** if the mapper's placement doesn't permit (e.g., same processor required).
- Fusion combines naturally with `tracing.md`: a fused trace template replays even more cheaply than an unfused one.

## Performance implications
- **Saves per-task runtime overhead** at pipeline stages 1-5. For workloads with many small fusible tasks, this can dominate the win.
- **Saves intermediate-copy materialization** for chained computations — the intermediate `physical-instance.md` doesn't need to exist as a full buffer.
- For multi-node distributed runs (paper `fusion2025.pdf`), task fusion + kernel fusion together compose to make Legion competitive with hand-tuned MPI+OpenMP/CUDA codes on stencil and ML workloads.
- Profile with `legion-prof.md` before/after — fused regions show up as fewer, larger task bars on processor rows.

## Debug signals
- **`legion-prof.md`** processor rows: fused operations show up as a single larger bar in place of multiple smaller ones.
- **`-level legion=2`** logs fusion decisions (depending on Legion version).
- **A workload that's slower under fusion** = the fusion isn't paying off; check that the fused tasks actually share a processor.

## Failure modes
- Fusing across incompatible tasks (different processors, conflicting privileges) → runtime declines to fuse; no harm but no win either.
- Application relying on intermediate side effects (debugging printfs, profile counters) at the boundary → those side effects may not happen at the fused boundary.

## Source pointers
- **Paper (task fusion)**: `raw/publications/pdfs/pawatm2022.pdf` — *Task Fusion in Distributed Runtimes* (PAW-ATM 2022).
- **Paper (task + kernel fusion)**: `raw/publications/pdfs/fusion2025.pdf` — *Composing Distributed Computations Through Task and Kernel Fusion* (ASPLOS 2025).
- **Runtime tree**: https://github.com/StanfordLegion/legion/tree/master/runtime/legion

## Related
- `wiki/concepts/operation-pipeline.md` — what fusion saves overhead in.
- `wiki/concepts/tracing.md` — combines with task fusion for max savings.
- `wiki/concepts/control-replication.md` — compatible with replicated execution.
- `wiki/concepts/kernel-fusion.md` — the layer-below sibling.
- `wiki/concepts/mapper.md` — typically the place fusion is hinted.

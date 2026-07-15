---
title: Control Replication
slug: control-replication
summary: An execution mode that runs N copies (shards) of a task in parallel while making them behave as a single logical task; the mechanism that makes Legion's implicit parallelism scale across nodes. Dynamic control replication (DCR, paper `dcr2021.pdf`) uses a two-stage coarse + fine dependence analysis with O(log N) cross-shard fences.
tags: [replication, distributed, parallelism, for-perf-debug, for-program-reasoning]
subsystem: legion
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/publications/pdfs/dcr2021.pdf
  - raw/youtube_transcripts/runtime_school_2023/transcripts/016_Legion_Runtime_Internals_-_Lesson_17_-_Control_Replication_Part_1.txt
  - raw/publications/publications.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
  - https://github.com/StanfordLegion/legion/tree/master/runtime/legion
related:
  - wiki/concepts/task.md
  - wiki/concepts/mapper.md
  - wiki/concepts/operation-pipeline.md
  - wiki/concepts/tracing.md
  - wiki/concepts/sharding-functor.md
  - wiki/concepts/regent-language.md
  - wiki/concepts/pygion.md
  - wiki/concepts/replicable-task.md
  - wiki/concepts/control-flow-as-data.md
  - wiki/concepts/collective-view.md
  - wiki/applications/pennant.md
  - wiki/applications/miniaero.md
---

## TL;DR
Control replication runs N copies (**shards**) of a task — typically the top-level task — across N processors / nodes. Each shard executes the same control flow but takes responsibility for a different slice of the work. The shards collectively behave as a single logical task: subtask launches happen exactly once in the logical operation graph, but **dependence analysis itself is partitioned across the shards**, eliminating the single-node bottleneck that limits non-replicated runs. Two variants: **static control replication** (SCR, paper `cr2017.pdf`, compiled by Regent) and **dynamic control replication** (DCR, paper `dcr2021.pdf`, runtime-driven). DCR is the modern default; this page covers it in depth.

## Mental model
> "We've actually run four copies of that top-level task in this program, but we've still made it look as if those four copies are all behaving as one logical task execution." — Runtime School 2023, Lesson 17.

Control replication is the SPMD-ification of a sequential Legion program. The application writes a single logical task; the runtime executes it as N shards in lockstep. **Control flow is data** (`control-flow-as-data.md`): branches, loop bounds, future values that influence which operations get issued are exchanged between shards via collectives so all shards agree on which operations exist, while disagreeing on which shard owns each one.

## Mechanism & API

**Application side**:
- Register the top-level task with `TaskVariantRegistrar::set_replicable(true)` (see `replicable-task.md`) — or use `__demand(__replicable)` in Regent.
- Enable at runtime: typically default in modern Legion when a replicable variant exists.
- The **sharding functor** (`sharding-functor.md`) chooses which shard owns each point of an `IndexLauncher`. The mapper picks the functor via `select_sharding_functor`.

**Two-stage dependence analysis** (`dcr2021.pdf` §4.1):

DCR's scalability comes from splitting analysis into two stages running concurrently as a pipeline:

1. **Coarse stage** — operates on **task groups** (sets of consecutive task-launch operations that are pairwise independent under a single launcher). For each group `G₁`, identify cross-group dependences against `G₂` using **representative tasks** (each task in the group is summarized by a single representative whose region argument is the upper-bound partition). The result: O(#groups²) dependences, not O(#tasks²). Cross-shard coordination at this layer uses **cross-shard fences** of cost O(log N) via collectives. The coarse stage runs **on every shard** for every task group — every shard knows about every cross-group dependence.

2. **Fine stage** — once a coarse-stage dependence is satisfied, each shard runs precise per-task analysis on the subset of tasks it owns (per the sharding function). Most dependences turn out to be **shard-local** (the cross-shard fence can be elided). Cross-shard fences are inserted only when same-region tasks landed on different shards.

The asymptotic result: dependence-analysis cost is O(log N) per cross-shard fence, total time roughly constant per shard regardless of node count. The paper demonstrates this on 512+ nodes.

**Control determinism** (`dcr2021.pdf` §3): replicated programs must be **control deterministic** — all shards must make the same sequence of Legion API calls with the same actual arguments. DCR enforces this dynamically: for each runtime API call on a replicated task, the runtime computes a **128-bit hash** of the call + arguments, then an **all-reduce collective** checks all shards' hashes match. Mismatch → fatal error with a diagnostic. The check runs asynchronously (latency-hidden) and is on by default but tunable.

Sources of non-determinism that violate control replication and how DCR handles them (`dcr2021.pdf` §3, §5.4):
- `rand()` per shard → wrong. Use Legion futures or tunables instead.
- File I/O → handled via **delayed `attach`/`detach`** as group operations, executed once via the runtime.
- Garbage collection (Python/Lua finalizers) → handled by delaying destruction until all shards observe the same operation count, then dispatching as a runtime op.

**Sharding functor** (`sharding-functor.md`) is a pure, deterministic, surjective function `task → shard_id`. Each shard runs analysis only for tasks the functor maps to its shard ID. Sharding functors are memoizable; the runtime caches their results.

**Side effects** (`dcr2021.pdf` §4.3):
- Persistent file I/O: handled via Legion's `attach`/`detach`; group-operated so the actual disk operation happens once.
- External buffers (MPI interop): handled via the attach/detach pattern.
- GC: delayed-detach pattern for languages with finalizers.

## Invariants
- All shards see the **same logical operation stream** in program order. They disagree only on which shard owns each operation.
- The sharding functor must be **pure** (deterministic given inputs) and **total** (every task maps to some shard).
- The runtime **dynamically checks control determinism** via 128-bit hash + all-reduce; mismatch is a fatal error.
- Sub-task launches in a replicated task happen **exactly once** in the logical graph regardless of N. The functor decides ownership.
- The number of shards is fixed at launch; DCR supports varying it across phases via the mapper interface.
- Two-stage analysis means **most dependences are shard-local**; cross-shard fences only fire when tasks landed on different shards.

## Performance implications

Per the `dcr2021.pdf` evaluation:

- **Required for multi-node scaling.** Non-replicated top-level tasks scale to ~8 nodes before single-node analysis dominates.
- **2D stencil**: weak scaling to 512 nodes with DCR; only 2.5% slowdown vs. static control replication.
- **PENNANT (hydrodynamics on unstructured mesh)**: weak scaling on Sierra to 256 nodes outperforms MPI+CUDA by 2.3× thanks to better intra-node load balancing.
- **FlexFlow (deep learning)**: scales to 4,000+ GPUs on Frontier-class hardware; matches manual MPI.
- **Legate (NumPy/SciPy)**: 11.4× faster than Dask on logistic regression, 2.7× on PCG solver — wins come from removing Dask's centralized control node.
- **Soleil-X (multi-physics)**: 82% parallel efficiency at 1024 GPUs.
- **HTR (hypersonic flows)**: 86-94% parallel efficiency to thousands of cores.

Index launches benefit most: each shard analyzes and maps only its share. Combined with **tracing** (`tracing.md`), per-shard analysis collapses to near-zero on traced replays. The combination is what enables Legion's near-linear weak scaling on stencil and ML workloads.

## Debug signals
- **printfs from the top-level task appear N times** in a multi-node run — that's the visible "you have N shards" signal.
- **Control-determinism violation** at runtime: fatal error with the failing API call name, the shard IDs, and the hash mismatch. Diagnose the source of per-shard divergence.
- **`-level replication=2`** logs per-shard operations, cross-shard fences, and collective transfers.
- **Legion Prof per-shard rows**: utility activity should be roughly equal across shards. Skew = bad sharding functor.
- **`-level dcr=2`** (where available) logs DCR-specific scheduling decisions.

## Failure modes
- Application reads `rand()` / `clock()` / file IO per shard without routing through Legion → control-determinism check fails (fatal error in modern Legion). Fix by using futures, tunables, or `attach`/`detach`.
- Sharding functor concentrates work → poor load balance; visible in Legion Prof.
- Mapper changing sharding functor across runs → unstable behavior; pin the functor or use the default.

## Source pointers
- **Paper (Dynamic CR)**: `raw/publications/pdfs/dcr2021.pdf` — PPoPP 2021, *Scaling Implicit Parallelism via Dynamic Control Replication* (Bauer, Lee, Slaughter, Jia, Di Renzo, Papadakis, Shipman, McCormick, Garland, Aiken).
- **Paper (Static CR — Regent compiler)**: `raw/publications/pdfs/cr2017.pdf` (SC 2017).
- **Legion API**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Lectures**: `raw/youtube_transcripts/runtime_school_2023/` Lessons 17–21.

## Related
- `wiki/concepts/task.md` — the unit that gets replicated.
- `wiki/concepts/mapper.md` — picks the sharding functor + replication policy.
- `wiki/concepts/operation-pipeline.md` — replication partitions pipeline stages across shards.
- `wiki/concepts/tracing.md` — multiplies the perf win by collapsing per-shard analysis on replay.
- `wiki/concepts/sharding-functor.md` — the per-shard ownership decision.
- `wiki/concepts/replicable-task.md` — the opt-in.
- `wiki/concepts/control-flow-as-data.md` — the principle.
- `wiki/concepts/collective-view.md` — the deduplication structure for shared regions across shards.

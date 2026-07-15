---
title: Control Flow as Data
slug: control-flow-as-data
summary: The principle behind control replication; treating the program's control flow (branches, loop bounds, future values) as data that the runtime exchanges between shards via collectives so all shards agree on which operations exist.
tags: [replication, distributed, execution, for-program-reasoning]
subsystem: legion
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/youtube_transcripts/runtime_school_2023/transcripts/016_Legion_Runtime_Internals_-_Lesson_17_-_Control_Replication_Part_1.txt
  - raw/publications/publications.md
github:
  - https://github.com/StanfordLegion/legion/tree/master/runtime/legion
related:
  - wiki/concepts/control-replication.md
  - wiki/concepts/replicable-task.md
  - wiki/concepts/sharding-functor.md
  - wiki/concepts/future.md
---

## TL;DR
"Control flow is data" is the framing behind Legion's `control-replication.md`: when a replicable task's body executes on N shards, the branches it takes, the loop bounds it uses, and the future values it reads are not separate "control" state — they're **data the runtime exchanges between shards via collectives** so all shards agree on which operations to issue. The confusion: the application writes a single sequential-looking program, but under replication the program's "control state" is implicitly distributed and re-synchronized. The runtime handles this for the application as long as the task body is deterministic given the same logical inputs.

## Mental model
"Control flow is data" reframes SPMD execution: instead of writing one program per process and synchronizing explicitly, write one logical program and let the runtime treat *every branch decision* as a value that must be consistent across shards. If shards disagree on a branch, they disagree on what operations exist — the replication contract is broken. The runtime enforces agreement via collectives at the right points: future reads, dynamic loop bounds, tunable-variable queries.

## Mechanism & API
Per Runtime School Lesson 17 + paper `dcr2021.pdf` (Dynamic Control Replication, PPoPP 2021):

When a `replicable-task.md` runs across N shards:
1. The runtime broadcasts (or collects) values that affect control flow — `future.get_result()` calls, `runtime->get_tunable_value()` returns, dynamic launch-bound computations.
2. Each shard executes the same control-flow path because all shards observe the same value.
3. Each shard then runs its share of the resulting operations as decided by the `sharding-functor.md`.

**What this looks like in source code** (the application doesn't see it):
```cpp
// Application code — looks sequential, runs on every shard:
int num_iters = runtime->get_tunable_value(ctx, NUM_ITERS_TUNABLE,
                                           MY_MAPPER_ID).get_result<int>();
for (int i = 0; i < num_iters; i++) {
  runtime->execute_index_space(ctx, stencil_launcher);
}
```

Under the hood:
- `get_tunable_value` is a future the runtime makes collective: all shards block until the value is broadcast.
- Each iteration's index launch is a logical operation visible to all shards; the sharding functor picks per-point owners.

If `num_iters` were a per-shard random value, the control flow would diverge — undefined behavior. This is why `replicable-task.md` requires determinism.

## Invariants
- Control-flow decisions in a replicable task **must be deterministic given the same logical inputs across all shards**.
- The runtime treats `future.get_result()`, `tunable-variable.md` queries, and other "data-dependent control flow" inputs as collective operations.
- A divergence in control flow between shards is **undefined behavior** — typically manifests as hangs or wrong results.
- Non-deterministic local state (`rand()`, `time()`, file IO) **must be routed through Legion** (futures, regions) so all shards see the same value.
- The runtime does not check that control flow agrees — it trusts the determinism contract.

## Performance implications
- The collective communication for shared control values is **cheap** for small values (one collective per future) and dominates only when control state is huge.
- Combined with `tracing.md`, the control-collective work is also memoized — second-iteration overhead is minimal.
- The technique enables true near-linear scaling on multi-node runs by partitioning analysis work across shards while keeping the application's logical program single-thread.
- See paper `dcr2021.pdf` for benchmark results on stencil and training workloads.

## Debug signals
- **Shards diverging in printf output** (different shards print different things at the same logical point) = non-determinism leaking through. The application reads a per-process value (rand, time) that should be routed through Legion.
- **Hangs in multi-node runs** during control-flow operations = one shard blocked on a collective the others didn't issue. Trace via `REALM_SHOW_EVENT_WAITERS`.
- **`-level replication=2`** logs the per-shard operations; mismatched streams indicate divergence.

## Failure modes
- Using `rand()` / `clock()` / mutable globals per process in a replicable task → divergent control flow.
- Reading from non-Legion shared state that varies per process → same problem.

## Source pointers
- **Paper (dynamic control replication)**: `raw/publications/pdfs/dcr2021.pdf` (PPoPP 2021).
- **Paper (static CR)**: `raw/publications/pdfs/cr2017.pdf` (SC 2017).
- **Lecture**: `raw/youtube_transcripts/runtime_school_2023/transcripts/016_..._Control_Replication_Part_1.txt`.

## Related
- `wiki/concepts/control-replication.md` — the system this principle underlies.
- `wiki/concepts/replicable-task.md` — what the task opts into.
- `wiki/concepts/sharding-functor.md` — partitions per-point ownership after control flow agrees.
- `wiki/concepts/future.md` — one of the values turned into collective data.

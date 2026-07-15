---
title: Sharding Functor
slug: sharding-functor
summary: A pure function from index-launch point to shard ID; under control replication, this is how shards agree on which one owns each point task.
tags: [replication, distributed, mapping, for-perf-debug]
subsystem: legion
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/youtube_transcripts/runtime_school_2023/transcripts/019_Legion_Runtime_Internals_-_Lesson_20_-_Control_Replication_Part_4.txt
  - raw/publications/publications.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
related:
  - wiki/concepts/control-replication.md
  - wiki/concepts/mapper.md
  - wiki/concepts/index-space-launch.md
  - wiki/concepts/default-mapper.md
  - wiki/concepts/slice-task.md
---

## TL;DR
A sharding functor is a deterministic, pure function `(point, launch_space, sharding_space) → shard_id` that, under control replication, decides which replicated copy of the parent task "owns" each point of an index launch. All shards run the same functor, so each can compute *independently* which points it must execute and which points belong to peers. The mapper picks the functor via the `select_sharding_functor` callback. The confusion: the functor doesn't *do* the work; it tells each shard which subset of the points is theirs, so the work splits N-ways across shards.

## Mental model
Sharding functors are to control replication what hash functions are to consistent hashing: a deterministic agreement mechanism. Every replica computes the same function over the same inputs and they end up with disjoint, complete partitions of the work — no communication needed for the assignment itself.

## Mechanism & API
A sharding functor inherits from `ShardingFunctor` and implements:
```cpp
class MyShardingFunctor : public ShardingFunctor {
public:
  ShardID shard(const DomainPoint &point,
                const Domain     &launch_space,
                const size_t      total_shards) override {
    // pure function: same (point, launch_space, total_shards) → same shard
    return point[0] % total_shards;
  }
};
```

Register at startup:
```cpp
Runtime::preregister_sharding_functor(MY_SHARDING_FUNCTOR_ID, new MyShardingFunctor());
```

The mapper selects the functor for each index launch:
```cpp
void MyMapper::select_sharding_functor(
    const MapperContext ctx, const Task &task,
    const SelectShardingFunctorInput &in, SelectShardingFunctorOutput &out) override {
  out.chosen_functor = MY_SHARDING_FUNCTOR_ID;
}
```

`DefaultMapper` picks a linear `point % N` functor by default — fine for uniform workloads.

Under control replication (`control-replication.md`):
- Logical analysis on each shard walks the index launch but the functor decides whether each point is "owned" (this shard must run it) or "observed" (some other shard owns it).
- Owned points produce real work on this shard; observed points produce stub entries that participate in dependence analysis but issue no execution.
- All shards see the same logical operation graph; the functor partitions it cleanly.

The Runtime School Lesson 20 (Part 4 of Control Replication) describes how the runtime tracks **projection summary trees** with per-shard children information so each shard knows which logical regions other shards' points reach into — enabling the necessary inter-shard fences without forcing all-to-all communication on every analysis.

## Invariants
- **Deterministic and pure.** Given identical `(point, launch_space, total_shards)`, every shard must return the same `shard_id`. Non-determinism causes shards to disagree and is undefined behavior.
- **Surjective enough** to keep work distributed. A functor that always returns shard 0 is legal but pathological — all work concentrates.
- The functor is **called once per (point, launch) pair on each shard** during logical analysis; not on the hot path of execution.
- Sharding functors are application-wide. Each is registered with a `ShardingID` and selected by the mapper per launch.
- Range: shard IDs span `0 .. total_shards-1`; returning out-of-range is an error.

## Performance implications
- **Load balance** is set by the functor. A skewed functor (e.g., `point / chunk` with non-uniform points-per-shard) concentrates work on a few shards; visible in Legion Prof as some application rows much busier than others.
- For data-parallel workloads with disjoint partitions, the **identity-mod-N** functor is usually optimal — `DefaultMapper`'s choice.
- For stencils with halo regions, a **block-cyclic** or **2D tiled** functor often beats linear: it keeps each shard's halo accesses local to a contiguous group of points.
- A bad sharding choice multiplies the visibility-analysis cost (more inter-shard fences); see paper `visibility2023.pdf`.

## Debug signals
- **Legion Prof multi-node**: per-shard utility-row activity should be roughly equal. Skew = bad functor.
- **`-level replication=2`**: logs per-shard ownership decisions.
- **Legion Spy**: under replication, one logical dataflow graph per shard; matching their structure confirms the functor agreed.

## Failure modes
- Non-deterministic functor → shards disagree on ownership → hangs or incorrect results.
- Skewed functor → underutilized shards (visible in Legion Prof).

## Source pointers
- **Legion API (`ShardingFunctor`, `preregister_sharding_functor`)**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Lecture**: `raw/youtube_transcripts/runtime_school_2023/transcripts/019_..._Control_Replication_Part_4.txt` (and Parts 1–5 broadly).
- **Paper (DCR)**: `raw/publications/pdfs/dcr2021.pdf`

## Related
- `wiki/concepts/control-replication.md` — what sharding functors enable.
- `wiki/concepts/mapper.md` — the `select_sharding_functor` callback picks one.
- `wiki/concepts/index-space-launch.md` — what gets sharded.
- `wiki/concepts/default-mapper.md` — picks the linear functor by default.

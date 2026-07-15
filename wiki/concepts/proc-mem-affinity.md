---
title: Processor-Memory Affinity
slug: proc-mem-affinity
summary: Realm's per-pair (Processor, Memory) bandwidth + latency record; what the mapper consults to pick a memory close to a chosen processor.
tags: [data-model, configuration, mapping, for-perf-debug, for-program-reasoning]
subsystem: realm
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/realm_02_machine_model.md
  - raw/website-pages/mapper.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/realm/machine.h
related:
  - wiki/concepts/realm-machine-model.md
  - wiki/concepts/processor-kinds.md
  - wiki/concepts/memory-kinds.md
  - wiki/concepts/mapper.md
  - wiki/concepts/map-task.md
---

## TL;DR
Processor-memory affinity is Realm's per-pair record of "how fast can processor P access memory M?". Each (Processor, Memory) pair gets a `ProcessorMemoryAffinity` entry with relative `bandwidth` and `latency` numbers. Mappers query the affinity table to decide which memory should host an instance for a given task's target processor. The confusion: bandwidth and latency are **ordinal**, not in absolute units — compare values within a single machine snapshot to determine "closest memory", not across machines.

## Mental model
Affinity is Realm's distance table. Each processor "owns" the memories closest to it (highest bandwidth, lowest latency); the mapper uses the table to keep tasks and their data co-located. Where a NUMA-aware program walks `/sys/devices/system/node/`, a Legion mapper queries `Machine::get_proc_mem_affinity`.

## Mechanism & API
```cpp
std::vector<Machine::ProcessorMemoryAffinity> affs;
machine.get_proc_mem_affinity(affs);

for (auto &aff : affs) {
  Processor p = aff.p;
  Memory m = aff.m;
  unsigned bandwidth = aff.bandwidth;  // higher = better
  unsigned latency   = aff.latency;    // lower  = better
}
```

`get_proc_mem_affinity` returns one entry per (Processor, Memory) pair that has a viable access path. Pairs not in the table have **no direct access** — a copy must be issued through an intermediate memory.

**Filter to a specific processor or memory**:
```cpp
std::vector<Machine::ProcessorMemoryAffinity> affs;
machine.get_proc_mem_affinity(affs, /*proc=*/local_proc);
// affs now contains only entries with p == local_proc.
```

**Memory-to-memory affinity** is the sibling — useful for picking copy paths:
```cpp
std::vector<Machine::MemoryMemoryAffinity> mm_affs;
machine.get_mem_mem_affinity(mm_affs);
```

**Inside a mapper's `map-task.md`**, the standard pattern (also used by `default-mapper.md`'s `default_policy_select_target_memory`):
```cpp
Memory best_mem;
unsigned best_bw = 0;
std::vector<Machine::ProcessorMemoryAffinity> affs;
machine.get_proc_mem_affinity(affs, /*proc=*/target_proc);
for (auto &aff : affs) {
  if (aff.bandwidth > best_bw) {
    best_bw = aff.bandwidth;
    best_mem = aff.m;
  }
}
// Use best_mem for the instance.
```

Cache the result in the mapper constructor if you'll call it on hot paths (per `pitfalls/mapper-stalls.md`).

## Invariants
- Each affinity record is **between one specific `Processor` handle and one specific `Memory` handle** — not between kinds. Multiple CPUs of the same kind may have different affinities to the same memory.
- Bandwidth and latency values are **comparable within one machine snapshot** but not across machines.
- A pair *not* in the affinity table means **no direct access**; the runtime will route through intermediate memories via `dma-system.md` if needed.
- Pair entries are populated at Realm startup based on hardware discovery; they don't change at runtime.
- The matrix is **directional**: `(P, M)` and `(M, P)` are separate concepts in principle, though in practice the same pair record covers both directions.

## Performance implications
- **The primary perf input for `map-task.md`**: pick the memory with the highest bandwidth to the chosen processor.
- `default-mapper.md`'s `default_policy_select_target_memory(ctx, proc, req)` does this for you — usually the right call inside custom mappers.
- Caching affinity queries in the mapper constructor (instead of per-callback) avoids `pitfalls/mapper-stalls.md`.
- Cross-node affinities are typically much lower than intra-node — the mapper should keep tasks on a node whenever possible.

## Debug signals
- **`LoggingWrapper`** logs show `target_memory` chosen for each `map_task`. Trace back to whether `proc_mem_affinity` was queried.
- **Legion Prof channel-row activity** indicates the mapper picked a far memory — affinity-based selection should reduce this.
- **`-level machine=2`** logs the discovered affinity table at startup.

## Failure modes
- Hard-coding a memory choice without consulting affinity → poor placement; `pitfalls/excessive-data-movement.md`.
- Calling `get_proc_mem_affinity` on every `map_task` instead of caching → `pitfalls/mapper-stalls.md`.

## Source pointers
- **Realm header**: https://github.com/StanfordLegion/legion/blob/master/runtime/realm/machine.h
- **Tutorial**: `raw/tutorials/realm_02_machine_model.md`
- **Mapper reference**: `raw/website-pages/mapper.md`

## Related
- `wiki/concepts/realm-machine-model.md` — holds the affinity table.
- `wiki/concepts/processor-kinds.md` — what `p` is.
- `wiki/concepts/memory-kinds.md` — what `m` is.
- `wiki/concepts/mapper.md` — consumer.
- `wiki/concepts/map-task.md` — the specific callback that uses this.

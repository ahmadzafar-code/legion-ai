---
title: Garbage Collection
slug: garbage-collection
summary: The runtime's mechanism for reclaiming distributed-collectable objects (region-tree nodes, equivalence sets, physical instances) once no operation can still reach them; debugged via -DLEGION_GC and tools/legion_gc.py.
tags: [debugging, memory, instances, for-perf-debug, for-correctness-debug]
subsystem: legion
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/website-pages/debugging.md
  - raw/youtube_transcripts/runtime_school_2023/transcripts/005_Legion_Runtime_Internals_-_Lesson_5_-_Distributed_Collectable_Objects.txt
github:
  - https://github.com/StanfordLegion/legion/tree/master/runtime/legion
  - https://github.com/StanfordLegion/legion/blob/master/tools/legion_gc.py
related:
  - wiki/concepts/distributed-collectable.md
  - wiki/concepts/reference-counting-invariants.md
  - wiki/concepts/physical-instance.md
  - wiki/concepts/region-tree.md
  - wiki/concepts/equivalence-set.md
  - wiki/pitfalls/instance-fragmentation.md
---

## TL;DR
Legion's garbage collection is a distributed reference-counting system that decides when to reclaim `physical-instance.md`s, `region-tree.md` nodes, `equivalence-set.md` objects, and the rest of the runtime's heap state. Each object is a `distributed-collectable.md` — a class with reference counts kept consistent across processes. Debugging the GC is essential when you see memory growth, instance churn, or "object never collected" symptoms. The confusion: GC is not a stop-the-world pause; it's per-object, asynchronous, reference-count-driven. The runtime collects each object as soon as its counts hit zero and no future user can reach it.

## Mental model
Legion's GC is `shared_ptr`-style distributed reference counting plus a "valid reference" extension that handles the case where an object is *logically* dead but still referenced by an in-flight operation. Every kind of runtime-managed state inherits the same machinery; debugging GC issues is uniform across object kinds.

## Mechanism & API
**Enabling GC logging at build time** (per `raw/website-pages/debugging.md`):
```bash
CC_FLAGS=-DLEGION_GC make
./app -level legion_gc=2 -logfile gc_%.log
legion/tools/legion_gc.py -l gc_*.log
```

`tools/legion_gc.py` analyzes the logs and reports:
- Objects that were never collected (potential leaks).
- Reference counting errors (counts going negative, double-frees).
- Premature collection of still-referenced objects.

**What the runtime does internally** (per Runtime School Lesson 5):
- Every collectable object inherits from `DistributedCollectable` (`distributed-collectable.md`).
- Each object tracks **resource references** (someone holds a handle) and **valid references** (an in-flight operation can still reach the object's data).
- The "valid reference" state is the extension over plain reference counting; it prevents premature collection of objects an active task might still need.
- On multi-node runs, the home node coordinates reference counts across all nodes that touch the object.
- Collection happens **asynchronously**; the runtime triggers it when all references drop to zero.

**Common collectable objects**:
- `physical-instance.md` — large; collection reclaims actual memory.
- `region-tree.md` nodes (logical regions, partitions, index spaces, field spaces) — small but numerous.
- `equivalence-set.md` — created lazily during physical analysis.
- Mapper state, future buffers, miscellaneous runtime metadata.

**Companion flag**: `-DTRACE_ALLOCATION` with `-level allocation=2` logs every instance allocation — useful for finding hot allocators.

## Invariants
- A `DistributedCollectable` is collected **iff** all reference counts drop to zero across all nodes.
- Reference counts are **monotone** from increment to decrement; underflow indicates a runtime bug or a manual reference-management error.
- The runtime collects objects asynchronously; there's no guarantee of *when* an unreferenced object goes away, only that it eventually does.
- An object reachable via `runtime->acquire_instance` is kept alive until released — useful for the mapper to keep hot instances warm.
- Cross-node collection requires the home node to coordinate; network partitions can delay collection but should not break it.

## Performance implications
- The GC machinery itself is **cheap** on the happy path — reference-count increments and decrements.
- The cost shows up at **collection time**: large instances being reclaimed produce DMA-free events in `legion-prof.md` memory rows.
- `pitfalls/instance-fragmentation.md` is a frequent GC issue — many objects created, immediately collected.
- Leaks (objects never collected) show up as memory growth in long-running programs.

## Debug signals
- **`tools/legion_gc.py -l gc_*.log`** output naming objects that were never collected = leaks. Trace the references back to find the holder.
- **`tools/legion_gc.py`** reporting "premature collection" = a runtime or app bug; report with the gc log attached.
- **Memory rows in Legion Prof** showing many short slabs (lifetime ≪ application time) = instance churn / fragmentation.
- **Memory growth over a long-running iterative app** = leak; rebuild with `-DLEGION_GC` and analyze.

## Failure modes
- Holding a Future, FieldAccessor, or PhysicalRegion past the natural lifetime of its data → keeps the object alive longer than expected.
- `acquire_instance` without a matching `release_instance` → instance pinned forever.

## Source pointers
- **Tool**: https://github.com/StanfordLegion/legion/blob/master/tools/legion_gc.py
- **Reference**: `raw/website-pages/debugging.md`
- **Lecture**: `raw/youtube_transcripts/runtime_school_2023/transcripts/005_..._Distributed_Collectable_Objects.txt`

## Related
- `wiki/concepts/distributed-collectable.md` — the base class.
- `wiki/concepts/reference-counting-invariants.md` — the rules that keep counts honest.
- `wiki/concepts/physical-instance.md` — the largest collectables.
- `wiki/concepts/region-tree.md` — also collected.
- `wiki/concepts/equivalence-set.md` — also collected.
- `wiki/pitfalls/instance-fragmentation.md` — a GC-adjacent symptom.

---
title: Memory Manager
slug: memory-manager
summary: The runtime's per-memory allocator + GC subsystem; tracks live physical instances in a Memory and triggers garbage collection when allocation pressure builds.
tags: [memory, instances, for-perf-debug]
subsystem: legion
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/website-pages/profiling.md
  - raw/website-pages/debugging.md
github:
  - https://github.com/StanfordLegion/legion/tree/master/runtime/legion
related:
  - wiki/concepts/physical-instance.md
  - wiki/concepts/memory-kinds.md
  - wiki/concepts/realm-machine-model.md
  - wiki/concepts/garbage-collection.md
  - wiki/pitfalls/instance-fragmentation.md
---

## TL;DR
The memory manager is the runtime's per-`Memory` (`memory-kinds.md`) allocator. It tracks every live `physical-instance.md` in a memory, satisfies new allocation requests from the mapper, and triggers `garbage-collection.md` when allocation pressure rises. The confusion: there's one memory manager per `Memory` handle — not one per memory kind. The system has many memory managers operating independently; they don't coordinate across memories.

## Mental model
The memory manager is `malloc` for one specific `Memory`. Each `Memory` (SYSTEM_MEM on node 3, GPU_FB_MEM on GPU 0, etc.) has its own manager that owns the storage, hands out allocations, and reclaims unused space. Unlike `malloc`, the manager knows about Legion's reference-counted instances and can trigger collection of zero-referenced ones to make room for a new request.

## Mechanism & API
The memory manager is internal — there's no public API for direct manipulation. Mapper code interacts with it through `find_or_create_physical_instance`:
```cpp
runtime->find_or_create_physical_instance(ctx, target_memory, constraints,
                                          regions, inst, created);
```

The runtime:
1. Asks the memory manager for `target_memory` to find a matching valid instance.
2. If none matches, asks for space to allocate a new one.
3. If space is tight, the manager triggers GC on its tracked instances.
4. If still no space after GC, the call fails — the mapper must pick a different memory.

**Configuration** (per `raw/website-pages/profiling.md`):
- Per-memory size flags: `-ll:csize` (SYSTEM_MEM), `-ll:fsize` (GPU_FB_MEM), `-ll:zsize` (Z_COPY_MEM), `-ll:rsize` (REGDMA_MEM), `-ll:gsize` (GLOBAL_MEM).
- Pin-memory flag: `-ll:pin 1` (default) pins CPU memory to enable GPU DMA.

**Tracing allocation** (per `raw/website-pages/debugging.md`):
```bash
CC_FLAGS=-DTRACE_ALLOCATION make
./app -level allocation=2
```
Logs every memory-manager allocation and free.

**Garbage collection logs** (per `garbage-collection.md`):
```bash
CC_FLAGS=-DLEGION_GC make
./app -level legion_gc=2 -logfile gc_%.log
legion/tools/legion_gc.py -l gc_*.log
```
The output names the memory manager that hosted each instance.

## Invariants
- Each `Memory` has **exactly one** memory manager; managers are per-handle, not per-kind.
- The manager **owns** the storage; the runtime does not allocate behind its back.
- An allocation that exceeds the configured size returns failure to the mapper — the program does not crash unless the mapper itself errors out.
- The manager triggers GC **before** failing; collection of zero-referenced instances happens automatically.
- Managers do **not** migrate instances across memories; physical instances are tied to one memory for their lifetime.

## Performance implications
- **Per-memory sizing is the first knob**: if the working set exceeds `-ll:csize`/`-ll:fsize`, allocations fail or trigger excessive GC.
- The manager's GC overhead shows up as `legion-prof.md` memory-row gaps + utility activity.
- `pitfalls/instance-fragmentation.md` is largely a manager-level symptom — many small instances with mismatched layout constraints prevent reuse.
- The `-ll:pin 1` setting (default) costs a small amount of physical RAM to keep host memory pinned for GPU DMA; turn off only if RAM is extremely tight.

## Debug signals
- **`tools/legion_gc.py`** output names the memory manager hosting each instance — leaks localized to one manager indicate either a per-memory bug or an oversized working set in that memory.
- **`-level allocation=2`** + `-DTRACE_ALLOCATION` logs every allocation; high per-second rate = allocator pressure.
- **OOM at `find_or_create_physical_instance`** = the named memory manager exhausted its space; bump the size flag or reduce the working set.

## Failure modes
- Working set exceeds memory size → allocation failures or aggressive GC; bump `-ll:*size`.
- Many small inconsistent-layout instances → `pitfalls/instance-fragmentation.md`; stabilize layout constraints.

## Source pointers
- **Runtime tree**: https://github.com/StanfordLegion/legion/tree/master/runtime/legion
- **Reference (sizing flags)**: `raw/website-pages/profiling.md`
- **Reference (alloc tracing)**: `raw/website-pages/debugging.md`

## Related
- `wiki/concepts/physical-instance.md` — what the manager allocates.
- `wiki/concepts/memory-kinds.md` — what each manager hosts.
- `wiki/concepts/realm-machine-model.md` — where each `Memory` (and its manager) lives.
- `wiki/concepts/garbage-collection.md` — what the manager triggers under pressure.
- `wiki/pitfalls/instance-fragmentation.md` — the most common manager-level symptom.

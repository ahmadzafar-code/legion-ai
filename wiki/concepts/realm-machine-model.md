---
title: Realm Machine Model
slug: realm-machine-model
summary: Realm's static description of the hardware available to an application; the universe of `Processor`s, `Memory`s, and their pairwise affinities (bandwidth, latency).
tags: [data-model, configuration, for-program-reasoning, for-perf-debug]
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
  - wiki/concepts/mapper.md
  - wiki/concepts/region-instance.md
  - wiki/concepts/event.md
  - wiki/concepts/dma-system.md
  - wiki/concepts/processor-kinds.md
  - wiki/concepts/memory-kinds.md
  - wiki/concepts/proc-mem-affinity.md
---

## TL;DR
The machine model is Realm's static description of the hardware: an enumeration of `Processor`s (CPU/GPU/UTIL/IO/OMP/Python), `Memory`s (system/GPU framebuffer/zero-copy/registered DMA/disk/HDF/file/...), and pairwise affinities (bandwidth and latency) between them. Application code reaches it via `Machine::get_machine()`; mappers query it to decide *where* tasks run and *which memory* holds each instance. The confusion: Realm processors and memories are **handles**, not threads or buffers — they name the hardware resources Realm has discovered or been configured with at startup.

## Mental model
The machine model is `/proc/cpuinfo` + `/proc/meminfo` + NUMA topology, unified across all nodes in the job. Each node contributes processors and memories; each pair gets an affinity record. Programs (and mappers) consult the model to ask "which memory is closest to this processor?" or "give me all GPUs on this node".

## Mechanism & API
```cpp
Machine m = Machine::get_machine();

// Iterate processors of a kind:
Machine::ProcessorQuery pq(m); pq.only_kind(Processor::TOC_PROC);
for (auto p : pq) { /* p is a GPU processor */ }

// Iterate memories of a kind:
Machine::MemoryQuery mq(m); mq.only_kind(Memory::GPU_FB_MEM);
for (auto mem : mq) { /* GPU framebuffer memory */ }

// Pairwise affinity:
std::vector<Machine::ProcessorMemoryAffinity> affs;
m.get_proc_mem_affinity(affs);  // for each pair: bandwidth, latency
m.get_mem_mem_affinity(...);    // memory-to-memory edges
```

**Processor kinds**:
- `LOC_PROC` — latency-optimized CPU. The default for application tasks.
- `TOC_PROC` — throughput-optimized GPU. Tasks pinned to TOC variants run GPU kernels.
- `UTIL_PROC` — runtime work (dep analysis, mapping, GC). Tune count with `-ll:util`.
- `IO_PROC` — long-running I/O operations.
- `PROC_GROUP` — group several processors to be treated as one for scheduling.
- `PROC_SET` — Kokkos/OpenMP processor set.
- `OMP_PROC` — OpenMP thread-pool processor.
- `PY_PROC` — Python interpreter task target (used by Pygion).

**Memory kinds**:
- `SYSTEM_MEM` — main system memory.
- `GPU_FB_MEM` — GPU device memory.
- `GPU_MANAGED_MEM` — CUDA managed (unified) memory.
- `GPU_DYNAMIC_MEM` — GPU memory that can grow at runtime.
- `Z_COPY_MEM` — host memory mapped for GPU access (no explicit copy needed).
- `REGDMA_MEM` — host memory registered with the NIC for RDMA. Size via `-ll:rsize`.
- `SOCKET_MEM` — NUMA-socket-local memory.
- `LEVEL1/2/3_CACHE_MEM` — explicit cache memories (rare; for testing).
- `DISK_MEM`, `HDF_MEM`, `FILE_MEM` — disk-backed memories.
- `GLOBAL_MEM` — global address space.

**ID encoding** (per `raw/tutorials/realm_02_machine_model.md`): 64-bit handles where the top 8 bits indicate object type (`1d` Processor, `1e` Memory), next 16 bits are the owner node, remaining bits are local index. Useful for debugging — you can decode a handle to see which node it lives on.

## Invariants
- The machine model is **static** for the lifetime of a Realm run; processors and memories are discovered at startup and don't change.
- Every `Processor` and `Memory` has a unique **home node**; cross-node access produces Realm messages.
- Affinity values are *relative* (bandwidth, latency are ordinal, not absolute). Compare within a single Machine snapshot.
- A given pair (proc, mem) is either affinity-related (proc can access mem) or not; "no affinity" means the proc cannot directly touch that memory — a copy through another memory is needed.

## Performance implications
- The mapper's most important decisions are driven by the machine model: pick `target_proc` of the right kind, pick a `target_memory` with high affinity to it.
- `-ll:cpu N` controls how many `LOC_PROC` are created per node. Default 1 — production runs typically set this to physical-core count.
- `-ll:gpu N` controls GPU count. `-ll:util N` controls utility processors.
- `-ll:csize`, `-ll:fsize`, `-ll:zsize`, `-ll:rsize` size the memories (`SYSTEM_MEM`, `GPU_FB_MEM`, `Z_COPY_MEM`, `REGDMA_MEM` respectively).
- Cache affinity queries inside the mapper constructor — repeatedly walking the machine in hot paths is a common source of `pitfalls/mapper-stalls.md`.

## Debug signals
- **`-ll:show_rsrv`** or `-level machine=2`: logs the discovered machine model at startup.
- **Mapper logs (`LoggingWrapper`)**: every `select_*` callback's `chosen_proc` / `chosen_memory` is interpreted against the model.
- **Out-of-memory** for a specific memory kind: bump the corresponding `-ll:*size`.

## Failure modes
- Insufficient `-ll:cpu`/`-ll:gpu` → all tasks queue on too few processors; Legion Prof shows long single-row activity.
- Insufficient `-ll:rsize` → inter-node copies hang or fall back to slow paths.

## Source pointers
- **Realm header**: https://github.com/StanfordLegion/legion/blob/master/runtime/realm/machine.h
- **Tutorial**: https://legion.stanford.edu/tutorial/realm/machine_model.html
- **Mapper reference**: https://legion.stanford.edu/mapper/

## Related
- `wiki/concepts/mapper.md` — primary consumer of the machine model.
- `wiki/concepts/region-instance.md` — allocated inside a specific `Memory`.
- `wiki/concepts/event.md` — Realm runtime that the machine model is part of.
- `wiki/concepts/dma-system.md` — routes copies across the memory topology.

---
title: Memory Kinds
slug: memory-kinds
summary: The taxonomy of memory types Realm exposes (SYSTEM_MEM, GPU_FB_MEM, Z_COPY_MEM, REGDMA_MEM, DISK_MEM, ...); what the mapper queries when picking instance placement.
tags: [data-model, configuration, memory, for-perf-debug, for-program-reasoning]
subsystem: realm
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/realm_02_machine_model.md
  - raw/website-pages/mapper.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/realm/memory.h
related:
  - wiki/concepts/realm-machine-model.md
  - wiki/concepts/physical-instance.md
  - wiki/concepts/mapper.md
  - wiki/concepts/processor-kinds.md
  - wiki/concepts/proc-mem-affinity.md
  - wiki/concepts/cuda-interop.md
---

## TL;DR
A memory kind is a category Realm uses to type each `Memory` handle. Common kinds: `SYSTEM_MEM` (main system RAM), `GPU_FB_MEM` (GPU framebuffer), `Z_COPY_MEM` (host memory mapped for GPU access), `REGDMA_MEM` (registered for NIC RDMA), `SOCKET_MEM` (NUMA-socket-local), `DISK_MEM` / `HDF_MEM` / `FILE_MEM` (storage-backed), `GLOBAL_MEM` (global), `GPU_MANAGED_MEM`/`GPU_DYNAMIC_MEM` (CUDA unified/dynamic). Mappers pick the right kind for each physical instance using `proc-mem-affinity.md`. The confusion: memory kind is not about *size* — it's about *accessibility*. `SYSTEM_MEM` is always large but slow for GPU access; `GPU_FB_MEM` is smaller but fast for the GPU.

## Mental model
Memory kinds are NUMA-style topology made explicit and machine-readable. Each `Memory` handle says "I'm this kind, here's my home node"; the mapper picks the right kind for each physical instance based on the processor that's going to consume it. Where a NUMA-aware program calls `numa_alloc_onnode`, a Legion mapper calls `find_or_create_physical_instance(target_mem, ...)` with the right `target_mem`.

## Mechanism & API
The 13 kinds (per `raw/tutorials/realm_02_machine_model.md`):

| Kind | Purpose | Tuning flag |
|---|---|---|
| `SYSTEM_MEM` | Main CPU memory. | `-ll:csize N` (MB) |
| `GPU_FB_MEM` | GPU framebuffer (device-local). | `-ll:fsize N` (MB per GPU) |
| `Z_COPY_MEM` | Zero-copy: host memory mapped for GPU DMA-free access. | `-ll:zsize N` (MB) |
| `REGDMA_MEM` | Memory registered with the NIC for RDMA. Required for high-BW inter-node. | `-ll:rsize N` (MB) |
| `SOCKET_MEM` | NUMA-socket-local memory. | — |
| `GPU_MANAGED_MEM` | CUDA unified memory. | — |
| `GPU_DYNAMIC_MEM` | GPU memory that can grow dynamically. | — |
| `LEVEL1_CACHE_MEM` / `LEVEL2_CACHE_MEM` / `LEVEL3_CACHE_MEM` | Explicit cache memories (rare; for testing). | — |
| `GLOBAL_MEM` | Global address space. | `-ll:gsize N` (MB) |
| `DISK_MEM` | Disk-backed memory. | — |
| `HDF_MEM` | HDF5 file-backed memory. | — |
| `FILE_MEM` | Generic file-backed memory. | — |

**Query from a mapper**:
```cpp
Machine::MemoryQuery mq(machine);
mq.only_kind(Memory::GPU_FB_MEM);
for (auto m : mq) { /* per-GPU framebuffer memories */ }
```

**Constrain an instance to a memory kind** (`instance-layout.md`):
```cpp
LayoutConstraintSet constraints;
constraints.add_constraint(MemoryConstraint(Memory::GPU_FB_MEM));
runtime->find_or_create_physical_instance(ctx, target_mem, constraints, ...);
```

The mapper passes both `target_mem` (a specific memory handle) and a `MemoryConstraint` (the kind); the runtime allocates the instance there.

## Invariants
- A `Memory` handle has exactly **one kind**, fixed at startup.
- Each kind has its own **home node** and **size** (set by tuning flags).
- `Z_COPY_MEM` is a special case: physically host memory, but addressable from GPUs via DMA-free mapping. Slower per-access than `GPU_FB_MEM` but no explicit copy.
- `REGDMA_MEM` is required for high-bandwidth inter-node copies via NIC RDMA. Default size is 0; you must opt in with `-ll:rsize`.
- Mismatched memory kind for the consuming processor's kind → automatic DMA (`dma-system.md`) at the boundary.

## Performance implications
- **Mapper instance placement** is the most influential perf decision; picking the right memory kind for each processor saves DMAs.
- Insufficient `-ll:csize` / `-ll:fsize` for the working set → instance allocation fails or memory pressure forces GC churn (`pitfalls/instance-fragmentation.md`).
- `Z_COPY_MEM` trades per-access latency for elimination of copies — good for small frequently-accessed data shared host/device, bad for large GPU kernels.
- `REGDMA_MEM` is the bottleneck on inter-node bandwidth — default size of 0 means **no RDMA**; bump it with `-ll:rsize` for multi-node runs.

## Debug signals
- **Legion Prof memory rows** are labeled by kind. Lots of activity on `SYSTEM_MEM` when GPU work is happening → instances landed in the wrong kind (see `pitfalls/excessive-data-movement.md`).
- **Out-of-memory at instance creation** → tune `-ll:csize` / `-ll:fsize` / `-ll:zsize` / `-ll:rsize` for the affected kind.
- **Inter-node DMAs hanging or slow** → almost certainly `-ll:rsize` is too small.
- **`-level dma=2`** logs every DMA operation with source and destination kinds.

## Failure modes
- Placing GPU-consumed instances in `SYSTEM_MEM` → DMA per access; `pitfalls/gpu-underutilization.md` symptoms.
- Allocating large data without sizing `-ll:csize`/`-ll:fsize` appropriately → OOM.

## Source pointers
- **Realm header**: https://github.com/StanfordLegion/legion/blob/master/runtime/realm/memory.h
- **Tutorial**: `raw/tutorials/realm_02_machine_model.md`
- **Mapper reference**: `raw/website-pages/mapper.md`

## Related
- `wiki/concepts/realm-machine-model.md` — holds the memory taxonomy.
- `wiki/concepts/physical-instance.md` — what lives in each kind.
- `wiki/concepts/mapper.md` — the picker.
- `wiki/concepts/processor-kinds.md` — sibling taxonomy.
- `wiki/concepts/proc-mem-affinity.md` — pairs them.

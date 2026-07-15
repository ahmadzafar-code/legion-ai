---
title: DMA System
slug: dma-system
summary: Realm's data-movement engine; issues structured copies, unstructured (gather/scatter) copies, fills, and reductions between region instances via per-memory-pair DMA channels.
tags: [memory, distributed, for-perf-debug]
subsystem: realm
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/realm_07_copies_and_fills.md
github:
  - https://github.com/StanfordLegion/legion/tree/master/runtime/realm
related:
  - wiki/concepts/region-instance.md
  - wiki/concepts/event.md
  - wiki/concepts/realm-machine-model.md
  - wiki/concepts/physical-analysis.md
  - wiki/concepts/legion-prof.md
---

## TL;DR
The DMA system is Realm's subsystem that actually moves bytes between `RegionInstance`s. When Legion's physical analysis decides a chosen instance needs data that lives elsewhere, it issues a `copy` (or `fill` or `reduction`); the DMA system routes the operation through one of its per-memory-pair channels and returns a Realm `Event` that triggers on completion. The confusion: the channel-row bars in Legion Prof are **DMA operations** — each one is the system saying "I had to move N bytes from memory X to memory Y to satisfy a later read".

## Mental model
The DMA system is to memories what Realm's processors are to compute: a set of asynchronous engines that consume events and produce events. Each pair of memories with a viable transport (NIC RDMA, NVLink, PCIe, shared-mem memcpy, GPU DMA, file IO) has a channel; the system picks the channel automatically based on the source/destination pair.

## Mechanism & API
The application or the higher-level runtime issues:
- **Structured copy**: dense source and destination index spaces (typically a single bounding rectangle).
  ```cpp
  std::vector<CopySrcDstField> srcs(1), dsts(1);
  srcs[0].set_field(inst_src, FID, sizeof(int));
  dsts[0].set_field(inst_dst, FID, sizeof(int));
  Event done = index_space.copy(srcs, dsts, ProfilingRequestSet(), wait_on);
  ```
- **Unstructured copy** (gather/scatter, "indirect"): the source/destination addressing isn't dense — a field holds the indirection.
- **Fill**: write a constant value into a destination.
  ```cpp
  index_space.fill(dsts, ProfilingRequestSet(), &fill_value, sizeof(fill_value), wait_on);
  ```
- **Reduction copy**: apply a reduction operator to merge source values into the destination.

All four return a Realm `Event` (`event.md`). They're asynchronous; the operation begins once the precondition event triggers and the channel has capacity.

Channel selection happens internally: the DMA system inspects the source/destination memory pair and picks an appropriate channel. Examples of channels Realm provides:
- **Local memcpy** for SYSTEM_MEM ↔ SYSTEM_MEM on the same node.
- **PCIe DMA** for SYSTEM_MEM ↔ GPU_FB_MEM.
- **NVLink** for GPU_FB_MEM ↔ GPU_FB_MEM on the same node when available.
- **GASNet active messages / RDMA** for cross-node copies; requires registered memory (`-ll:rsize`).
- **File channel** for DISK_MEM and HDF_MEM I/O.

For unstructured copies, the source and destination index spaces are first intersected (`IndexSpace::compute_intersection`) and the resulting index space drives the actual copy.

## Invariants
- Every DMA operation returns an `Event` that triggers **once** when the operation completes.
- The DMA system is **non-blocking from the caller's perspective**; it queues, schedules, and runs asynchronously.
- Source and destination instances must each remain valid through the operation's lifetime; destroying an instance with an in-flight DMA is undefined.
- The DMA system **may merge** small copies in flight or split large ones — application code cannot rely on a 1:1 op:transfer mapping.
- Reduction copies require the destination to be a reduction-capable instance and a registered `ReductionOpID`.

## Performance implications
- **Channel rows in Legion Prof = the DMA system at work.** A busy channel row indicates many bytes are moving. Reducing this is usually a mapper change (co-locate instances with compute) or a partition change (reduce halo overlap).
- Inter-node copies need **registered memory** (`-ll:rsize`). Zero by default; tune up for high-bandwidth distributed runs.
- GPU↔GPU on the same node uses **NVLink** when available — much faster than going through SYSTEM_MEM.
- Many small copies are far slower than one large copy due to per-op channel-acquire cost; coarsen partitions when fragmentation hurts.
- **Unstructured copies** are dramatically more expensive than structured — they require an intersection computation and irregular addressing.

## Debug signals
- **Legion Prof channel rows**: each bar is a DMA op. Heavy activity = data movement bottleneck (see `pitfalls/excessive-data-movement.md`).
- **`-level dma=2`**: logs per-op DMA scheduling and completion.
- **`-level activemsg=2`**: cross-node copies route through active messages; logs reveal network behavior.
- **Inter-node copy hang**: usually missing or insufficient registered memory; bump `-ll:rsize`.

## Failure modes
- [Excessive data movement](../pitfalls/excessive-data-movement.md) — channel rows dominate the profile.
- [GPU underutilization](../pitfalls/gpu-underutilization.md) — channel activity on SYSTEM_MEM↔GPU_FB starves the GPU.

## Source pointers
- **Realm runtime**: https://github.com/StanfordLegion/legion/tree/master/runtime/realm
- **Tutorial**: https://legion.stanford.edu/tutorial/realm/index_space_copy_fill.html (mirrored at `raw/tutorials/realm_07_copies_and_fills.md`)
- **Paper (Realm)**: `raw/publications/pdfs/realm2014.pdf`

## Related
- `wiki/concepts/region-instance.md` — what the DMA system moves between.
- `wiki/concepts/event.md` — what DMA operations produce.
- `wiki/concepts/realm-machine-model.md` — the memory taxonomy DMA routes across.
- `wiki/concepts/physical-analysis.md` — Legion stage that decides when copies are needed.
- `wiki/concepts/legion-prof.md` — where DMA activity shows up.

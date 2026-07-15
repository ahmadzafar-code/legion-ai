---
title: CUDA Interop
slug: cuda-interop
summary: Realm's GPU integration — register tasks against `TOC_PROC`, allocate framebuffer/managed/zero-copy memory through Realm, attach pre-existing CUDA arrays, and let Realm orchestrate streams/events. The substrate every Legion GPU debug story rests on.
tags: [gpu, instances, memory, for-perf-debug, for-program-reasoning]
subsystem: realm
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/realm_14_cuda_interop.md
github:
  - https://github.com/StanfordLegion/legion/tree/master/runtime/realm/cuda
related:
  - wiki/concepts/processor-kinds.md
  - wiki/concepts/memory-kinds.md
  - wiki/concepts/region-instance.md
  - wiki/concepts/realm-machine-model.md
  - wiki/pitfalls/gpu-underutilization.md
---

## TL;DR
Realm exposes CUDA as a first-class concern: register tasks against `TOC_PROC` (throughput-optimized core / GPU processor), allocate region instances in GPU memory kinds (`GPU_FB_MEM`, `GPU_DYNAMIC_MEM`, `GPU_MANAGED_MEM`, `Z_COPY_MEM`), attach pre-existing `cudaArray`/CUDA pointers as external resources, and write CUDA kernels that take Realm `AffineAccessor`s as arguments. Realm sets up the CUDA context, runs the task body on the right device, and handles stream/event coordination. The confusion: "CUDA interop" isn't a special integration — it's just the standard Realm API targeting `TOC_PROC` instead of `LOC_PROC`. Most GPU debugging stories (`pitfalls/gpu-underutilization.md`, channel-row activity, missing variants) reduce to "did this code use the CUDA interop correctly?".

## Mental model
CUDA interop is "GPU as just another processor kind". Realm pre-creates the CUDA context per GPU at startup, picks one when a `TOC_PROC` task is dispatched, ensures the task body sees the right device, and integrates CUDA's stream/event model into Realm's broader event graph. You don't `cudaSetDevice` or `cudaStreamSynchronize` — Realm does it implicitly. Where a hand-rolled CUDA app stitches kernel launches together with `cudaStream_t` + `cudaEvent_t`, a Realm CUDA app stitches them with Realm events.

## Mechanism & API

**Enable CUDA in the build**:
```bash
cmake -DLegion_USE_CUDA=ON -DCUDA_TOOLKIT_ROOT_DIR=/usr/local/cuda ..
```

**Configuration flags** (per `raw/tutorials/realm_14_cuda_interop.md`):
- `-ll:gpus N` — associate the first N GPUs.
- `-ll:gpu_ids x,y,z` — pin to specific CUDA device IDs.
- `-cuda:skipgpus N` — stride between chosen GPUs.
- `-cuda:hostreg N` — host memory pinned for DMA (default 1 GiB).
- `-cuda:contextsync` — force `cuCtxSynchronize` after each `TOC_PROC` task (debug-only; slow).
- `-cuda:legacysync` — track GPU progress via legacy stream events.
- `-cuda:ipc` — use legacy CUDA IPC for inter-process GPU memory sharing.

**Register a task against `TOC_PROC`**:
```cpp
Processor::register_task_by_kind(
    Processor::TOC_PROC, false /*!global*/, GPU_TASK_ID,
    CodeDescriptor(gpu_task), ProfilingRequestSet(), 0, 0).wait();
```

**Launch on a GPU processor**:
```cpp
Processor p = Machine::ProcessorQuery(Machine::get_machine())
                  .only_kind(Processor::TOC_PROC).first();
Event e = p.spawn(GPU_TASK_ID, &args, sizeof(args));
```

**Allocate framebuffer memory**:
```cpp
Memory gpu_mem = Machine::MemoryQuery(Machine::get_machine())
                     .has_capacity(bounds.volume() * sizeof(float))
                     .best_affinity_to(gpu).first();
RegionInstance::create_instance(inst, gpu_mem, bounds, field_sizes,
                                /*SOA=*/1, ProfilingRequestSet());
```

**Memory kinds for GPU work** (per `raw/tutorials/realm_14_cuda_interop.md`):

| Kind | Maps to | Use case |
|---|---|---|
| `GPU_FB_MEM` | Pre-allocated `cudaMalloc` (sized via `-ll:fsize`) | Standard device-local data |
| `GPU_DYNAMIC_MEM` | Per-instance `cudaMalloc` | Variable-size GPU buffers |
| `GPU_MANAGED_MEM` | `cudaMallocManaged` (sized via `-ll:msize`) | Unified-memory data |
| `Z_COPY_MEM` | `cudaMallocHost` (sized via `-ll:zsize`) | Host+device-accessible data |

**External CUDA arrays** (for `cudaSurfaceObject_t`-style irregular access):
```cpp
ExternalCudaArrayResource cuda_array_external(gpu_idx, array);
RegionInstance::create_external_instance(
    array_instance, cuda_array_external.suggested_memory(),
    layout.clone(), cuda_array_external, ProfilingRequestSet());
```

**Inside a GPU task**:
- `AffineAccessor<float, 2> acc(physical_region, FID_X);` — Realm gives you a device-side pointer + stride.
- Launch CUDA kernels normally; pass `acc` to the kernel. The kernel uses `acc[Point<2>(x,y)]` for typed reads.
- Don't call `cudaDeviceSynchronize` / `cudaStreamSynchronize` — Realm coordinates streams via events.

## Invariants
- `TOC_PROC` tasks **run with the device context pre-set** by Realm; the task body can launch CUDA kernels without setting the device.
- One `TOC_PROC` per GPU; the mapping is fixed at startup based on `-ll:gpus` and `-ll:gpu_ids`.
- Realm coordinates **stream usage internally**; user kernels should target the default stream (or query Realm's stream API where available).
- Calling `cuda*Synchronize` in a task body **blocks Realm's cooperative scheduling** — strongly discouraged (per the tutorial's best practices).
- `Z_COPY_MEM` is host memory mapped for GPU access — slower per-access than `GPU_FB_MEM`, but no DMA needed.
- GPU memory regions can be `Pre-allocated` at startup (`GPU_FB_MEM`/`GPU_MANAGED_MEM`/`Z_COPY_MEM`) or grown per-instance (`GPU_DYNAMIC_MEM`).

## Performance implications
- **Match `TOC_PROC` variants to `GPU_FB_MEM` instances** for hot data. Falling through to `SYSTEM_MEM` forces a DMA per access (`pitfalls/excessive-data-movement.md`).
- **`Z_COPY_MEM`** trades per-access latency for elimination of explicit copies. Good for small frequently-shared data; bad for large hot loops.
- **`GPU_DYNAMIC_MEM`** is slower to allocate but doesn't pre-commit a fixed framebuffer slice; useful for adaptive workloads. Default is `-ll:fsize`'s value.
- Avoid `cudaDeviceSynchronize` / `cuStreamSynchronize` in task bodies — they defeat Realm's async scheduling.
- Use `LOC_PROC` for CPU-bound checking / orchestration tasks, not `TOC_PROC`. `TOC_PROC`s are scarce (one per GPU) and CPU work blocks GPU dispatch.

## Debug signals
- **`-cuda:contextsync`** forces synchronization after every `TOC_PROC` task — much slower but easier to diagnose CUDA errors. Debug-only.
- **`-cuda:nohijack`** silences a Realm warning about CUDA hijack.
- **`-cuda:skipbusy`** skips GPUs that fail initialization.
- **`legion-prof.md` channel rows** show DMAs between `SYSTEM_MEM` and `GPU_FB_MEM`; lots of activity means data is misplaced.
- **GPU rows empty despite a registered TOC_PROC variant**: the mapper picked a CPU. Verify with `mapper-logging.md`.
- **`cudaErrorIllegalAddress` or similar**: typically a bad accessor or unsupported pointer escape; rebuild with `-DBOUNDS_CHECKS` and rerun.

## Failure modes
- Calling `cuda*Synchronize` in a `TOC_PROC` task body → degraded throughput, possible deadlocks.
- GPU variant not registered → mapper falls back to CPU; `pitfalls/gpu-underutilization.md`.
- Instance in `SYSTEM_MEM` consumed by `TOC_PROC` task → per-access DMA; throughput collapse.
- Mixing CUDA streams without Realm's coordination → race conditions on completion ordering.

## Source pointers
- **CUDA runtime tree**: https://github.com/StanfordLegion/legion/tree/master/runtime/realm/cuda
- **Tutorial**: `raw/tutorials/realm_14_cuda_interop.md`

## Related
- `wiki/concepts/processor-kinds.md` — `TOC_PROC` is the GPU processor kind.
- `wiki/concepts/memory-kinds.md` — `GPU_FB_MEM`, `GPU_DYNAMIC_MEM`, `GPU_MANAGED_MEM`, `Z_COPY_MEM`.
- `wiki/concepts/region-instance.md` — the substrate for GPU buffers.
- `wiki/concepts/realm-machine-model.md` — how Realm discovers GPUs at startup.
- `wiki/pitfalls/gpu-underutilization.md` — the most common CUDA-interop debug story.

---
title: Excessive Data Movement
slug: excessive-data-movement
summary: Channel rows in Legion Prof are dominated by copies between memories; physical instances are not co-located with the processors that consume them.
tags: [for-perf-debug, instances, memory, mapping]
status: draft
created: 2026-05-15
updated: 2026-05-15
related:
  - wiki/concepts/physical-instance.md
  - wiki/concepts/mapper.md
  - wiki/concepts/map-task.md
  - wiki/concepts/dma-system.md
  - wiki/concepts/realm-machine-model.md
  - wiki/concepts/legion-prof.md
---

## Symptom

- **Channel rows** in Legion Prof's `timeline-view.md` are nearly as busy as processor rows — the DMA system is moving data almost continuously.
- The `critical-path.md` (press `a` in the profiler) runs through channel-row bars, not through compute.
- Wall-clock time scales with **data volume** more than with task count — adding processors doesn't help; data movement is the bottleneck.
- For GPU runs: persistent **`SYSTEM_MEM → GPU_FB_MEM`** activity, suggesting the GPU is repeatedly fed from host memory.

## Cause

The mapper places instances in memories that are **far from the consuming processor**. Three patterns:

1. **Instance in `SYSTEM_MEM`, consumer on GPU** (`TOC_PROC`): every invocation requires a host→device DMA. The GPU stalls waiting for data each call. Common when `map_task.md` picks the local CPU's preferred memory by default and forgets to consult `proc_mem_affinity` for the GPU target.

2. **Instance recreation across iterations**: each iteration's mapper picks a different memory for the same logical region (related to `mapper-bouncing`), so the runtime issues a copy to migrate the data. Visible as DMA every iteration even though the data isn't being modified.

3. **Aliased subregions touched by tasks on different processors**: e.g., a ghost-cell halo computed on processor A and read by processor B's stencil. Each iteration requires the halo to ship A→B (real, unavoidable communication, but worth measuring) — but if the partition is wrong, the halo may be larger than necessary.

A subtler case: **virtually-mapped regions in inner tasks** (`virtual-mapping.md` + `inner-task.md`) are placed by the subtasks. If subtasks each pick different memories, the runtime emits copies between them.

## Fix

- **In `map_task`, query `proc_mem_affinity`** and pick a memory close to `target_proc`. `default_policy_select_target_memory` from `default-mapper.md` does this for you — call it instead of hard-coding.
- **For shared host/device data**, use `Z_COPY_MEM` (zero-copy memory accessible from both CPU and GPU) — single instance, no explicit DMA. Trade-off: slower per-access than `GPU_FB_MEM` but no copy.
- **`postmap_task`**: after `map_task`, request a prefetch copy into the next consumer's memory. The runtime overlaps the copy with the current task's execution.
- **For long-running loops, premap once outside the loop**: create the instance in the right memory once, hold a reference, and let every iteration reuse it. Combined with **tracing** (`tracing.md`) this collapses to one cold-path copy plus N free replays.
- **For halo-style patterns**, partition so ghost cells are minimum-size; check disjointness with `partition-checks.md`.

After the fix, channel rows should be dominated by **one-time, structural copies** (initial data distribution, halo exchange) — not per-iteration churn. The critical path should run through compute.

## Underlying concepts

- `wiki/concepts/physical-instance.md` — what's being moved.
- `wiki/concepts/mapper.md` / `wiki/concepts/map-task.md` — where placement is decided.
- `wiki/concepts/dma-system.md` — the engine doing the moving (and producing the channel-row bars).
- `wiki/concepts/realm-machine-model.md` — the affinity model the mapper queries.
- `wiki/concepts/legion-prof.md` — the timeline where the symptom is visible.

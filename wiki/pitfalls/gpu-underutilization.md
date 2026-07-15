---
title: GPU Underutilization
slug: gpu-underutilization
summary: GPU processor rows in Legion Prof show large idle gaps while CPU rows are busy; mapping decisions or data placement are starving the GPUs.
tags: [for-perf-debug, mapping, gpu, memory]
status: draft
created: 2026-05-15
updated: 2026-05-15
related:
  - wiki/concepts/mapper.md
  - wiki/concepts/physical-instance.md
  - wiki/concepts/task.md
  - wiki/concepts/legion-prof.md
  - wiki/concepts/cuda-interop.md
  - wiki/workflows/write-a-custom-mapper.md
---

## Symptom
- **Legion Prof GPU rows** (TOC_PROC) are mostly empty bars; CPU rows are saturated.
- Critical path (press `a`) runs through CPU rows that hand work to GPU rows just often enough to keep them limping.
- `nvidia-smi` shows low GPU utilization while the Legion app is "running".

## Cause
Three independent candidates — usually one or two are active at a time. Confirm in Legion Prof:

1. **No GPU variant exists for the relevant task.** Registration only added a `LOC_PROC` variant, so the mapper has no GPU candidate to pick. Symptom: zero bars on TOC rows during that task's window.
2. **The mapper placed the task on a CPU.** A GPU variant exists, but `map_task` returned a CPU processor (or the default mapper's cost model preferred CPU). Symptom: TOC rows are empty but CPU rows show the task running, and `LoggingWrapper` confirms the placement.
3. **Data is on the wrong memory.** The task is on the GPU but its physical instance is in `SYSTEM_MEM`. The runtime issues a host→device DMA each invocation, visible as **busy channel rows** between SYSTEM_MEM and GPU_FB. The GPU sits idle while the copy completes.

## Fix
- **Register a GPU variant.** Add a `TaskVariantRegistrar` with `ProcessorConstraint(Processor::TOC_PROC)` for the same task ID, with a CUDA-aware implementation. The mapper now has a candidate.
- **Steer the mapper.** In `select_task_options` set `output.initial_proc` to a TOC processor when the task ID is GPU-eligible:
  ```cpp
  if (task.task_id == GPU_TASK_ID) {
    Machine::ProcessorQuery pq(machine); pq.only_kind(Processor::TOC_PROC);
    output.initial_proc = pq.first();
  }
  ```
  Or override `map_task` to set `output.target_procs` to TOC processors.
- **Place instances in GPU memory.** In `map_task`, create the chosen instance with `MemoryConstraint(Memory::GPU_FB_MEM)` for the matching `target_proc`. Use `default_policy_select_target_memory` from `DefaultMapper` for sensible defaults.
- **For shared host/device data**, use `Memory::Z_COPY_MEM` (zero-copy) — single instance accessible from both, no explicit DMA.
- **Confirm**: re-run with `LoggingWrapper` + `-level mapper=2` and verify both `chosen_instances` and `target_procs` are GPU-side. In Legion Prof, channel-row activity between SYSTEM_MEM and GPU_FB should vanish.

## Underlying concepts
- `wiki/concepts/mapper.md` — where placement decisions are made.
- `wiki/concepts/physical-instance.md` — where the data lives and what layout it has.
- `wiki/concepts/task.md` — variants and processor constraints.
- `wiki/concepts/legion-prof.md` — channel-row vs processor-row signals.
- `wiki/concepts/cuda-interop.md` — the full Realm GPU surface (`TOC_PROC`, GPU memory kinds, `-cuda:*` flags).

---
title: Processor Kinds
slug: processor-kinds
summary: The taxonomy of processor types Realm exposes (LOC_PROC, TOC_PROC, UTIL_PROC, IO_PROC, OMP_PROC, PY_PROC, PROC_GROUP, PROC_SET); what the mapper queries when picking task placement.
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
  - https://github.com/StanfordLegion/legion/blob/master/runtime/realm/processor.h
related:
  - wiki/concepts/realm-machine-model.md
  - wiki/concepts/mapper.md
  - wiki/concepts/task-variant.md
  - wiki/concepts/memory-kinds.md
  - wiki/concepts/proc-mem-affinity.md
  - wiki/concepts/cuda-interop.md
---

## TL;DR
A processor kind is a category Realm uses to type each `Processor` handle. There are eight kinds: `LOC_PROC` (latency-optimized CPU), `TOC_PROC` (throughput-optimized GPU), `UTIL_PROC` (runtime-work helper), `IO_PROC` (I/O-bound work), `OMP_PROC` (OpenMP thread-pool), `PY_PROC` (Python interpreter), `PROC_GROUP` (cluster of processors), `PROC_SET` (Kokkos-style set). Mappers query processor kinds to decide where to send tasks; task variants are registered with a `ProcessorConstraint` to declare which kinds they can run on. The confusion: processor "kinds" aren't operating-system-level threads — they're Realm-level scheduling abstractions; a single physical core can host multiple Realm processors of different kinds.

## Mental model
Processor kinds are Realm's processor "type system". Each kind has its own scheduling semantics (CPU = latency, GPU = throughput, util = runtime overhead). The mapper picks one kind per task based on the task's registered variants and the application's performance goals. Where Linux groups threads by priority/affinity, Realm groups *Processor handles* by kind.

## Mechanism & API
The eight kinds (per `raw/tutorials/realm_02_machine_model.md`):

| Kind | Purpose | Tuning flag |
|---|---|---|
| `LOC_PROC` | **L**atency-**o**ptimized **c**ore — standard CPU. Default for app tasks. | `-ll:cpu N` |
| `TOC_PROC` | **T**hroughput-**o**ptimized **c**ore — GPU. | `-ll:gpu N` |
| `UTIL_PROC` | Utility processor — runtime work (mapper, dep analysis). | `-ll:util N` |
| `IO_PROC` | I/O-bound work; pools threads for blocking ops. | `-ll:io N` |
| `OMP_PROC` | OpenMP-style thread-pool processor. | `-ll:ocpu N` (and `-ll:onuma`) |
| `PY_PROC` | Python interpreter task target (used by Pygion). | `-ll:py N` |
| `PROC_GROUP` | A user-defined group of processors treated as one for scheduling. | — |
| `PROC_SET` | Kokkos-style set of processors. | — |

**Query from a mapper**:
```cpp
Machine::ProcessorQuery pq(machine);
pq.only_kind(Processor::TOC_PROC);
for (auto p : pq) { /* GPU processors */ }
```

**Constrain a task variant to a kind** (`task-variant.md`):
```cpp
TaskVariantRegistrar reg(GPU_TASK_ID, "gpu_kernel");
reg.add_constraint(ProcessorConstraint(Processor::TOC_PROC));
Runtime::preregister_task_variant<gpu_kernel>(reg, "gpu_kernel");
```

The runtime refuses to map a task whose chosen processor's kind doesn't match the variant's `ProcessorConstraint`.

## Invariants
- A `Processor` has exactly **one kind**, fixed at startup.
- The kind is the **most coarse-grained** filter on processor selection; finer choice happens via affinities (`proc-mem-affinity.md`).
- `UTIL_PROC` is reserved for **runtime work** — your tasks should not target it directly. Increasing `-ll:util` adds runtime-side bandwidth, not application bandwidth.
- `TOC_PROC` does **not** mean "runs CUDA code" — it means "Realm picked this processor for throughput-optimized work". The application's GPU variant has to be registered and the mapper has to pick the variant; `TOC_PROC` is the substrate.
- The runtime creates one Realm task queue per processor; processors of the same kind do not necessarily share queues.

## Performance implications
- **The mapper's first decision is processor kind**: send to `LOC_PROC`, `TOC_PROC`, `OMP_PROC`, etc. Get this wrong and everything else fails.
- Insufficient `-ll:cpu` / `-ll:gpu` for the workload → queueing on too few processors; visible in `legion-prof.md` as long single-row activity.
- `-ll:util` controls runtime-side parallelism for `mapper-callback.md`s, dependence analysis, GC. Too few → `pitfalls/mapper-stalls.md`.
- Tasks with **no GPU variant** can only target `LOC_PROC` regardless of how much `-ll:gpu` you set → `pitfalls/gpu-underutilization.md`.

## Debug signals
- **Legion Prof row labels** show each processor's kind. Compare actual task placement against intent.
- **`LoggingWrapper`** logs name the chosen processor's kind in `target_procs` output.
- **Error around "no valid variant" at submit time** → the mapper picked a kind no registered variant supports for this task.

## Failure modes
- Registering only a `LOC_PROC` variant for a task and expecting GPU execution → `pitfalls/gpu-underutilization.md`.
- Setting `-ll:gpu N` for a workload with no `TOC_PROC` variants → wasted hardware.

## Source pointers
- **Realm header**: https://github.com/StanfordLegion/legion/blob/master/runtime/realm/processor.h
- **Tutorial**: `raw/tutorials/realm_02_machine_model.md`
- **Mapper reference**: `raw/website-pages/mapper.md`

## Related
- `wiki/concepts/realm-machine-model.md` — the static description that holds processor kinds.
- `wiki/concepts/mapper.md` — what queries them.
- `wiki/concepts/task-variant.md` — what constrains itself to them.
- `wiki/concepts/memory-kinds.md` — sibling taxonomy.
- `wiki/concepts/proc-mem-affinity.md` — pairs them with memories.

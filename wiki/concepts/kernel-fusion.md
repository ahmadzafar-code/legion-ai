---
title: Kernel Fusion
slug: kernel-fusion
summary: A code-generation optimization that merges multiple compute kernels (typically GPU) inside a fused task into a single kernel; eliminates kernel-launch latency and lets intermediate values stay in registers/shared memory.
tags: [execution, gpu, for-perf-debug]
subsystem: legion
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/publications/publications.md
github:
  - https://github.com/StanfordLegion/legion/tree/master/runtime/legion
related:
  - wiki/concepts/task-fusion.md
  - wiki/concepts/regent-language.md
  - wiki/concepts/operation-pipeline.md
  - wiki/concepts/physical-instance.md
---

## TL;DR
Kernel fusion is the **code-generation-level** sibling of `task-fusion.md`: once two tasks are fused at the runtime level, the kernels inside them are merged at the compiled-code level. The fused kernel reads inputs once, computes both operations, and writes outputs once — keeping intermediates in registers or shared memory instead of materializing them in DRAM. Subject of the paper *Composing Distributed Computations Through Task and Kernel Fusion* (`fusion2025.pdf`, ASPLOS 2025). The confusion: kernel fusion requires compiler support and is most natural for **Regent** (`regent-language.md`) tasks, where the compiler controls code generation. For hand-written C++/CUDA tasks, kernel fusion is the application's responsibility (write the fused kernel explicitly).

## Mental model
Kernel fusion is loop fusion in compiler-speak, lifted to distributed task graphs. Where a CPU compiler fuses adjacent `for` loops to keep intermediate values in cache, the Legion + Regent toolchain fuses task bodies so intermediate buffers stay in GPU registers/shared memory. The runtime + compiler together identify the opportunity (`task-fusion.md` at runtime, kernel fusion at codegen).

## Mechanism & API
The mechanism (per `raw/publications/publications.md` ASPLOS 2025 entry):
1. The runtime identifies a fusion-eligible task sequence via `task-fusion.md`.
2. The Regent compiler (or a custom toolchain) lowers the fused tasks' bodies into a single kernel.
3. The fused kernel is registered as a new task variant.
4. The runtime dispatches the fused variant when the fusion conditions are met.

**For Regent applications**: the compiler can do this automatically when the task bodies are written in Regent's IR. No manual kernel-merging needed.

**For C++ + CUDA applications**: kernel fusion is largely manual. The application writes the fused kernel explicitly (e.g., a "stencil + relaxation" combined GPU kernel) and registers it. The `task-fusion.md` machinery then dispatches the fused task at the right times.

**Combining with task fusion**: kernel fusion's benefit is realized only when paired with task fusion — otherwise the runtime would dispatch each task separately and the kernels would not be in the same launch.

## Invariants
- Kernel fusion **requires compiler support** — Regent has it; C++ does not (the application must hand-write the fused kernel).
- The fused kernel must observe the same privileges and bounds as the constituent kernels — bugs at this layer manifest as correctness issues.
- Intermediates kept in registers/shared memory **must fit** — if they don't, the compiler falls back to materializing in DRAM (a partial win).
- Combining with `tracing.md` further multiplies the benefit (memoize once, replay the fused kernel many times).
- The technique applies on CPU (loop fusion via Terra/LLVM) and GPU (CUDA/HIP kernel fusion) similarly.

## Performance implications
- **The biggest perf win** comes from eliminating intermediate-buffer materialization. A stencil + relaxation that would normally write to DRAM, read it back, and continue — instead keeps the intermediate in registers, cutting bandwidth requirements.
- **Kernel-launch latency saved**: one GPU kernel launch instead of N. For small kernels this is comparable to the kernel runtime itself.
- For Regent codes, kernel fusion can produce 2-5× speedups on stencil-like workloads (per `fusion2025.pdf`).
- For C++ applications without compiler support, the win is only via task fusion + hand-written fused kernels.

## Debug signals
- **`legion-prof.md`** GPU rows: one large fused-kernel bar in place of multiple smaller ones.
- **`-level legion=2`** + compiler-side logs show the fused kernel registration.
- **Wrong output from a fused kernel** = a compiler-level fusion bug or a hand-rolled fused-kernel bug; reverting to per-task kernels isolates.

## Failure modes
- Intermediates too large for register/shared memory → fall back to DRAM materialization; the win is partial.
- Hand-written fused kernel with a bug → wrong output for the fused path.

## Source pointers
- **Paper (task + kernel fusion)**: `raw/publications/pdfs/fusion2025.pdf` — *Composing Distributed Computations Through Task and Kernel Fusion* (ASPLOS 2025).
- **Paper (task fusion alone)**: `raw/publications/pdfs/pawatm2022.pdf`.
- **Runtime tree**: https://github.com/StanfordLegion/legion/tree/master/runtime/legion

## Related
- `wiki/concepts/task-fusion.md` — the runtime-level sibling.
- `wiki/concepts/regent-language.md` — the compiler with built-in support.
- `wiki/concepts/operation-pipeline.md` — broader context.
- `wiki/concepts/physical-instance.md` — what kernel fusion lets you avoid materializing.

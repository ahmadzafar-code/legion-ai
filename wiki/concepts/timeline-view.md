---
title: Timeline View
slug: timeline-view
summary: The primary Legion Prof display; per-processor / per-memory / per-channel rows showing task execution, instance lifetimes, and inter-memory copies on a wall-clock axis.
tags: [profiling, tooling, for-perf-debug]
subsystem: cross
layer: tooling
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/website-pages/profiling.md
github:
  - https://github.com/StanfordLegion/legion/tree/master/tools/legion_prof_rs
related:
  - wiki/concepts/legion-prof.md
  - wiki/concepts/critical-path.md
  - wiki/concepts/event-graph.md
  - wiki/concepts/physical-instance.md
  - wiki/concepts/dma-system.md
---

## TL;DR
The timeline view is what you see when you open a Legion Prof profile: a horizontally-scrolling time axis with stacked rows for each processor (CPU, GPU, utility, IO), each memory (system, GPU framebuffer, zero-copy, registered DMA), and each channel (DMA pairs between memories). Each task is a colored bar; each instance is a slab on its memory's row; each copy is a bar on its channel. The confusion: rows are not threads — they're *resources*. A "busy" processor row means a task is *holding* that processor; a "busy" memory row means an instance is *occupying* that memory.

## Mental model
The timeline view is `tracy-profiler` for distributed tasking. Where a thread-based profiler shows what each thread did over time, the timeline view shows what each *resource* (CPU/GPU/memory/channel) did over time. The same task instance shows up exactly once — on the processor that ran it.

## Mechanism & API
**Capture** (from `raw/website-pages/profiling.md`):
```bash
DEBUG=0 make
./app -lg:prof <N> -lg:prof_logfile prof_%.gz
```

**Launch the viewer**:
```bash
legion_prof --view prof_*.gz       # local desktop UI
legion_prof --archive prof_*.gz -o out/   # shareable web archive
legion_prof --serve prof_*.gz             # HTTP server for remote viewing
legion_prof --attach http://host:8080     # client to a remote serve
```

**Row types and what each shows**:
- **Processor rows** (`LOC_PROC` CPU / `TOC_PROC` GPU / `UTIL_PROC` utility / `IO_PROC` I/O): each colored bar is a task or runtime operation. Color = task ID. Width = wall-clock duration.
- **Memory rows** (`SYSTEM_MEM` / `GPU_FB_MEM` / `Z_COPY_MEM` / `REGDMA_MEM` / ...): each slab shows a `physical-instance.md` occupying that memory between creation and destruction. Heavy slab activity = many instances live; thin slabs = quick allocations and frees (churn).
- **Channel rows**: each bar is a `dma-system.md` operation between a pair of memories (e.g., `SYSTEM_MEM → GPU_FB_MEM`). Heavy channel activity = lots of data movement.

**UI interactions** (per `raw/website-pages/profiling.md`):
- **Click-drag** to zoom; press `u` to undo a zoom; `0` to reset.
- **`s`** to search; `c` to clear.
- **Hover** a task bar to see name, duration, processor, memory.
- **Left-click** a task to highlight its incoming dependencies.
- **Right-click** a task to see its parent + children.
- **`a`** to toggle the `critical-path.md` overlay.

## Invariants
- Each processor row shows a single timeline — at most one task executes there at a time (modulo concurrent tasks on the same processor in special cases).
- Memory rows reflect the **physical instance** view (per `physical-instance.md`), not logical regions.
- Channel rows reflect **actual DMA operations** physical analysis emitted, not application-issued copies (though most of the time these align).
- The wall-clock axis is aligned across all nodes' rows — but **clock skew** between nodes can make message ordering look "impossible" near node boundaries.
- The profile reflects a release-build measurement; debug builds skew the timing.

## Performance implications
- The timeline view's most informative columns:
  - **Gaps on processor rows** = idle; trace what was supposed to enable the next task.
  - **Many short bars** on a processor = fine-grained tasking, possibly `pitfalls/runtime-overhead-dominates.md`.
  - **Many short slabs** on a memory row = instance churn, possibly `pitfalls/instance-fragmentation.md`.
  - **Busy channels** = data movement, possibly `pitfalls/excessive-data-movement.md`.
  - **GPU rows empty** while CPU busy = `pitfalls/gpu-underutilization.md`.

## Debug signals (what to look for, in order)
- Press `a` first for `critical-path.md` overlay — your wall-clock floor.
- Look at processor rows for big idle gaps.
- Look at memory rows for slab churn.
- Look at channel rows for copy density.
- Compare per-node rows in multi-node runs for shard skew.

## Failure modes
- Looking only at busy rows without enabling critical path → you optimize off-critical work and total time doesn't improve.
- Viewing a debug build's profile → the overhead distorts everything; rebuild with `DEBUG=0`.

## Source pointers
- **Profiler source**: https://github.com/StanfordLegion/legion/tree/master/tools/legion_prof_rs
- **Reference**: `raw/website-pages/profiling.md`

## Related
- `wiki/concepts/legion-prof.md` — host tool.
- `wiki/concepts/critical-path.md` — the `a` overlay.
- `wiki/concepts/event-graph.md` — Spy's complement showing causality instead of time.
- `wiki/concepts/physical-instance.md` — what memory rows show.
- `wiki/concepts/dma-system.md` — what channel rows show.

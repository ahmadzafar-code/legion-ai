---
title: Legion Prof
slug: legion-prof
summary: Legion's performance profiler; ingests per-node logs and renders an interactive timeline of task execution, memory instances, and inter-memory data movement.
tags: [profiling, tooling, for-perf-debug]
subsystem: cross
layer: tooling
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/website-pages/profiling.md
  - raw/youtube_transcripts/bootcamp_2017/transcripts
  - raw/youtube_transcripts/retreat_2024/transcripts
github:
  - https://github.com/StanfordLegion/legion/tree/master/tools/legion_prof_rs
related:
  - wiki/concepts/operation-pipeline.md
  - wiki/concepts/mapper.md
  - wiki/concepts/physical-instance.md
  - wiki/concepts/legion-spy.md
  - wiki/concepts/critical-path.md
  - wiki/concepts/realm-profiling.md
---

## TL;DR
Legion Prof renders a timeline of an application's execution: one row per processor (CPU/GPU/utility), one row per memory (showing instance lifetimes), one row per channel (showing inter-memory copies). It's the first tool to reach for when a Legion program is too slow. Modern Legion Prof is implemented in Rust (`legion_prof_rs`). The confusion: profiling needs a **release build** without debug/check flags — running the profiler against a debug build measures the wrong thing.

## Mental model
Legion Prof is Legion's `perf record` + `tracy` rolled into one. Where a traditional profiler shows CPU samples, Legion Prof shows the *operation graph in time*: each task is a colored bar on the processor that ran it, each instance is a slab on a memory row, each copy is a bar on a channel row. The critical-path view (press `a`) draws the longest chain through that graph — that's the line you fix first.

## Mechanism & API
**Capture:**
```bash
DEBUG=0 make   # release build, no -DPRIVILEGE_CHECKS/-DBOUNDS_CHECKS/-DLEGION_SPY
./app -lg:prof <N> -lg:prof_logfile prof_%.gz   # N = number of nodes
```
The `%` is replaced by node index → one file per node.

**Install profiler:**
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
cargo install --locked --all-features --path legion/tools/legion_prof_rs
```
Always install from the **same Legion commit** the app was built against; version skew yields silently wrong views.

**Four viewing modes:**
- `legion_prof --view prof_*.gz` — local desktop UI (default).
- `legion_prof --archive prof_*.gz -o out/` — self-contained static-web archive (shareable).
- `legion_prof --serve prof_*.gz` — HTTP server (port 8080) for remote viewing.
- `legion_prof --attach http://host:8080` — client to a remote serve.

**Key shortcuts inside the UI:**
- Click-drag = zoom; `u` = undo zoom; `0` = reset zoom.
- `s` = search; `c` = clear search; `?` = help.
- Left-click a task = highlight its dependencies (incoming).
- Right-click a task = parent + children.
- **`a` = critical path overlay.**

**Row types:**
- **Processor rows** (LOC/TOC/UTIL/IO): each colored bar is a task or runtime operation. Gaps = idle.
- **Memory rows** (SYS/GPU_FB/Z_COPY/REGDMA/...): bars show instance lifetimes.
- **Channel rows**: each bar is a copy/fill between two memories. Heavy channel activity = data movement bottleneck.

For maximum information, pair with Legion Spy: `./app -lg:prof N -lg:spy -lg:prof_logfile prof_%.gz -logfile spy_%.log` — then critical path is annotated with dependence reasons. See `legion-spy.md`.

## Invariants
- Profiles capture **wall-clock per-node activity**; clock skew between nodes can make message ordering look "impossible" — see the docs.
- Profiles **do not** capture application internals (your kernel's PMU counters); use a vendor tool (`nsys`, `perf`) for that.
- Profiles taken against a debug build (`DEBUG=1`) are dominated by check overhead and **do not reflect release behavior**.
- `-lg:prof_logfile` files are gzipped by default; profiles require ZLIB unless built with `USE_ZLIB=0`.

## Performance implications
- The capture overhead is small but non-zero; for ground-truth timings, A/B against a non-profiled run.
- Three perf factors Legion Prof helps measure (per `raw/website-pages/profiling.md`):
  1. **Overall task/data throughput** (aggregate work done).
  2. **Critical path latency** (longest dependent chain).
  3. **Runtime overhead** (utility-processor activity).

## Debug signals (what to look for)
- **Idle application processors + busy utility processors** → stuck in pipeline stage 2/4/5 (`operation-pipeline.md`).
- **Long copy bars in channel rows** → misplaced instances; visit `mapper.md`.
- **Many short instance slabs on a memory row** → fragmentation; visit `physical-instance.md`.
- **GPU rows mostly empty, CPU rows full** → no GPU variant or mapper preferred CPU.
- **Critical path (`a`) is a single chain of tasks** → false dependencies, no index launch, or missed tracing.
- **Per-shard utility activity skewed** under control replication → bad sharding functor.

## Failure modes (related pitfalls)
- [Long dependence chains](../pitfalls/long-dependence-chains.md)
- [GPU underutilization](../pitfalls/gpu-underutilization.md)
- [Excessive data movement](../pitfalls/excessive-data-movement.md)
- [Mapper stalls](../pitfalls/mapper-stalls.md)
- [Instance fragmentation](../pitfalls/instance-fragmentation.md)
- [Runtime overhead dominates](../pitfalls/runtime-overhead-dominates.md)
- [Missed tracing opportunity](../pitfalls/missed-tracing-opportunity.md)

## Source pointers
- **Source (Rust profiler)**: https://github.com/StanfordLegion/legion/tree/master/tools/legion_prof_rs
- **Reference**: https://legion.stanford.edu/profiling/ (mirrored at `raw/website-pages/profiling.md`)
- **Demo walkthrough**: `raw/youtube_transcripts/bootcamp_2017/transcripts/` Lesson 8.

## Related
- `wiki/concepts/operation-pipeline.md` — what the rows represent.
- `wiki/concepts/mapper.md` — what the timeline reveals about placement decisions.
- `wiki/concepts/physical-instance.md` — what memory + channel rows describe.
- `wiki/concepts/legion-spy.md` — for dependence and event causality (Spy complements Prof's timeline).
- `wiki/workflows/profile-an-app.md` — end-to-end profiling workflow.
- `wiki/workflows/debug-perf-bottleneck.md` — what to do when Prof points at a problem.

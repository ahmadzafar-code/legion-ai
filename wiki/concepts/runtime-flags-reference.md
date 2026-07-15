---
title: Runtime Flags Reference
slug: runtime-flags-reference
summary: Consolidated lookup table of `-ll:*` (Realm machine configuration) and `-lg:*` (Legion runtime behavior) flags; what each controls, default value, when to tune it.
tags: [configuration, debugging, tooling, for-perf-debug, for-correctness-debug]
subsystem: cross
layer: tooling
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/website-pages/profiling.md
  - raw/website-pages/debugging.md
  - raw/website-pages/getting_started.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/realm/runtime.cc
related:
  - wiki/concepts/realm-machine-model.md
  - wiki/concepts/processor-kinds.md
  - wiki/concepts/memory-kinds.md
  - wiki/concepts/operation-pipeline.md
  - wiki/workflows/profile-an-app.md
  - wiki/workflows/debug-perf-bottleneck.md
---

## TL;DR
Legion + Realm expose ~30 command-line flags that control hardware allocation, runtime scheduling, and debug behavior. They fall into two namespaces: **`-ll:*`** (low-level / Realm — processors, memories, threading) and **`-lg:*`** (Legion high-level runtime — windowing, scheduling, debug aids). This page is the consolidated reference. The confusion: every flag is mentioned somewhere in the per-concept pages, but you usually need to set 3-5 together — `-ll:cpu N -ll:gpu N -ll:csize N -ll:fsize N -ll:util N`. This table is the at-a-glance lookup.

## Mental model
Flags are configuration knobs. `-ll:*` sets up the *substrate* (how many CPUs, how big is system memory, how many utility processors). `-lg:*` tunes the *runtime* (how aggressively it schedules ahead, what debug checks fire). The Realm flags shape what `realm-machine-model.md` looks like at startup; the Legion flags shape how `operation-pipeline.md` runs.

## `-ll:*` — Realm (machine) configuration

| Flag | What it does | Default | When to tune |
|---|---|---|---|
| `-ll:cpu N` | Create N `LOC_PROC` (CPU) per node | 1 | Always set for production; typically physical cores |
| `-ll:gpu N` | Create N `TOC_PROC` (GPU) per node | 0 | Always set on GPU runs |
| `-ll:util N` | Create N `UTIL_PROC` (utility) per node | 1 | Bump to 2-4 if `pitfalls/mapper-stalls.md` or `pitfalls/runtime-overhead-dominates.md` |
| `-ll:io N` | Create N `IO_PROC` (I/O) | 0 | Set when doing significant async I/O |
| `-ll:ocpu N` | Create N `OMP_PROC` (OpenMP) | 0 | OpenMP-style task variants |
| `-ll:py N` | Create N `PY_PROC` (Python) | 0 | Pygion / Python tasks |
| `-ll:csize N` | `SYSTEM_MEM` size in MB | 512 | Bump to working-set size |
| `-ll:fsize N` | `GPU_FB_MEM` size per GPU in MB | 256 | Bump to fit working set in GPU |
| `-ll:zsize N` | `Z_COPY_MEM` size in MB | 64 | Bump for shared host/device data |
| `-ll:rsize N` | `REGDMA_MEM` (RDMA-registered) in MB | 0 | **Required for multi-node** — set 1024+ |
| `-ll:gsize N` | `GLOBAL_MEM` (GASNet global) in MB | 256 | GASNet-based distributed runs |
| `-ll:stacksize N` | Stack size per CPU thread in MB | 2 | Tasks with deep recursion / large stack arrays |
| `-ll:sdpsize N` | GASNet RDMA segment pinned memory in MB | 64 | Large inter-node transfers |
| `-ll:lmbsize N` | Max long-message buffer in MB | 1 | Large active-message payloads |
| `-ll:numlmbs N` | Long-message buffers per node pair | 2 | High-fanout messaging patterns |
| `-ll:amsg N` | Active-message handler threads | 1 | Heavy inter-node messaging |
| `-ll:bgwork N` | Background work threads | 1 | DMAs, deferred allocations |
| `-ll:pin {0,1}` | Pin CPU memory for GPU DMA | 1 | Disable only if RAM is tight |
| `-ll:force_kthreads` | Make all Realm threads kernel-visible | off | Required for `gdb`'s `thread apply all bt` |
| `-ll:onuma {0,1}` | NUMA-pin OpenMP threads | 0 | Multi-socket OpenMP workloads |
| `-ll:separate` | Force separate runtime instances (debug) | off | Inter-node protocol debugging |

## `-lg:*` — Legion (runtime) behavior

| Flag | What it does | Default | When to tune |
|---|---|---|---|
| `-lg:window N` | Max outstanding (unmapped) operations | 1024 | Increase for app with many independent tasks |
| `-lg:sched N` | Max ready tasks before pausing scheduler | 1 | Increase to discover more parallelism before execution |
| `-lg:width N` | Operations analyzed per scheduling pass | 4 | Increase to amortize stage-2 overhead |
| `-lg:message N` | Max active-message size in bytes | 4096 | Tune to network MTU |
| `-lg:filter N` | Trim instance user lists at this size | 0 (off) | Long-running apps with heavy instance reuse |
| `-lg:prof N` | Enable Legion Prof for N nodes | off | Always for profiling |
| `-lg:prof_logfile <pat>` | Profile output pattern | — | Always with `-lg:prof`; use `prof_%.gz` |
| `-lg:spy` | Enable Legion Spy logging | off | When debugging dependencies |
| `-lg:partcheck` | Verify partition disjointness at creation | off | Always during partition development; see `partition-checks.md` |
| `-lg:inorder` | Force in-order operation execution | off | Reproducing timing-dependent bugs (`in-order-execution.md`) |
| `-lg:delay N` | Sleep N seconds at startup | 0 | Pre-execution `gdb` attach (`delay-start.md`) |
| `-lg:no_tracing` | Disable tracing | off | A/B test tracing's benefit |
| `-lg:control_replication` | Enable control replication | varies | Multi-node scaling |

## Logging-related flags (orthogonal — see `logger-categories.md`)

| Flag | What it does |
|---|---|
| `-level cat=N` | Set logging level for category `cat` to N (1=spew, 2=debug, 3=info, 4=print, 5=warning, 6=error) |
| `-logfile pattern` | Route log output to files; `%` is replaced by node index |

## Environment variables (orthogonal — see individual concept pages)

| Variable | Effect |
|---|---|
| `LEGION_BACKTRACE=1` | Print stack on error (`backtrace-mode.md`) |
| `REALM_BACKTRACE=1` | Same — alias |
| `LEGION_FREEZE_ON_ERROR=1` | Pause process on error for `gdb` attach (`freeze-on-error.md`) |
| `REALM_SHOW_EVENT_WAITERS=N+M` | Dump pending event waiters after N seconds, every M thereafter |

## Compile-time flags (orthogonal — set via `CC_FLAGS=...`)

| Flag | Effect |
|---|---|
| `-DPRIVILEGE_CHECKS` | Runtime privilege verification (`privilege-checks.md`) |
| `-DBOUNDS_CHECKS` | Runtime bounds verification (`bounds-checks.md`) |
| `-DLEGION_SPY` | Enable Spy checking-mode logs |
| `-DFULL_SIZE_INSTANCES` | Allocate top-level region size for all instances |
| `-DTRACE_ALLOCATION` | Log every memory-manager allocation |
| `-DLEGION_GC` | Log garbage-collection events (`garbage-collection.md`) |

## Typical combinations

**Production GPU run on 4-node cluster** (16 cores, 4 GPUs each):
```bash
mpirun -np 4 ./app -ll:cpu 16 -ll:gpu 4 -ll:util 4 \
    -ll:csize 8000 -ll:fsize 16000 -ll:zsize 2000 -ll:rsize 4096
```

**Profile capture**:
```bash
./app -lg:prof <N> -lg:prof_logfile prof_%.gz
```

**Correctness debug session**:
```bash
DEBUG=1 CC_FLAGS="-DPRIVILEGE_CHECKS -DBOUNDS_CHECKS" make
LEGION_BACKTRACE=1 LEGION_FREEZE_ON_ERROR=1 ./app -lg:partcheck -lg:inorder
```

**Pre-execution `gdb` attach** (multi-node):
```bash
mpirun -np 4 ./app -lg:delay 30 -ll:force_kthreads
# In a separate shell:
# gdb -p <PID printed at startup>
```

## Invariants
- `-ll:*` flags configure Realm at startup; their effect is fixed for the run.
- `-lg:*` flags either toggle runtime modes or set tuning constants; some are runtime-mutable but most are startup-only.
- Flag values are **per-node** for `-ll:*` (each node sees the same hardware allocation; the runtime tunes itself per-node).
- Compile flags (`CC_FLAGS=...`) require a rebuild; runtime flags do not.
- Most flags compose safely; the exceptions are documented per-flag on their concept pages.

## Performance implications
- **Memory sizing** (`-ll:csize`, `-ll:fsize`, `-ll:zsize`, `-ll:rsize`): undersizing forces eviction / re-allocation churn (see `instance-fragmentation.md`). Oversizing wastes RAM but is harmless to perf. Always size to the working set.
- **Scheduling depth** (`-lg:window`, `-lg:sched`, `-lg:width`): larger windows expose more parallelism but increase memory + runtime overhead. Bump when `runtime-overhead-dominates.md` is *not* the bottleneck and you have spare runtime headroom.
- **Utility-processor count** (`-ll:util`): the throughput cap on runtime work. If Legion Prof shows util rows saturated, increase. If util rows are idle, leave at 1.
- **`-lg:prof`**: itself adds ~1-3% overhead in capture; always profile a *release* build (`DEBUG=0`).
- **`-lg:inorder`**, **`-lg:partcheck`**, **`-DPRIVILEGE_CHECKS`**, **`-DBOUNDS_CHECKS`**: debug-only; each adds 10-50%+ overhead. Remove for performance runs.

## Debug signals
- **Wrong hardware allocation** (e.g., only one CPU shown in Legion Prof when 16 expected): re-check `-ll:cpu`. Output of `-level realm=2` at startup confirms the allocation.
- **Distributed run hangs immediately** with no error: typical sign that `-ll:rsize` is 0 — multi-node Realm needs registered-memory pools. Set `-ll:rsize 1024+`.
- **Runtime warns about empty `-lg:prof_logfile`**: the `%` substitution was missing — files all overwrite each other. Use `prof_%.gz`.
- **`-level cat=N`** enables fine-grained log channels (see `logger-categories.md`); inspect `-level mapper=2` (mapper decisions), `-level legion_spy=2` (Spy events), `-level realm=2` (Realm startup + DMA).

## Source pointers
- **Reference (profiling flags)**: `raw/website-pages/profiling.md`
- **Reference (debug flags)**: `raw/website-pages/debugging.md`
- **Build / install reference**: `raw/website-pages/getting_started.md`
- **Realm runtime entry point** (parses `-ll:*` flags): https://github.com/StanfordLegion/legion/blob/master/runtime/realm/runtime.cc
- **Legion runtime entry point** (parses `-lg:*` flags): https://github.com/StanfordLegion/legion/blob/master/runtime/legion/runtime.cc

## Related
- `wiki/concepts/realm-machine-model.md` — what `-ll:*` flags configure.
- `wiki/concepts/processor-kinds.md` / `wiki/concepts/memory-kinds.md` — the taxonomies the sizing flags target.
- `wiki/concepts/operation-pipeline.md` — what `-lg:*` flags tune.
- `wiki/workflows/profile-an-app.md` — flag combinations for profiling.
- `wiki/workflows/debug-perf-bottleneck.md` — flag combinations for debugging.

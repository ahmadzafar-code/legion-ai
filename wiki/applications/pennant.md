---
title: Pennant
slug: pennant
summary: LANL's 2D unstructured-mesh Lagrangian hydrodynamics mini-app, ported to Regent; the canonical control-replication + dependent-partitioning benchmark, and the most-cited Legion app in retreat debug demos.
tags: [data-model, partitioning, replication, tracing, parallelism, for-program-reasoning, for-perf-debug]
subsystem: cross
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/publications/publications.md
  - raw/youtube_transcripts/retreat_2024/transcripts/017_Legion_Retreat_2024_-_Debugging_Legion_Applications_-_Michael_Bauer.txt
  - raw/resources/resources.md
  - raw/website-pages/community.md
github:
  - https://github.com/StanfordLegion/legion/tree/master/language/examples
related:
  - wiki/concepts/dependent-partitioning.md
  - wiki/concepts/partition-by-image.md
  - wiki/concepts/partition-by-preimage.md
  - wiki/concepts/ghost-region.md
  - wiki/concepts/control-replication.md
  - wiki/concepts/dynamic-tracing.md
  - wiki/concepts/regent-language.md
  - wiki/concepts/regent-demand-directive.md
  - wiki/concepts/index-space-launch.md
  - wiki/concepts/projection-functor.md
  - wiki/concepts/sharding-functor.md
  - wiki/applications/circuit.md
  - wiki/applications/miniaero.md
---

## TL;DR
Pennant is a 2D unstructured-mesh Lagrangian hydrodynamics mini-app from LANL (originally by Charles Ferenbaugh, LA-CC-12-021) ported to Regent. It models the same physics — predictor-corrector hydro with QCS artificial viscosity — as LANL's production rad-hydro code FLAG. It is **the** canonical Regent + control-replication + dependent-partitioning benchmark: weak-scales to 1024 nodes at ~87% efficiency in the cr2017 paper, gives a 96% LOC reduction from explicit colorings in dpl2016, and is Michael Bauer's go-to live demo for Legion Prof debugging (retreat 2024). The confusion: Pennant *underperforms* on tracing speedup compared to Circuit/MiniAero (2.8× vs 4×+) because its main loop is gated by a `calc_dt_hydro` global reduction — the runtime can't run ahead until the convergence value is back.

## What it computes
2D unstructured-mesh Lagrangian hydrodynamics with three built-in test problems:
- **Sedov** — point-blast shock expanding into a quiescent medium.
- **Noh** — strong compression / radial inflow.
- **Leblanc** — 1D shock tube extended to 2D.

The main loop is predictor-corrector:
```
adv_pos_half → calc_centers/volumes → calc_state_at_half
  → calc_force_pgas_tts → qcs_zone_center_velocity
  → qcs_corner_divergence → qcs_force → calc_work
  → adv_pos_full → calc_work_rate_energy_rho_full
  → calc_dt_hydro                                  ← blocking convergence reduction
```
Test configs live in `language/examples/pennant.tests/` (`sedov`, `sedovsmall`, `sedovbig` from `1x30` through `16x30`, `leblanc*`, `noh*`).

## Region & partition structure
Three logical regions reflect the dual-mesh data model used by all Lagrangian hydro codes:
- **`rp` (points)** — vertex positions, velocities, forces, masses.
- **`rz` (zones)** — cell-centered material state: density, pressure, energy, area.
- **`rs` (sides)** — the connectivity glue: each side connects one point to one zone (one side per corner of each zone). This is the canonical "side region" data structure.

The Regent variants in `language/examples/`:
- **`pennant.rg`** — reference; uses pre-computed C++-built `legion_coloring_t` handles.
- **`pennant_fast.rg`** — vectorized with `__demand(__vectorize)`; private/ghost split via `s_span.internal`; uses `cross_product` and `equal` for span strip-mining.
- **`pennant_dp.rg`** — "data parallel" variant. **Important caveat**: despite the name, the upstream master version does **not** use `image()`/`preimage()` dependent-partitioning operators — it still uses plain `partition(disjoint|aliased, ..., coloring)` where the colorings come from a C++ `cpennant.generate_mesh_raw(...)` call. The image/preimage formulation lives **in the dpl2016 paper only** and demonstrates what Pennant *could* look like; the upstream code path uses pre-computed colorings, presumably for performance or for not having upstreamed the paper variant.
- **`pennant_sequential.rg`** — no partitioning baseline.
- **`pennant_stripmine.rg`** — strip-mining variant.
- **`pennant_common.rg`** — shared `fspace zone`/`fspace point`/`fspace side` types, Terra-based `config` struct, and the `compute_coloring(...)` Terra entrypoint that builds `legion_coloring_t`.
- **`pennant.cc/.h`** — C++ mesh generator (`generate_mesh_rect/pie/hex`, `compact_mesh`, `color_spans`, `sort_zones_by_color`) plus `class PennantMapper` (six DefaultMapper overrides).

## Main loop
Each iteration runs an index launch over the `npieces` zone partition for every kernel above. Privileges flow through the `rz`/`rp`/`rs` regions:
- Zone tasks (`calc_centers`, `calc_volumes`, `calc_state_at_half`) consume zone partition + the side partition that references each zone.
- Point tasks (`adv_pos_*`) consume the point partition + the side partition that touches each point.
- The QCS artificial-viscosity kernels span side+zone+point — the heaviest cross-region pattern in the app.
- **`calc_dt_hydro`** is a blocking global reduction returning a Future that gates the next iteration; this is the `Future`-driven convergence test Bauer points to as Pennant's primary scaling bottleneck.

## Legion features exercised
- **`dependent-partitioning.md`** — Pennant is the paper-level textbook example (dpl2016 §4): given a disjoint zone partition, derive the side partition as `preimage(zone-of-side)` and the point partition as `image(side-to-point)`, achieving a **96% LOC reduction** (163 → 6) and matching/beating hand-rolled colorings at scale. **Code-visibility caveat**: the upstream master `pennant_dp.rg` does *not* contain these operators; it uses pre-computed `legion_coloring_t` handles built in C++. The image/preimage formulation is a paper-only artifact in current Legion.
- **`partition-by-image.md`** + **`partition-by-preimage.md`** — sides → points (image), zones → sides (preimage) per the dpl2016 paper formulation.
- **`ghost-region.md`** — `pennant_fast.rg`'s `s_span.internal` distinguishes private-point fast path from ghost-point exchange.
- **`control-replication.md`** — Pennant scales because each shard runs the same Regent top-level over its piece; cr2017 §5.3 weak-scales to 1024 nodes.
- **`sharding-functor.md`** + **`replicable-task.md`** — the shard-affinity glue.
- **`dynamic-tracing.md`** — the iteration body is the canonical trace block; the convergence-test stall is why the speedup is "only" 2.8× (trace2018).
- **`regent-language.md`** + **`regent-demand-directive.md`** — `__demand(__vectorize)` in `pennant_fast.rg`, `__demand(__inline)` for helpers, optional `__demand(__cuda)` on flux kernels.
- **`future.md`** — `calc_dt_hydro` returns a Future; this Future drives the convergence loop and is the runtime-block source.

## Performance characteristics
**Scaling (cr2017 §5.3, Piz Daint, 7.4M zones/node weak-scaling):**
- Regent + control replication: ~87% parallel efficiency at 1024 nodes.
- MPI reference: ~82%.
- MPI+OpenMP: ~64% (suffers from thread imbalance at scale).
- Regent without CR: collapses past ~64 nodes.

**Single-node:** Regent is *slower* than reference MPI (the runtime needs a CPU core); CR closes the gap at scale by amortizing the `dt` reduction across many shards.

**Tracing (trace2018):**
- Pennant gets **2.8×** with dynamic tracing.
- Circuit, Stencil, MiniAero, Soleil-X get **4.2×+**.
- The paper explicitly attributes the gap to the convergence predicate: "the main loop in PENNANT is guarded by a convergence predicate that in turn prevents a replay of the trace until the condition is resolved. A trace replay overlaps with tasks only for 25% or less of the time per iteration."

**Partitioning startup (dpl2016):**
- DPL image/preimage: **76-100% single-thread speedup** vs. explicit colorings.
- **29× speedup at 64 nodes** vs. naive coloring for the partitioning phase.

## Debug signals
Sourced from Michael Bauer's retreat 2024 debug demo (`raw/youtube_transcripts/retreat_2024/transcripts/017_..._Debugging_Legion_Applications_-_Michael_Bauer.txt`):

1. **GPU gaps despite a registered TOC variant** → check mapper-call latency in the util processor rows. Bauer observed `select_task_options` / `slice_task` / `map_task` taking 7-53 ms per call, suspected CUDA-driver thread interference. Cross-link `pitfalls/mapper-stalls.md`.
2. **High channel-row activity** between `SYSTEM_MEM` and `GPU_FB_MEM` → mapper placed instances in system memory; Bauer found PCIe copies of 80-174 ms per transfer (multi-hop, often 2 system-↔-FB copies for one logical transfer). Cross-link `pitfalls/excessive-data-movement.md` and `select-instance.md`.
3. **Deferred-copy bars going red** (`deferred` time sub-millisecond, target 10s-100s of ms) → runtime has caught up to execution; Legion isn't running ahead. In Pennant this is *endemic* because `calc_dt_hydro` returns a Future that blocks the next iteration.
4. **Tracing speedup well below 4×** → expected for Pennant due to convergence-predicate blocking; not a bug. Compare against Circuit/Stencil to confirm.
5. **Slow startup partitioning** → switch to `pennant_dp.rg` style image/preimage (dpl2016 result: 29× at 64 nodes vs. explicit coloring).
6. **Bad scaling past 32-64 nodes** → verify `-fcontrol-replication 1` was set during Regent compile; cr2017 shows Regent-without-CR collapses at that point.

## Code excerpts

**Dual-mesh field spaces** (`pennant_common.rg`) — the type-system encoding of the side-connects-point-to-zone topology:
```regent
fspace zone {
  zxp : vec2, zx : vec2, zareap : double, zarea : double,
  zvolp : double, zvol : double, zm : double, zrp : double, zr : double,
  ze : double, zetot : double, zw : double, zwrate : double,
  zp : double, zss : double, zuc : vec2, znump : uint8,
  -- ... ~20 fields per zone
}
fspace point {
  px0 : vec2, pxp : vec2, px : vec2,
  pu0 : vec2, pu : vec2, pap : vec2, pf : vec2,
  pmaswt : double, has_bcx : bool, has_bcy : bool,
}
fspace side(rz : region(zone), rpp : region(point),
            rpg : region(point), rs : region(side(rz, rpp, rpg, rs))) {
  mapsz  : ptr(zone, rz),
  mapsp1 : ptr(point, rpp, rpg),
  mapsp2 : ptr(point, rpp, rpg),
  mapss3 : ptr(side(rz, rpp, rpg, rs), rs),
  mapss4 : ptr(side(rz, rpp, rpg, rs), rs),
  sareap : double, sarea : double, svolp : double, svol : double,
  -- ... ~20 more side fields
}
```
The `side` fspace is **parameterized over four regions** — owned zones (`rz`), private points (`rpp`), ghost points (`rpg`), and a recursive reference to its own region (`rs`). Side pointers (`mapsp1`, `mapsp2`) span `rpp` ∪ `rpg`, which is how Regent enforces "this point may be ghost" statically.

**Pre-computed colorings, not DPL** (`pennant_dp.rg`, the "data parallel" variant) — the partition setup as it actually exists in master:
```regent
-- Partition zones into disjoint pieces.
var rz_all_p = partition(disjoint, rz_all, colorings.rz_all_c)

-- Partition points into private and ghost regions.
var rp_all_p = partition(disjoint, rp_all, colorings.rp_all_c)
var rp_all_private = rp_all_p[0]
var rp_all_ghost = rp_all_p[1]

-- Partition private points into disjoint pieces by zone.
var rp_all_private_p = partition(disjoint, rp_all_private, colorings.rp_all_private_c)

-- Partition ghost points into aliased pieces by zone.
var rp_all_ghost_p = partition(aliased, rp_all_ghost, colorings.rp_all_ghost_c)

-- Partition sides into disjoint pieces by zone.
var rs_all_p = partition(disjoint, rs_all, colorings.rs_all_c)
```
The `colorings.*` handles come from a C++ `cpennant.generate_mesh_raw(...)` call. **No `image()`, `preimage()`, `cross_product`, `by_field`, or `by_restriction` operators appear in any Pennant `.rg` file in current master.** The dpl2016 image/preimage formulation is paper-only.

**The traced predictor-corrector loop** (`pennant.rg`):
```regent
__demand(__trace)
do
  __demand(__index_launch)
  for i in pieces do init_step_points(rp_all_private_p[i], enable) end
  __demand(__index_launch)
  for i in pieces do init_step_zones(rz_all_p[i], enable) end

  dt = calc_global_dt(dt, dtfac, dtinit, dtmax, dthydro, time, tstop, cycle)

  __demand(__index_launch)
  for i in pieces do adv_pos_half(rp_all_private_p[i], dt, enable) end
  __demand(__index_launch)
  for i in pieces do adv_pos_half(rp_all_shared_p[i], dt, enable) end
  __demand(__index_launch)
  for i in pieces do
    calc_centers(rz_all_p[i], rp_all_private_p[i],
                 rp_all_ghost_p[i], rs_all_p[i], enable)
  end
  -- ... calc_volumes, calc_char_len, calc_rho_half, sum_point_mass,
  --     calc_state_at_half, calc_force_pgas_tts, qcs_*, adv_pos_full,
  --     calc_work, calc_work_rate_energy_rho_full, calc_dt_hydro ...
end
```
The entire timestep body is wrapped in **one** `__demand(__trace)` block; each phase is an `__demand(__index_launch)` over `pieces`. This is the trace block whose replay is bounded by the `calc_dt_hydro` Future at the next cycle's `calc_global_dt`.

**The Future-gating reduction** (`pennant.rg`) — the smoking gun for the 2.8× tracing limit:
```regent
dthydro = dtmax
__demand(__index_launch)
for i in pieces do
  dthydro min= calc_dt_hydro(rz_all_p[i], dt, dtmax, cfl, cflv, enable)
end
```
`dthydro` is a per-cycle Future built from a `min=` reduction across pieces. The next cycle's `calc_global_dt` consumes it, creating a cross-iteration Future dependency *inside* the traced block. Trace replay can launch cycle N+1's tasks, but the runtime can't actually retire them until cycle N's `dthydro` Future resolves — that's why the trace overlap caps at "~25% of the time per iteration" per trace2018.

**Manually-inlined `calc_dt_hydro`** (`pennant.rg`, with comment in source) — Pennant fuses two min-reductions to keep tracing happy:
```regent
-- Hack: manually inline calc_dt_courant + calc_dt_volume into one task
task calc_dt_hydro(rz : region(zone), dt : double, dtmax : double,
                   cfl : double, cflv : double, enable : bool) : double
  -- merged loop produces a single Future instead of two
  ...
```
Without the manual inline, the original two-Future `min` would defeat tracing.

**Toplevel with paired annotations** (`pennant.rg`):
```regent
__demand(__inner, __replicable)
task toplevel()
```
Both `__inner` (this task only spawns subtasks; no leaf work) and `__replicable` (eligible for control replication across shards). This is the canonical Regent annotation pair for scalable toplevel tasks.

**PennantMapper's load-bearing override** (`pennant.cc`) — uses the first region's color as the SPMD shard index:
```cpp
Processor PennantMapper::default_policy_select_initial_processor(
                                    MapperContext ctx, const Task &task)
{
  if (!task.regions.empty() &&
      task.regions[0].handle_type == SINGULAR) {
    Color index = runtime->get_logical_region_color(ctx, task.regions[0].region);
    std::vector<Processor> &local_procs =
      sysmem_local_procs[proc_sysmems[local_proc]];
    if (local_procs.size() > 1) {
      return local_procs[index % local_procs.size()];
    } else if (local_procs.size() > 0) {
      return local_procs[0];
    }
  }
  return DefaultMapper::default_policy_select_initial_processor(ctx, task);
}
```
Six DefaultMapper overrides total (`rank_processor_kinds`, `select_initial_processor`, `select_target_processors`, `select_instance_region`, `map_copy`, `map_must_epoch`), but no `slice_task` — per-piece locality comes entirely from keying `select_initial_processor` on `regions[0]`'s color.

## Source pointers
- **Regent sources**: https://github.com/StanfordLegion/legion/tree/master/language/examples
  - `pennant.rg` (~1554 lines), `pennant_fast.rg`, `pennant_common.rg`, `pennant_sequential.rg`, `pennant_stripmine.rg`, **`pennant_dp.rg`** (DPL showcase).
- **C++ glue**: `pennant.cc`, `pennant.h` — mesh generator + `class PennantMapper`.
- **Test configurations**: `pennant.tests/` — `sedov{,small,big}`, `leblanc{,4x30,big}`, `noh{,small,poly,square}`.
- **Python wrapper**: https://github.com/StanfordLegion/legion/tree/master/apps/pennant/python
- **Original LANL source**: https://github.com/lanl/PENNANT — Charles Ferenbaugh, LA-CC-12-021.

## Real reported issues

Pennant accumulates the most real bug reports of any Legion app — every new scaling regime (256 nodes, 64-node weak scaling, GPU gather copies, control-replicated wrappers) has exposed a distinct runtime issue. Verbatim issues filed against Pennant in `StanfordLegion/legion`:

- **[#1661 — Modified Pennant does not weak scale on Perlmutter](https://github.com/StanfordLegion/legion/issues/1661)** (rupanshusoi, 2024-03-24, **open**). "This code does not weak scale to even 64 nodes... The scaling problem seems to stem from the gap between the end of the first wrapper task and the start of the second. This gap increases with node count, going from 0.6 s on 4 nodes, to 1.5 s on 16, and to 8 s on 64. Furthermore, there is a stack of equivalence set tasks on the utility processors in this gap... It is thousands of tasks tall on 64 nodes." Bauer's diagnosis: "the implementation of virtual mapped regions for control replication is completely un-optimized... Optimizing it would take at least an entire month of engineering work." Wrapper tasks had hundreds-to-thousands of region requirements scaling with machine size. **Not fixed.**

- **[#1449 — CRC mismatch at 256 nodes in C++ Pennant](https://github.com/StanfordLegion/legion/issues/1449)** (elliottslaughter, 2023-03-31, **open**). "I'm seeing the following failure in C++ Pennant starting at 256 nodes: `CRC MISMATCH: arg0=1610612737 ... exp=300a8000 act=68e03e37`. It happens about 80% of the time." Root cause: GASNet medium active-message size limit; debug GASNet flagged `medium payload too large! src=6/0 tgt=11/0 max=4072 act=6224`. Treichler confirmed duplicate of realm#97. **Workaround**: `REALM_NETWORKS=gasnet1` + rebuild GASNet with `--with-aries-max-medium=8192`; O(N)-unsustainable at 512+ nodes. **No upstream fix.**

- **[#1802 — Realm Gather Copy Hang](https://github.com/StanfordLegion/legion/issues/1802)** (lightsighter, 2024-12-06, **closed**). "I see non-deterministic hangs when running Pennant C++ when using gather copies to initialize the mesh on GPUs. Logs with `-level dma=1` show that there are always more started copies than there are completed copies at the point that we hang... it was hanging ~80% of runs for me." **Workaround**: `-gex:objcount 8192`. **Fix**: Pryakhin MR 1574 — "We shake loose some of the timing by deferring the completion of a transfer descriptor and end up re-inserting the same xd to the front therefore leaving out those that actually need to be completed first."

- **[#1052 — Pennant `dcr_noidx` is 10× slower than `dcr_idx`](https://github.com/StanfordLegion/legion/issues/1052)** (elliottslaughter, 2021-04-11, **closed without fix**). At Piz Daint: "pennant: `dcr_noidx` is 10x slower than `dcr_idx` (probably an application or mapper issue)." Never investigated upstream.

- **[#1041 — LLVM 6 codegen regression also affects Pennant](https://github.com/StanfordLegion/legion/issues/1041)** (elliottslaughter, 2021-04-03, **closed**). Single-node: "Pennant: 2.4 ⇒ 3.2 s (33% increase in running time)." Same root cause as Circuit's #1041 — PTX rounding-mode emission suppressed FMA. Slaughter: "rounding modes are in fact responsible for the performance gap." **Fix**: `-ffast-math` (`contract` flag). Final: Pennant 2.328s vs 2.354s baseline.

- **[#1357 — Future leak in Pennant](https://github.com/StanfordLegion/legion/issues/1357)** (elliottslaughter, 2022-11-30, originally found by lightsighter, **closed**). **Fix**: MR 703, merged 2023-03-10. Slaughter confirmed `pennant.rg` runs with `Leaked Futures: 0` after fix.

- **[#1836 — `-lg:dump_physical_traces` broken for Pennant](https://github.com/StanfordLegion/legion/issues/1836)** (rohany, 2025-02-27, **closed**). Assertion `finder != memo_entries.end()` in `ReplayMapping::to_string` because Pennant has tasks with no region arguments not added to `memo_entries`. **Fix**: MR 1709, merged 2025-03-13.

- **[#770 — Tracing causes nondeterministic Pennant divergence](https://github.com/StanfordLegion/legion/issues/770)** (Steven Brill / CFD; Bauer added 2020-03-13, **closed**): "I have observed a similar behavior with Pennant, but in my case I only see it show up around 128 GPUs across 16 nodes." **Fix**: magnatelee commit `a923f1b6`, 2020-03-24.

- **[#234 — Pennant exercises a DMA bug](https://github.com/StanfordLegion/legion/issues/234)** (lightsighter, 2017-03-25, **closed**). Copy targeting an instance built from an empty index space; source/destination intersected non-emptily but the instance was empty — Realm did not check domain subset. **Fix**: commit `da5e088`.

- **[#1841 — "revive out of order commit" required Pennant workarounds](https://github.com/StanfordLegion/legion/issues/1841)** (elliottslaughter, 2025). Blocking on a future causes "spurious dependency on every intervening task." **Fix**: MR 1909, merged 2025-10-02.

- **[#446 — `__demand(__inline)` on `continue_simulation` produced wrong answer](https://github.com/StanfordLegion/legion/issues/446)** (manopapad, 2018-10-29, **closed**). "Assertion failed: sv negative." **Fix**: magnatelee commit `f7452239`.

- **[#1649 — Non-deterministic Pennant segfault on 2 Perlmutter nodes](https://github.com/StanfordLegion/legion/issues/1649)** (rupanshusoi, 2024-03-14, closed 2024-03-20). **[#1671 — Replicating onto half the machine assertion failure](https://github.com/StanfordLegion/legion/issues/1671)** (rupanshusoi, 2024-03-30). Both closed via runtime fixes.

### Bauer's 2024 retreat live debug demo (verbatim)

From `raw/youtube_transcripts/retreat_2024/transcripts/017_..._Debugging_Legion_Applications_-_Michael_Bauer.txt`, on profiles co-generated with Slaughter on Sapling:

> **Anomaly 1 — slow mapper calls (lines 251, 322-341)**: "this application pennant actually suffers from sort of a blocking behavior where it has to synchronize to test a... convergence condition every iteration... some of the mapper calls are actually taking a very long time. So Legion actually profiles all your mapper calls and stuff... Running for about seven milliseconds inside of your mapper there... it's actually running for like 12 milliseconds here. It took like 53 milliseconds to handle this whole mapper call, which is very unusual. We're not entirely sure yet why that is happening."

> **Anomaly 2 — multi-hop PCIe copies (lines 378-418)**: "the problem here is actually with data movement... channel utilization is really high, right bordering on like, you know, like oftentimes 100% of channel utilization... node zero system zero to node zero frame buffer two... that's like 80 milliseconds for this particular case or, you know, 174 milliseconds in that case... they're copying over the PCI bus and it's really slow... there's like a bug in the mapper that is placing its instances in the wrong place."

> **Anomaly 3 — deferred-copy "red" bars (lines 446-468)**: "this deferred value should actually be fairly large. It should be like tens or hundreds of milliseconds. And when it gets like less than like, you know, a millisecond down to like, few hundreds of microseconds or less, you know, that's where the execution is catching up with Legion's dependence analysis and starting to cause bubbles in your pipeline and latency... So we sort of mark these as red."

No fix stated — this is a live diagnostic walkthrough, not a bug-fix narrative.

### trace2018 §VI — the structural 2.8× tracing limit (verbatim)

From `raw/publications/pdfs/trace2018.pdf`:

> "Dynamic tracing improves the speedup of applications by 4.2× or more, except for PENNANT, which is improved by 2.8×. Unlike the other programs, the main loop in PENNANT is guarded by a convergence predicate that in turn prevents a replay of the trace until the condition is resolved. A trace replay overlaps with tasks only for 25% or less of the time per iteration, which explains an improvement that is 4× off of the improvement in the runtime overhead."

Structural — not patched.

## Tool-coverage classification

Each Pennant issue above split across the standard Legion diagnostic stack:

**Legion Prof alone surfaces it** (visible in timeline / processor / channel / utility / deferred-time rows):
- **#1661** — Soi explicitly says "stack of equivalence set tasks on the utility processors... thousands of tasks tall on 64 nodes" — the utility-processor row in Prof is the smoking gun. The growing inter-wrapper gap (0.6 → 1.5 → 8 s) is read off the timeline directly.
- **Bauer 2024 retreat Anomaly 1** — mapper-call durations 7-53 ms are visible in Prof's utility-processor row (Legion profiles all mapper callbacks).
- **Bauer 2024 retreat Anomaly 2** — multi-hop PCIe copies, channel utilization ~100%, 80-174 ms per copy. All visible on Prof's channel rows (`SYSTEM_MEM ↔ GPU_FB_MEM`).
- **Bauer 2024 retreat Anomaly 3** — deferred-copy "red" bars. Prof's deferred-time overlay turns bars red when deferred < threshold (sub-ms vs target tens-to-hundreds of ms).
- **#1052** — `dcr_noidx` 10× slower than `dcr_idx`: directly visible as task-bar duration difference between the two variants. (Not investigated, but Prof is the right tool.)

**Legion Prof + Legion Spy together are needed** (Prof shows the perf symptom; Spy answers "is there a real causal edge?"):
- **trace2018 §VI Pennant 2.8× ceiling** — Prof shows the trace-replay bar ending before the next iteration starts, and Spy's event graph reveals the **real** Future-dependency on `dthydro min= calc_dt_hydro(...)` that prevents earlier launch. Prof says "gap"; Spy says "yes, the convergence Future is a real edge."
- **#1421** style (would apply here too) — any "serialization with no Spy-visible cause" is the Prof+Spy signature of a tracing-internal fence.

**Profiler can't tell** (need runtime logs, error text, source inspection, or external tools):
- **#1449** — CRC mismatch at 256 nodes. Hard failure with `CRC MISMATCH` text; Pennant doesn't get far enough to produce a useful Prof trace. Root cause via debug GASNet (`medium payload too large! src=6/0 tgt=11/0 max=4072 act=6224`) — outside Legion Prof.
- **#1802** — Realm gather copy hang. Hang means partial Prof trace; root cause via `-level dma=1` Realm logs ("there are always more started copies than there are completed copies").
- **#1041** — LLVM 6 CUDA codegen regression. Same as Circuit's #1041; needs PTX inspection.
- **#1357** — Future leak. Surfaces in Legion's leaked-futures count printed at exit; not a Prof signal.
- **#1836** — `-lg:dump_physical_traces` assertion. Crash before any Prof output.
- **#770** — tracing-induced nondeterministic divergence. Wrong numerical answer; not a perf bug, Prof can't see correctness.
- **#234** — DMA bug from empty-instance domain. Crash; Realm-internal.
- **#446** — `__demand(__inline)` on `continue_simulation` produced wrong answer. Correctness bug; needs `-DPRIVILEGE_CHECKS` / `-DBOUNDS_CHECKS` or value inspection.
- **#1841** — out-of-order commit's "spurious dependency on every intervening task" — Spy *can* see this (extra edges in the event graph), but the actual debugging required source-level reasoning about which Future was being blocked on.

## Papers that benchmark it
- **dpl2016 (OOPSLA)** — *Dependent Partitioning*; Pennant is one of three case studies. **96% LOC reduction**, 29× partitioning speedup at 64 nodes.
- **cr2017 (SC)** — *Control Replication*; §5.3 weak-scales to 1024 nodes at 87% efficiency, matches/beats MPI reference.
- **trace2018 (SC)** — *Dynamic Tracing*; Pennant gets 2.8× speedup, the lowest of five test apps; convergence-predicate explanation.
- **idx2021 (SC)** — *Index Launches*; index-launch optimization applies to Pennant's per-piece loops (Pennant itself not explicitly evaluated; Circuit and Stencil are).
- **dcr2021 (PPoPP)** — *Dynamic Control Replication*; doesn't benchmark Pennant directly but cites it as prior-art canonical CR application.
- **autotrace2025 (ASPLOS)** — *Apophenia*; Pennant is the manually-traced reference baseline that automatic tracing is calibrated against.

## Related
- `wiki/applications/circuit.md` — sibling Legion canonical app; graph RLC vs. unstructured hydro.
- `wiki/applications/miniaero.md` — sibling Legion canonical app; 3D unstructured CFD.
- `wiki/concepts/dependent-partitioning.md` — Pennant is the dpl2016 case study.
- `wiki/concepts/control-replication.md` — Pennant is the cr2017 headline benchmark.
- `wiki/concepts/dynamic-tracing.md` — Pennant exhibits the convergence-predicate tracing-limit.
- `wiki/concepts/future.md` — `calc_dt_hydro` Future is the iteration gate.
- `wiki/pitfalls/mapper-stalls.md` — Bauer's retreat-2024 Pennant demo shows it live.
- `wiki/pitfalls/excessive-data-movement.md` — Bauer's retreat-2024 multi-hop-copy anomaly.

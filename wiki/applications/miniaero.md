---
title: MiniAero
slug: miniaero
summary: Sandia's 3D unstructured-mesh finite-volume compressible Navier-Stokes proxy app, ported to Regent; canonical benchmark for hybrid SOA-AOS data layout and Regent's `__demand(__cuda)` GPU placement.
tags: [data-model, partitioning, gpu, parallelism, instances, for-program-reasoning, for-perf-debug]
subsystem: regent
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/publications/pdfs/slaughter_thesis.pdf
  - raw/publications/pdfs/regent2015.pdf
  - raw/publications/pdfs/cr2017.pdf
github:
  - https://github.com/StanfordLegion/legion/tree/master/language/examples
related:
  - wiki/concepts/regent-language.md
  - wiki/concepts/regent-demand-directive.md
  - wiki/concepts/ghost-region.md
  - wiki/concepts/disjoint-partition.md
  - wiki/concepts/reduce-privilege.md
  - wiki/concepts/instance-layout.md
  - wiki/concepts/cuda-interop.md
  - wiki/concepts/control-replication.md
  - wiki/applications/circuit.md
  - wiki/applications/pennant.md
---

## TL;DR
MiniAero is Sandia's Mantevo proxy CFD app — a 3D unstructured-mesh finite-volume compressible Navier-Stokes solver modeled on the production SIERRA/Aero code — ported to Regent. The Legion port lives only in Regent (`language/examples/miniaero.rg`); there is no C++ Legion variant. It is the canonical benchmark for showing that Regent's high-productivity port can match — and at scale, exceed — hand-tuned MPI+Kokkos. The Slaughter thesis (§8.3.3) reports the Regent version achieving *slightly over 100% parallel efficiency* at 1024 nodes vs. the MPI+Kokkos reference, with 30% fewer lines of code. The confusion: people expect image/preimage dependent partitioning here, but the actual `miniaero.rg` uses explicit colorings with hierarchical disjoint-cell + ghost-coloring + per-block face-categorization; the canonical-DPL story for *un*structured meshes lives in `pennant.md` instead.

## What it computes
3D explicit finite-volume compressible Navier-Stokes:
- Inviscid and viscous variants (toggle via `miniaero.inp`).
- Roe flux for inviscid convective terms; Newtonian flux for viscous terms.
- Green-Gauss cell-face gradients.
- Van Albada / Venkatakrishnan limiters (slope limiters for second-order accuracy).
- RK4 explicit time stepping.

Three built-in problems:
- **Sod** — 1D shock tube extended to 3D.
- **Viscous Flat Plate** — boundary-layer canonical test.
- **Inviscid Ramp** — supersonic flow over a wedge.

Input lives in `miniaero.inp` (problem type, cells per direction, dt, viscous flag, second-order flag). Reference Mantevo implementation is MPI+Kokkos (https://github.com/Mantevo/miniAero).

## Region & partition structure
Three primary region kinds (typical for unstructured 3D FV):
- **Cells** — cell-centered conservative state (`rho`, `rho*u/v/w`, `rho*E`) and derived quantities (`pressure`, gradients, residual `cell_flux`).
- **Faces** — face-centered flux state plus `left` and `right` cell pointers. Each face stores `left : ptr(Cell, rcell)` (same-block) and `right : ptr(Cell, rcell, rcell_ghost)` (possibly ghosted across block boundaries). **Faces are duplicated at submesh boundaries** so faces themselves never need to be exchanged — only ghost-cell state is exchanged.
- **Nodes** — geometric vertices.

**Partitioning** is hierarchical and explicit (not image/preimage):
1. Cells get a disjoint partition per block via `legion_coloring` C-API calls.
2. A second coloring creates the ghost-cell partition (`create_cell_ghosting`).
3. Faces get **per-block colorings categorizing them by boundary type**: `BC_INTERIOR`, `BC_TANGENT`, `BC_EXTRAPOLATE`, `BC_INFLOW`, `BC_NOSLIP`. Each boundary kind dispatches a different flux task.

## Main loop
RK4 explicit:
```
for stage in 1..4:
  compute_face_gradients         -- per-cell, reads ghost cells
  compute_min_max                 -- min/max reductions for limiters
  compute_limiter                 -- per-cell limiter values
  for face_kind in {interior, tangent, extrapolate, inflow, noslip}:
    roe_flux_compute_flux           (inviscid)
    newtonian_viscous_flux_compute_flux  (if viscous)
  compute_face_flux              -- accumulate `cell_flux` (reduction)
  rk_update                      -- advance cell state by stage weight
```
Hot kernels carry `__demand(__cuda)` and are JIT-compiled to GPU at startup; `__demand(__inline)` is used liberally on helpers to eliminate task-call overhead for ~10-line kernels.

## Legion features exercised
- **`regent-language.md`** + **`regent-demand-directive.md`** — `__demand(__cuda)` and `__demand(__inline)` are the principal mapping/perf knobs in `miniaero.rg`. The custom mapper (`MiniAeroMapper`) is minimal.
- **`ghost-region.md`** — ghost cells exchanged at submesh boundaries; faces *not* exchanged.
- **`disjoint-partition.md`** + **`coloring.md`** — hierarchical explicit colorings; cells partitioned disjointly per block.
- **`reduce-privilege.md`** + **`reduction-instance.md`** — `cell_flux += ...` accumulates contributions from all faces touching a cell; min/max reductions for limiter normalization.
- **`cuda-interop.md`** — `__demand(__cuda)` task variants drive GPU placement; reductions on shared cells use atomics.
- **`instance-layout.md`** — Slaughter §6.3 (regent2015) reports the **hybrid SOA-AOS layout** giving Regent a 2.8× speedup over MPI+Kokkos at 8 cores — locality of arithmetically-dense field tuples (the 5-component conservative state) wins over either pure SOA or pure AOS.
- **`control-replication.md`** — at 1024 nodes the Regent version slightly *exceeds* 100% parallel efficiency vs. MPI+Kokkos baseline (Slaughter thesis §8.3.3).

## Performance characteristics
**Scaling (Slaughter thesis §8.3.3, Figure 8.7):**
- Weak-scales to 1024 nodes.
- Regent version: slightly over 100% parallel efficiency vs. MPI+Kokkos baseline.
- Regent LOC: ~30% fewer than MPI+Kokkos reference (Slaughter reports 2836 Regent vs. 3993 reference C++).

**Single-node performance (Regent SC2015 §6.3):**
- Regent outperformed MPI+Kokkos by **2.8× at 8 cores** via the hybrid SOA-AOS layout.
- This is the canonical motivation for Legion's flexible instance layouts (see `instance-layout.md`).

**Profile:** Memory-bound. Hot kernels are flux and gradient loops — arithmetic intensity is low, locality matters far more than FLOPs. Fragmented `cell` instances or per-field SOA/AOS mismatches are the primary failure mode.

**Ghost-cell traffic:** scales with submesh surface area, not volume. Weak-scaling regressions almost always point to oversized ghost regions or missed control replication.

**Tracing (trace2018):** MiniAero gets **4.2×+** speedup from dynamic tracing — much better than Pennant's 2.8× because MiniAero has no per-iteration convergence-test stall (it's pure explicit RK4).

## Debug signals
- **GPU utilization low despite `__demand(__cuda)` annotation** → the CUDA variant didn't compile, or the mapper isn't selecting it. Check Regent compile logs for `__cuda` warnings and verify the mapper's `default_policy_select_target_processors` is returning a `TOC_PROC`.
- **Channel-row activity dominates timeline** → `cell` instances misaligned with consuming task placement; check `select-instance.md` mapper logic and per-field layout constraints.
- **Tracing speedup well below 4×** → unexpected for MiniAero (no convergence-test gate); investigate per-iteration overhead and runtime stalls.
- **Weak-scaling collapse past 64-128 nodes** → control replication not enabled (`-fcontrol-replication 1` at Regent compile time) or sharding-functor mismatch.
- **Memory-bound throughput regression after refactor** → SOA/AOS layout regression. Compare to the hybrid SOA-AOS layout described in regent2015 §6.3.

## Code excerpts

**Cell field space** (`miniaero.rg`) — AoS layout for conservative-variable tuples plus per-cell scratch (limiters, gradients, fluxes):
```regent
fspace Cell {
  solution_n     : Solution,    -- Solution = double[5]: rho, rho*u/v/w, rho*E
  solution_np1   : Solution,
  solution_temp  : Solution,
  residual       : Solution,
  cell_flux      : Solution,
  stencil_min    : Solution,
  stencil_max    : Solution,
  limiter        : Solution,
  cell_gradients : Gradient,    -- Gradient = double[15]: 3 per component
  cell_connectivity : CellConnect,
  cell_centroid  : Vec3,
  volume         : double,
}
```

**Face field space with multi-region pointers** (`miniaero.rg`) — the canonical Legion unstructured-mesh idiom:
```regent
fspace Face(rcell : region(Cell),
            rcell_ghost : region(Cell)) {
  left  : ptr(Cell, rcell),
  right : ptr(Cell, rcell, rcell_ghost),
  face_connectivity : FaceConnect,
  face_centroid : Vec3,
  area     : Vec3,
  tangent  : Vec3,
  binormal : Vec3,
  is_reversed : uint8,
}
```
This is the load-bearing pattern: `left` is constrained to the owned cell region, while `right` is a **union pointer spanning both owned and ghost regions**. The type system enforces "the face's right neighbor may be off-block" statically — no runtime check, no dispatch, just type-level disjoint-union semantics. There is no `fspace Node` — node coordinates are computed on the fly via `mesh_node_coordinate()` / `mesh_node_id()` helpers, saving memory.

**RK4 time integration loop** (`miniaero.rg`):
```regent
for ts = 0, interface.time_steps do
  for rk_stage = 0, 4 do
    update_rk_stage_alpha_and_initialize_solution_fields(
        rcell, rk_stage, rk4.alpha_, (rk_stage == 0))
    face_gradient(rcell, rface, face_category)
    compute_min_max(rcell, rface, face_category)
    compute_limiter(rcell, rface, face_category)
    compute_face_flux(false, false, rcell, rface, face_category)
    -- ... apply fluxes to residuals, accumulate to solution_temp ...
  end
end
```
Four sequential RK substages per timestep; each substage is gradient → min/max → limiter → flux. The hot kernels (`face_gradient`, `compute_face_flux`, etc.) carry `__demand(__cuda)` directives elsewhere in the file, JIT-compiled to GPU at startup.

**Ghost coloring built in Terra**, not Regent (`miniaero.rg`):
```regent
terra create_cell_ghosting(mesh : &MeshTopology,
                           ghost_cell_coloring : coloring_t)
  for i = 0, mesh.num_blocks_ do
    create_cell_ghosting_for_block(mesh, mesh_get_block_by_id(mesh, i),
                                   ghost_cell_coloring)
  end
  for iblk = 0, mesh.num_blocks_ do
    coloring_ensure_color(ghost_cell_coloring, iblk)
  end
end
```
Like Pennant, MiniAero builds `legion_coloring_t` in Terra (raw C-level), then hands it to `partition()` in Regent. No DPL operators.

**The minimal custom mapper** (`miniaero.cc`):
```cpp
class MiniAeroMapper : public DefaultMapper {
public:
  MiniAeroMapper(MapperRuntime *rt, Machine machine, Processor local,
                 const char *mapper_name);

  void default_policy_select_target_processors(
      MapperContext ctx, const Task &task,
      std::vector<Processor> &target_procs) override;

  LogicalRegion default_policy_select_instance_region(
      MapperContext ctx, Memory target_memory,
      const RegionRequirement &req,
      const LayoutConstraintSet &constraints,
      bool force_new_instances, bool meets_constraints) override;
};

void MiniAeroMapper::default_policy_select_target_processors(
    MapperContext ctx, const Task &task, std::vector<Processor> &target_procs) {
  target_procs.push_back(task.target_proc);
}

LogicalRegion MiniAeroMapper::default_policy_select_instance_region(
    MapperContext ctx, Memory target_memory, const RegionRequirement &req,
    const LayoutConstraintSet &layout_constraints,
    bool force_new_instances, bool meets_constraints) {
  return req.region;
}
```
**Both overrides are essentially no-ops that restrict DefaultMapper.** `select_target_processors` pins each task to exactly the proc the default mapper picked (disables proc-group fanout). `select_instance_region` returns the requested region verbatim (no parent-region instance sharing). All real placement decisions are made by Regent's `__demand(__cuda)` directives and the default mapper.

**The nearly-empty C++ header** (`miniaero.h`):
```c
extern "C" {
  void register_mappers();
}
```
That's the entire content (modulo include guards). No task ID enums, no accessor typedefs, no field/region IDs — a Regent-based Legion app doesn't need the C++ task-ID scaffolding a hand-written C++ Legion app requires, because Regent generates task registration from the `.rg` source.

## Source pointers
- **Regent main**: https://github.com/StanfordLegion/legion/tree/master/language/examples
  - `miniaero.rg` — primary Regent source.
  - `miniaero.cc`, `miniaero.h` — `class MiniAeroMapper : public DefaultMapper`; minimal overrides (`default_policy_select_target_processors`, `default_policy_select_instance_region`).
- **Sequential baseline**: `miniaero_sequential.rg`, `miniaero_sequential.cc`, `miniaero_sequential.h`.
- **Original Mantevo MPI+Kokkos reference**: https://github.com/Mantevo/miniAero — `kokkos/` for Kokkos build, `Makefile.mpi` for MPI. The Kokkos build supports CUDA via `KOKKOS_ARCH=Power8,Pascal60`.
- **Input format**: `miniaero.inp` — problem type (sod / viscous-flat-plate / inviscid-ramp), cells per direction, dt, viscous flag, second-order flag.

## Real reported issues

MiniAero has fewer perf-flavored issues than Pennant — most filed bugs are correctness (assertion failures, uninitialized reads) rather than scaling regressions. The performance characteristics come largely from the papers, not from filed perf issues. Verbatim reports:

- **[#79 — Locally mapped but remote execution bug for non-leaf task](https://github.com/StanfordLegion/legion/issues/79)** (magnatelee, 2015-10-21, **closed 2015-11-03**). "Running the C++ miniaero (non-spmd version) with a locally mapping mapper gives me the following assertion failure: `complete_points <= total_points`." Bauer's diagnosis: "when we locally map a task and then send it remotely, we're not properly sending back the 'mapped' message from the remote node to the owner node when the task is not a leaf task." **Fix**: commit `538b3d3` (and `0251ee0`/`5e1d50f`).

- **[#80 — Invalid remove version state in release builds](https://github.com/StanfordLegion/legion/issues/80)** (magnatelee, 2015-10-21, **closed**). Regent miniaero release binary on two nodes hit assertion in `DistributedCollectable::update_state` at `garbage_collection.cc:665`. Reproducible only in `DEBUG=0`. **Fix**: Bauer pushed to master Nov 2015; Lee confirmed gone 2015-11-04.

- **[#95 — NO_REGION points in index space launch](https://github.com/StanfordLegion/legion/issues/95)** (hnkolla, 2015-11-05, **closed**). Index task gets instantiated for every point even when projection resolves to NO_REGION for most points; reproduced with both MiniAero and S3D. Overhead concern. **Fix**: partial overhead-reduction patch by Bauer in 2015; Treichler later (2017-10-13) noted sparse-index-space launches now handle the case properly.

- **[#294 — Deppart merge blockers: NaNs in MiniAero, multi-node slowdown](https://github.com/StanfordLegion/legion/issues/294)** (lightsighter, 2017-09-07, **closed 2017-09-25**). Bauer's checklist for the dependent-partitioning rewrite merge explicitly tracked two MiniAero blockers: "NaNs in miniaero (both)" and the accompanying MiniAero perf regression checks against the perf chart at `stanfordlegion.github.io/perf-frontend`. Both checked off by merge time.

- **[#295 — Crash receiving profiling responses](https://github.com/StanfordLegion/legion/issues/295)** (magnatelee, 2017-09-11, **closed**). Running `regent.py miniaero/rdir_1ghost.rg -lg:prof 1` crashes with assertion `prev >= cnt` in `legion_profiling.cc:1185` because Realm sends multiple profiling responses for a multi-pair copy. **Workaround**: commit `d344855` disabling DMA grouping. Permanent fix later merged (Lee confirmed 2018-02-13).

- **[#356 — Bug in Regent partitioning routines blocked MiniAero](https://github.com/StanfordLegion/legion/issues/356)** (lightsighter, 2018-02, **closed**). Legion-Spy safety check fired because the shallow-partition code (revived in `99837ec`) produced subregions not dominated by their parent. Slaughter: "@magnatelee is unable to run MiniAero because of this." **Fix**: pending-partition complete cross product (`f4249ff`) — the broken shallow path was no longer needed.

- **[#450 — MiniAero read of uninitialized data](https://github.com/StanfordLegion/legion/issues/450)** (lightsighter, 2018-11-07, **closed 2018-12-21**). `compute_limiter_task` reads field 211 with read-only privileges before initialization (LEGION ERROR 68). Test disabled in CI for ~6 weeks. **Fix**: Lee pushed a fix to the MiniAero repo; re-enabled in commit `72cf6cadc4`.

- **[#1611 — Modernize MiniAero test](https://github.com/StanfordLegion/legion/issues/1611)** (elliottslaughter, 2023-12-09, **closed 2024-01-31**). The MiniAero CI test used old Legion accessor interfaces and failed under C++14. **Fix**: Slaughter ported MiniAero to Regent (commits `d1f9bc2839`, `a14954dca6`), then turned off the old C++ test (`6158f72e3f`).

- **[Mantevo/miniAero #2 — Only 20% of Roofline](https://github.com/Mantevo/miniAero/issues/2)** (Johannes / @elReynerino, FAU Erlangen-Nuernberg student, 2017-08-28, **open**, **no Sandia response**). On Tesla K40 + 2× Xeon E5-2650v2, the Kokkos build reaches only ~20% of roofline on the ramp test across all cell counts. Filed against upstream Mantevo, not Legion. Demonstrates that the memory-bound profile is intrinsic to MiniAero's flux-loop algorithm, not a Legion-specific artifact.

### Paper-stated perf characteristics

- **`regent2015.pdf` §6.3**: "Regent outperforms MPI+Kokkos on 8 cores by a factor of 2.8X through the use of a hybrid SOA-AOS data layout... The improved data layout substantially boosts cache reuse and improves utilization of memory bandwidth." Also: "MiniAero is sensitive to the combination of index launches and mapping elision. When both optimizations are disabled, the code runs serially."
- **`slaughter_thesis.pdf` §8.1.3**: "MiniAero is mostly memory-bound, and thus is sensitive to optimizations that improve locality... locality, and thus performance, benefits substantially from using a hybrid data layout, where some fields are stored in SOA layout and others are stored in AOS layout. The versions of Legion used in the experiments did not support this kind of hybrid layout, and thus the initial Regent implementation of MiniAero uses arrays to achieve the same effect."
- **`slaughter_thesis.pdf` §8.3.3 + Fig 8.7**: Regent *without* control replication "struggles to scale beyond a modest number of nodes" — throughput per node collapses from ~1.3M cells/s at 1-8 nodes to ~250K at 64 nodes. With CR: "slightly over 100% parallel efficiency at 1024 nodes." MPI+Kokkos baseline ~400-500K cells/s/node throughout.
- **`slaughter_thesis.pdf` §8.3.6 / Table 8.1**: MiniAero's dynamic region intersections are the slowest of any benchmark: shallow 259 ms at 1024 nodes, complete 43 ms (vs Stencil 78 ms / 1.3 ms).
- **`trace2018.pdf` §VI / Table II**: MiniAero achieves max 5.1× speedup with dynamic tracing at 256 nodes; without tracing only 5.0× relative to single-node, with tracing reaches 121.4×. Tracing reduces per-task overhead from 940 μs to 183 μs (5× amortization). Lee et al.: "For the two biggest programs, MiniAero and Soleil-X, Legion's dynamic task scheduling plays a crucial role in overlapping communication with computation as their tasks have parallelism due to field non-interference."

### Honest gaps

- No GitHub PRs returned by the dedicated PR search — fixes for MiniAero issues came as direct commits referenced inside issue threads.
- No mailing-list, retreat-talk, or bootcamp-talk specifically discussing MiniAero perf was located.
- Mantevo upstream has only 4 total issues; only #2 is perf-flavored and remains unanswered. The Sandia repo is effectively unmaintained.

## Tool-coverage classification

Each MiniAero issue above split across the standard Legion diagnostic stack. MiniAero's profile is unusual: most filed bugs are correctness (assertions, uninitialized reads) rather than perf regressions, so Prof's coverage of *real* issues is narrower than for Pennant.

**Legion Prof alone surfaces it** (timeline / processor / channel / utility rows; comparison to baseline):
- **#294 (multi-node slowdown part)** — Prof regression against the saved baseline at `stanfordlegion.github.io/perf-frontend` is what flagged it as a release blocker.
- **Paper §6.3 (regent2015) — 2.8× single-node speedup via hybrid SOA-AOS** — Prof shows shorter task bars in the kernels after layout change.
- **Paper §8.3.3 (Slaughter) — throughput collapse without CR** — Prof shows per-node throughput regression with node count; cells-per-second is computed from task durations.
- **Paper §8.3.6 / Table 8.1 — slow dynamic region intersections (259 ms at 1024 nodes)** — Prof shows the partition-op meta-task durations directly.
- **trace2018 Table II — 5.1× tracing speedup at 256 nodes** — Prof comparison with/without tracing; task granularity reduction 940 μs → 183 μs visible per task.

**Legion Prof + Legion Spy together are needed**:
- **#95** — NO_REGION points in index space launch. Prof would show extra task bars (or zero-duration bars); Spy's **dataflow graph** confirms those tasks don't touch any region. Together they prove the overhead is from over-instantiation, not from real work.
- **#294 (NaN part)** — Prof shows tasks running normally; Spy's event graph + value inspection narrows where NaN enters. Spy alone identifies missing producers; Prof confirms timing of when NaN-producing task ran.

**Legion Spy alone surfaces it** (runtime safety check, not a timing observation):
- **#356** — "subregions not dominated by their parent." This is Spy's runtime dominance check firing; Prof has no view of region-tree invariants.
- **#450** — read of uninitialized data (LEGION ERROR 68). The privilege/visibility analysis fires before the task runs; Spy's dataflow graph would show a read with no upstream writer. Prof never sees the task complete normally.

**Profiler can't tell** (need runtime assertion text, error catalog, build tooling, or external tools):
- **#79** — assertion `complete_points <= total_points` in the runtime. Crash; pre-Prof.
- **#80** — assertion in `DistributedCollectable::update_state`. Crash; release-build-only.
- **#295** — crash *in the profiler itself* (`prev >= cnt` in `legion_profiling.cc:1185`). Trying to use `-lg:prof 1` is what causes the crash. Ironic but real.
- **#1611** — build/CI failure under C++14. Compile-time issue.
- **Mantevo/miniAero #2 — "only 20% of roofline"** — this is a *roofline* measurement, which requires bandwidth + FLOPS counters from NVIDIA Nsight Compute (or equivalent). Legion Prof shows task durations but not arithmetic intensity vs memory bandwidth — out-of-scope for Legion's tooling. The result is consistent with Slaughter §8.1.3's "mostly memory-bound" characterization; the diagnosis came from an external roofline plot.

## Papers that benchmark it
- **Slaughter thesis (2017)** — `raw/publications/pdfs/slaughter_thesis.pdf` §8.1.3 (description), §8.3.3 (weak-scaling evaluation), Figure 8.7. The headline 1024-node result.
- **regent2015 (SC)** — `raw/publications/pdfs/regent2015.pdf` §6.3. Single-node hybrid SOA-AOS speedup (2.8× over MPI+Kokkos at 8 cores).
- **cr2017 (SC)** — `raw/publications/pdfs/cr2017.pdf` §5.2. MiniAero is one of the headline control-replication scaling benchmarks.
- **trace2018 (SC)** — *Dynamic Tracing*; MiniAero is one of five evaluated programs and gets 4.2×+ speedup.
- *Not* in Bauer thesis (2014, S3D-focused) and *not* in `publications.md` as a numbered citation — the canon is Slaughter + Regent SC2015 + CR SC2017. Gap flagged in `wiki/meta/source-coverage.md`.

## Related
- `wiki/applications/circuit.md` — sibling canonical app; graph RLC vs. 3D unstructured FV.
- `wiki/applications/pennant.md` — sibling canonical app; 2D unstructured hydro. Pennant is the dpl2016 image/preimage showcase; MiniAero uses explicit colorings instead.
- `wiki/concepts/regent-demand-directive.md` — `__demand(__cuda)` is MiniAero's principal GPU-placement knob.
- `wiki/concepts/instance-layout.md` — the hybrid SOA-AOS layout that gives MiniAero its single-node speedup.
- `wiki/concepts/cuda-interop.md` — the GPU substrate.
- `wiki/concepts/ghost-region.md` — the cell-ghost exchange pattern; faces are duplicated, cells are exchanged.

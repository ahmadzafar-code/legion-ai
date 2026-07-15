---
title: Circuit
slug: circuit
summary: Legion's canonical graph-application example — explicit RLC simulation on a partitioned circuit graph with private/shared/ghost nodes, reduction privileges, custom mapper, and GPU variants.
tags: [data-model, parallelism, gpu, mapping, instances, for-program-reasoning, for-perf-debug]
subsystem: cross
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/11_circuit_simulation.md
  - raw/website-pages/getting_started.md
github:
  - https://github.com/StanfordLegion/legion/tree/master/examples/circuit
  - https://github.com/StanfordLegion/legion/tree/master/language/examples
related:
  - wiki/concepts/logical-region.md
  - wiki/concepts/partition.md
  - wiki/concepts/disjoint-partition.md
  - wiki/concepts/ghost-region.md
  - wiki/concepts/reduce-privilege.md
  - wiki/concepts/reduction-instance.md
  - wiki/concepts/mapper.md
  - wiki/concepts/instance-layout.md
  - wiki/concepts/cuda-interop.md
  - wiki/applications/pennant.md
  - wiki/applications/miniaero.md
---

## TL;DR
Circuit is Legion's canonical *graph-application* example. It simulates an arbitrary electrical circuit (nodes = components, edges = wires) using an explicit iterative RLC solver and demonstrates almost every load-bearing Legion concept on one page: region partitioning into private/shared/ghost subsets, REDUCE privileges with associative-commutative ops, reduction instances (fold + list variants), a custom mapper that pins SOA layouts and GPU placement, and `TaskHelper`-templated CPU + GPU variants per phase. Tutorial 11 (https://legion.stanford.edu/tutorial/circuit.html) walks through it. The confusion: documentation oscillates between a custom `AccumulateCharge` reduction op (the older tutorial flavor) and Legion's built-in `SumReduction<float>` (current master sources) — both express the same idea.

## What it computes
A three-phase explicit RLC step over a graph:
1. **`CalcNewCurrentsTask`** — voltage differentials across wires → new wire currents.
2. **`DistributeChargeTask`** — currents push charge onto endpoint nodes (this is the reduction phase — endpoint nodes are shared across pieces).
3. **`UpdateVoltagesTask`** — accumulated charge → new node voltages.

The outer loop is `for (i = 0; i < num_loops; i++)` over the `num_pieces` partition; each piece holds a slab of private nodes, the wires whose left endpoint is private to that piece, and ghost-references to nodes owned by other pieces.

## Region & partition structure
Three top-level regions (see `struct Circuit` in `examples/circuit/circuit.h`):
- `all_nodes` — one element per circuit node; fields `node_cap`, `leakage`, `charge`, `node_voltage`.
- `all_wires` — one element per wire; fields for current, end voltages, in/out node pointers, resistance/inductance/capacitance.
- `node_locator` — small lookup region pinning each node to its piece.

`struct Partitions` carries five logical partitions:
- `pvt_nodes` — disjoint partition of nodes private to each piece (read-write within the piece).
- `shr_nodes` — disjoint partition of nodes owned by each piece but accessed *as endpoints from wires in other pieces* (these are reduction targets).
- `ghost_nodes` — aliased partition giving each piece a view into the *other* pieces' `shr_nodes` (read-only references to neighbors).
- `pvt_wires` — disjoint partition of wires by owning piece (a wire belongs to the piece of its source node).
- `node_locations` — companion partition of `node_locator`.

Each `CircuitPiece` (passed as task argument) bundles the four logical regions plus counters (`num_wires`, `num_nodes`, `first_wire`, `first_node`, `dt`, `steps`).

## Main loop
```cpp
for (int iter = 0; iter < num_loops; iter++) {
  TaskHelper::dispatch_task<CalcNewCurrentsTask>(args, piece_partitions, pieces);
  TaskHelper::dispatch_task<DistributeChargeTask>(args, piece_partitions, pieces);
  TaskHelper::dispatch_task<UpdateVoltagesTask>(args, piece_partitions, pieces);
  if (checks_enabled) TaskHelper::dispatch_task<CheckTask>(args, pieces);  // NaN scan
}
```
Each `dispatch_task` is an `IndexLauncher` over the piece partition. Privileges:
- **CalcNewCurrents**: `READ_WRITE` on wires; `READ_ONLY` on pvt+shr+ghost node voltages.
- **DistributeCharge**: `READ_ONLY` on wires; `READ_WRITE` on pvt node charges; **`REDUCE` (`SumReduction<float>`)** on shr+ghost node charges. The reduction is what lets multiple pieces contribute concurrently to a single shared endpoint.
- **UpdateVoltages**: `READ_WRITE` on node voltages + charges; `READ_ONLY` on capacitance/leakage.

## Legion features exercised
- **`logical-region.md`** + **`field-space.md`** — the substrate.
- **`partition.md`** + **`disjoint-partition.md`** + **`aliased-partition.md`** + **`ghost-region.md`** — the private/shared/ghost trinity. Circuit is the textbook ghost-region pattern.
- **`reduce-privilege.md`** + **`reduction-instance.md`** — `SumReduction<float>` (or the legacy `AccumulateCharge` op with `apply`/`fold`/identity-zero). Both fold and list reduction instance kinds available via the `reduction_list` flag on `RegionRequirement`.
- **`region-requirement.md`** — multiple `RegionRequirement`s per task, each with its own privilege+partition.
- **`mapper.md`** + **`default-mapper.md`** — `CircuitMapper` overrides `map_task` + `map_inline`; legacy versions set `blocking_factor = max_blocking_factor` for SOA, modern versions express the same intent via `default_policy_select_constraints`.
- **`task-variant.md`** + **`cuda-interop.md`** — each phase task registers both a CPU and a GPU variant; GPU implementation uses CUDA `atomicAdd` for reductions.
- **`index-space-launch.md`** — every phase is an index launch over the `num_pieces` piece partition.
- **`tracing.md`** — the three-phase loop is the canonical body for `begin_trace`/`end_trace` memoization.

## Performance characteristics
- **Reduction is the perf-critical step** — `DistributeCharge` must fold contributions from many pieces onto each shared/ghost node. A fold reduction instance per piece + final fold-in at the next consumer is typically faster than reducing in place under atomics.
- **SOA layout is mandatory for vectorization** — the `CircuitMapper` historically pinned this via `blocking_factor`. Without it, the CPU variants don't vectorize over wire fields. Verify in Legion Prof timeline that the wire-task bars are short.
- **GPU variants** use coalesced SOA reads + `atomicAdd` for cross-piece reductions; for most regions SOA on the GPU is also the fastest layout.
- **In-situ correctness**: `CheckTask` (a READ_ONLY NaN-scan) runs in parallel with the next iteration — it's a textbook example of an off-critical-path consumer task.

## Debug signals
- **Long DistributeCharge bars on the timeline** — usually means the reduction is happening atomically into the destination instead of through a fold instance. Check that the mapper requested a reduction instance (`reduction_list` flag, or modern reduction-instance constraint).
- **Wire-task bars longer than expected on CPU** — SOA layout wasn't applied; check `CircuitMapper::map_task` against `instance-layout.md`.
- **GPU rows mostly idle with CPU busy on phase tasks** — GPU variant didn't register or the mapper didn't pick it; cross-link `pitfalls/gpu-underutilization.md`.
- **Bounds-check failures on `ghost_nodes`** — the ghost partition is aliased; verify with `partition-checks.md` (`-lg:partcheck`) that the constructor did the right cross-piece coloring.

## Code excerpts

**The region tree as a data type** (`examples/circuit/circuit.h`):
```cpp
struct Circuit {
  LogicalRegion all_nodes;
  LogicalRegion all_wires;
  LogicalRegion node_locator;
};

struct CircuitPiece {
  LogicalRegion pvt_nodes, shr_nodes, ghost_nodes;
  LogicalRegion pvt_wires;
  unsigned num_wires; Point<1> first_wire;
  unsigned num_nodes; Point<1> first_node;
  float dt; int steps;
};

struct Partitions {
  LogicalPartition pvt_wires;
  LogicalPartition pvt_nodes, shr_nodes, ghost_nodes;
  LogicalPartition node_locations;
};
```
The private/shared/ghost split is made explicit as three named region handles per piece — this is the data-type form of the partition trinity.

**Field-array packing** (`circuit.h`) — Legion's idiom for vector-valued fields without nested types:
```cpp
enum WireFields {
  FID_IN_PTR, FID_OUT_PTR, FID_IN_LOC, FID_OUT_LOC,
  FID_INDUCTANCE, FID_RESISTANCE, FID_WIRE_CAP,
  FID_CURRENT,
  FID_WIRE_VOLTAGE = (FID_CURRENT+WIRE_SEGMENTS),
  FID_LAST = (FID_WIRE_VOLTAGE+WIRE_SEGMENTS-1),
};
```
`FID_CURRENT` through `FID_CURRENT+WIRE_SEGMENTS-1` is a packed field-array.

**REDUCE + SIMULTANEOUS coherence** on the reduction phase (`circuit_cpu.cc`):
```cpp
DistributeChargeTask::DistributeChargeTask(...)
 : IndexLauncher(TASK_ID, launch_domain, ...) {
  RegionRequirement rr_shared(lp_shr_nodes, 0/*identity*/,
                              REDUCE_ID, SIMULTANEOUS, lr_all_nodes);
  rr_shared.add_field(FID_CHARGE);
  add_region_requirement(rr_shared);

  RegionRequirement rr_ghost(lp_ghost_nodes, 0/*identity*/,
                             REDUCE_ID, SIMULTANEOUS, lr_all_nodes);
  rr_ghost.add_field(FID_CHARGE);
  add_region_requirement(rr_ghost);
}
```
`REDUCE_ID` is the registered reduction operator's ID; `SIMULTANEOUS` is the coherence mode that permits concurrent reducers.

**Three accessor flavors in one task body** (`circuit_cpu.cc` — pedagogically dense):
```cpp
typedef ReductionAccessor<SumReduction<float>,false/*exclusive*/,1,coord_t,
                          Realm::AffineAccessor<float,1,coord_t> > AccessorRDfloat;

void DistributeChargeTask::cpu_base_impl(const CircuitPiece &p, ...)
{
  const AccessorROfloat fa_in_current(regions[0], FID_CURRENT);
  const AccessorRWfloat fa_pvt_charge(regions[1], FID_CHARGE);
  const AccessorRDfloat fa_shr_charge(regions[2], FID_CHARGE, REDUCE_ID);
  const AccessorRDfloat fa_ghost_charge(regions[3], FID_CHARGE, REDUCE_ID);

  for (unsigned i = 0; i < p.num_wires; i++) {
    float in_current = -dt * fa_in_current[wire_ptr];
    reduce_node(fa_pvt_charge, fa_shr_charge, fa_ghost_charge,
                in_loc, in_ptr, in_current);
  }
}
```
One task touches `READ_ONLY` (wires), `READ_WRITE` (private nodes), and `REDUCE` (shared+ghost nodes) accessors simultaneously. `PointerLocation` (`in_loc`) tells the dispatch helper which of the three accessors to use per pointer.

**The Regent equivalent** (`language/examples/circuit/circuit_base.rg`) collapses the same logic to a `+=` operator:
```regent
task distribute_charge(rn : region(Node),
                       rw : region(Wire(rn)))
where reads(rw.{in_ptr, out_ptr, current._0, current._2}),
      reads writes(rn.charge)
do
  for w in rw do
    var in_current = -DT * w.current._0
    var out_current = DT * w.current._2
    w.in_ptr.charge += in_current
    w.out_ptr.charge += out_current
  end
end
```
The Regent compiler infers reduction privilege from `+=` and emits the C++-equivalent `ReductionAccessor` under the hood. `region(Wire(rn))` is the wire region *parameterized by* the node region — Regent's type system encodes the pointer relationship statically.

**Toplevel orchestration in Regent — 4 lines** (`circuit_base.rg`):
```regent
for j = 0, conf.num_loops do
  calculate_new_currents(conf.steps, rn, rw)
  distribute_charge(rn, rw)
  update_voltages(rn)
end
```
No explicit launchers, no IndexLaunches; the Regent compiler synthesizes index-space launches from partition annotations elsewhere in the file.

**Custom mapper override surface** (`circuit_mapper.cc`):
```cpp
class CircuitMapper : public DefaultMapper {
public:
  virtual void map_task(const MapperContext ctx, const Task& task,
                        const MapTaskInput& input, MapTaskOutput& output);
  virtual void map_inline(const MapperContext ctx, const InlineMapping& inline_op,
                          const MapInlineInput& input, MapInlineOutput& output);
private:
  void map_circuit_region(...,
                          ReductionOpID redop, LogicalRegion colocation);
};

// In map_task:
if (req.tag == COLOCATION_NEXT_TAG)
  map_circuit_region(ctx, req.region, task.target_proc,
                     target_memory, output.chosen_instances[idx],
                     req.privilege_fields, req.redop,
                     task.regions[idx+1].region);
```
Only `map_task` and `map_inline` are overridden; the rest delegates to `DefaultMapper`. The `COLOCATION_NEXT_TAG` is a Circuit-specific `RegionRequirement.tag` convention that forces two consecutive requirements to share a physical instance — a concrete example of how custom mappers extend default policy through tag conventions.

## Source pointers
- **C++ source**: https://github.com/StanfordLegion/legion/tree/master/examples/circuit
  - `circuit.h`, `circuit.cc`, `circuit_init.cc`, `circuit_cpu.cc`, `circuit_gpu.cu`
  - `circuit_mapper.h`, `circuit_mapper.cc` — `class CircuitMapper : public DefaultMapper`
- **Regent source**: https://github.com/StanfordLegion/legion/tree/master/language/examples
  - `circuit_bishop.rg`, `circuit_sparse.rg` — standalone variants
  - `circuit/circuit_base.rg` — principal Regent entry point; tasks `calculate_new_currents`, `distribute_charge`, `update_voltages`, `toplevel`
  - `circuit/circuit_dep_par.rg`, `circuit_dep_par2*.rg`, `circuit_dep_par3.rg` — dependent-partitioning variants
- **CLI flags**: `-l num_loops`, `-i steps`, `-p num_pieces`, `-npp nodes_per_piece`, `-wpp wires_per_piece`, `-pct pct_wire_in_piece`, `-s random_seed`, `-sync`, `-checks`, `-dump`.

## Real reported issues

Verbatim issues filed against Circuit in `StanfordLegion/legion`. URLs + reporter + fix-status:

- **[#1640 — Bad mapping in modified Circuit](https://github.com/StanfordLegion/legion/issues/1640)** (rupanshusoi, 2024-02-26, **open**). Modified Circuit with a control-replicated wrapper task "does not weak scale to even 16 nodes". Channel utilization doubles 16→32 nodes; profile shows idle gaps from 20.9-21.5s. Bauer's diagnosis: "Pretty much every single task in the second level of the hierarchy [was] mapped remotely from where it was sharded... Tasks were just being randomly sprayed across the machine... the mapping was awful." Fix discussed: write a custom mapper that shards correctly and maps locally. Workaround if using default mapper: pass `-dm:memoize` (Rohan Yadav). **Not resolved upstream.**

- **[#1421 — Implicit execution fences from physical tracing serialize Circuit checkpoint copies](https://github.com/StanfordLegion/legion/issues/1421)** (elliottslaughter, 2023-03-06, **closed**). Attach/copy/detach checkpoint ops on disjoint region trees appeared to serialize with the timestep loop despite no Spy-visible dependence. Slaughter narrowed cause to physical tracing inserting execution fences at trace boundaries. **Workaround**: `-lg:no_physical_tracing`. Bauer: "It's only safe to elide those fences if we know that the traces are idempotent and the replays are back-to-back." Fix landed via Legion MR 698.

- **[#1087 — Tracing regression in `circuit_sparse.rg` without DCR](https://github.com/StanfordLegion/legion/issues/1087)** (elliottslaughter, 2021-05-25, **open**). At 128 nodes on Piz Daint: "tracing, no DCR, no index launches (3.193 seconds) vs no tracing, no DCR, no index launches (2.027 seconds)." Suspected remote-tracing path issue. Companion #1086 reports similar index-launch regression at the same config. **No fix.**

- **[#1241 — Network congestion warning when running Circuit on Avon](https://github.com/StanfordLegion/legion/issues/1241)** (ZamanLantra, 2022-04-23, **closed**). On Mellanox ConnectX-6 HDR100 IB with ibv conduit: "WARNING: A significant number of long latency messages... 4162 messages... longer than 1000.00us... representing 18.57% of 22407 total messages." Bauer's diagnosis: startup-overhead noise from small sample size; at 20+ loop iterations the warning disappeared. Suggested workaround: more utility processors via `-ll:util N`.

- **[#1041 — LLVM 6/13 CUDA codegen regression](https://github.com/StanfordLegion/legion/issues/1041)** (elliottslaughter, 2021-04-03, **closed**). Single-node Circuit slowed 2.0s → 2.5s (+25%) moving Regent from LLVM 3.8 to LLVM 6. Root cause: LLVM 13 emitted `add.rn.f32`/`mul.rn.f32` instead of `add.f32`/`mul.f32`, suppressing FMA fusion. Fix: Slaughter enabled the `contract` fast-math flag in Terra's CUDA codegen path (terralang/terra#528 + the `regent-fast-math` branch); final result 1.9562s vs 1.9726s baseline.

- **[#1436 — Mapper memoization-key bug causing "Bad co-location"](https://github.com/StanfordLegion/legion/issues/1436)** (elliottslaughter, 2023-03-14, **closed**). C++ Circuit at `-pct 98` or 4 nodes hit mapper errors due to insufficient memoization key (commit 1a408cd). Bauer's fix: "The memoization key in the mapper wasn't sufficient. Pull and try again."

## Tool-coverage classification

How each real reported issue above splits across the standard Legion diagnostic stack:

**Legion Prof alone surfaces it** (timing/locality patterns visible in the timeline + utility + channel rows):
- **#1640** — "tasks randomly sprayed across the machine": locality visible on processor rows; channel utilization doubling 16→32 nodes visible on channel rows. Bauer literally diagnosed this in Legion Prof.
- **#1087** — "physical replay trace boxes that seem to run in the range of 20-25 ms": meta-task rows show the long bars.
- **Slaughter's CR scaling claim for Circuit (cr2017)** — every weak-scaling regression is first a Prof regression vs. a saved baseline.

**Legion Prof + Legion Spy together are needed** (Prof shows the gap, Spy answers "is there a real logical dependence?"):
- **#1421** — Slaughter explicitly says "appeared to serialize... despite no Spy-visible dependence." Prof showed the serialization gap; Spy ruled out a logical edge; the gap was caused by an **invisible** physical-tracing fence — neither tool draws those.
- **#1436** — mapper "Bad co-location" is a runtime error caught before execution, but post-fix verification (no co-location violations) uses Spy's event-graph for confidence.

**Profiler can't tell** (need runtime logs, runtime warning text, source inspection, or external tools):
- **#1041** — LLVM CUDA codegen regression. Prof shows tasks running slower but cannot identify *why* the GPU kernel is slower; root cause was PTX `add.rn.f32` suppressing FMA — needs reading generated PTX.
- **#1241** — GASNet long-latency-message warning is emitted by GASNet to stderr at shutdown, not from Prof. Prof can confirm the warning correlates with startup (small sample size).
- **Mapper errors** (#1436 pre-fix) — fatal errors before any Prof trace is produced; surface via `LEGION_BACKTRACE=1` + the error-message catalog.

## Papers that benchmark it
- **Slaughter thesis (2017)** — Circuit is a Regent showcase application (`raw/publications/pdfs/slaughter_thesis.pdf`).
- **dcr2021** — Dynamic Control Replication uses Circuit-class workloads as a baseline.
- **trace2018** (Dynamic Tracing) — Circuit is one of five evaluation programs; benefits >4× from tracing because its three-phase loop has no convergence-test stall.
- **idx2021** (Index Launches) — Circuit is in the evaluation suite.

## Related
- `wiki/applications/pennant.md` — sibling canonical Legion app; unstructured mesh hydro vs. graph RLC.
- `wiki/applications/miniaero.md` — sibling canonical Legion app; unstructured 3D CFD.
- `wiki/concepts/reduce-privilege.md` — the privilege Circuit exercises most heavily.
- `wiki/concepts/ghost-region.md` — the partition idiom Circuit canonized.
- `wiki/concepts/cuda-interop.md` — Circuit's GPU variants substrate.
- `wiki/concepts/tracing.md` — Circuit benefits ~4× from dynamic tracing per trace2018.

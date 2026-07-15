---
title: Move from Single-Node to Distributed
slug: move-from-single-node-to-distributed
summary: A recipe for scaling a working single-node Legion app to multiple nodes; the path is GASNet build, control replication, tracing, and validation with Legion Prof per-shard rows.
tags: [for-perf-debug, distributed, replication]
status: draft
created: 2026-05-15
updated: 2026-05-15
related:
  - wiki/concepts/control-replication.md
  - wiki/concepts/replicable-task.md
  - wiki/concepts/sharding-functor.md
  - wiki/concepts/tracing.md
  - wiki/concepts/realm-machine-model.md
  - wiki/concepts/legion-prof.md
  - wiki/concepts/gasnet.md
  - wiki/concepts/runtime-flags-reference.md
  - wiki/pitfalls/runtime-overhead-dominates.md
---

## Inputs

- A working single-node Legion application using the default mapper or a custom one.
- A target multi-node cluster with GASNet (or GASNetEx) installed.

## Steps

1. **Build Legion with GASNet enabled**:
   ```bash
   USE_GASNET=1 make
   ```
   `gasnet` is the system layer Realm uses for inter-node communication. The wiki defers GASNet's deep internals as outside the application-debug scope, but the build flag is required. See `raw/website-pages/gasnet.md` for installation details.

2. **Register the top-level task as replicable** (`replicable-task.md`):
   ```cpp
   TaskVariantRegistrar reg(TOP_LEVEL_TASK_ID, "top_level");
   reg.add_constraint(ProcessorConstraint(Processor::LOC_PROC));
   reg.set_replicable(true);
   Runtime::preregister_task_variant<top_level_task>(reg, "top_level");
   ```
   Without `set_replicable`, the top-level task runs on one processor across all nodes and dependence analysis becomes the bottleneck (`pitfalls/runtime-overhead-dominates.md` at scale).

3. **Ensure the task is deterministic**. A replicable task's body must produce the same operation stream given the same logical inputs across all shards (`replicable-task.md` invariants). Audit for:
   - `rand()` / `clock()` / file IO outside Legion (sample via tunable variables or futures instead).
   - Mutable globals that vary per process.
   - Side effects that diverge across shards.

4. **Launch on multiple nodes**:
   ```bash
   mpirun -np 4 ./app -ll:cpu 16 -ll:gpu 4 -ll:csize 8000 -ll:fsize 16000 -ll:util 4
   ```
   `-ll:cpu` / `-ll:gpu` / `-ll:csize` / `-ll:fsize` / `-ll:util` set per-node CPU/GPU counts and memory sizes — see `realm-machine-model.md` for the full set.

5. **Verify control replication is active**: a printf in the top-level task body should appear N times in the output (one per shard). If you see it once, replication isn't engaged.

6. **Tune the sharding functor** (`sharding-functor.md`). `default-mapper.md` uses linear `point % N`, which is fine for uniform workloads. For irregular work distributions:
   ```cpp
   class MyShardingFunctor : public ShardingFunctor {
   public:
     ShardID shard(const DomainPoint &p, const Domain &launch_space,
                   const size_t total_shards) override {
       // application-specific: block-cyclic, 2D tiled, etc.
     }
   };
   Runtime::preregister_sharding_functor(MY_SHARD_ID, new MyShardingFunctor());
   ```
   Have the mapper pick it via `select_sharding_functor`.

7. **Enable tracing** (`wiki/workflows/enable-tracing.md`). Tracing's benefit multiplies under control replication — per-shard analysis collapses to near-zero on replay.

8. **For RDMA networking**, enable registered memory:
   ```bash
   ./app -ll:rsize 4096
   ```
   `REGDMA_MEM` is required for high-bandwidth inter-node copies; default is 0.

9. **Profile per-shard activity** with `legion-prof.md`. Each shard contributes its rows. Look for:
   - **Balanced shards**: utility activity on each shard should be roughly equal. Skew = bad sharding functor.
   - **Inter-node channel rows**: heavy activity between nodes' memories indicates the data partition forces cross-node copies; consider a better partition.
   - **One shard idle while others busy**: a sharding bug — possibly non-deterministic top-level task.

10. **Validate the result is correct** at the new scale — large data-parallel workloads can mask races that only manifest with N>1 shards.

## Outputs

- A Legion app running correctly on multi-node hardware.
- A per-shard profile confirming balanced load and no cross-node hot spots.
- Quantified scaling efficiency vs. the single-node baseline.

## When to use

- An application is correct on a single node and the user wants to scale.
- A profile shows `pitfalls/runtime-overhead-dominates` at scale — the symptom is missing control replication.
- The application uses iterated loops that benefit from per-shard tracing.

## Related

- `wiki/concepts/control-replication.md` — the SPMD execution model.
- `wiki/concepts/replicable-task.md` — the `set_replicable` opt-in.
- `wiki/concepts/sharding-functor.md` — per-point ownership across shards.
- `wiki/concepts/tracing.md` — multiplies the win.
- `wiki/concepts/realm-machine-model.md` — runtime configuration flags.
- `wiki/concepts/legion-prof.md` — verify per-shard balance.
- `wiki/pitfalls/runtime-overhead-dominates.md` — the symptom this workflow resolves.

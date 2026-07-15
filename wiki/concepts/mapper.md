---
title: Mapper
slug: mapper
summary: A per-processor C++ object that decides where each task runs and where each physical instance lives; orthogonal to correctness, controls all of Legion's performance.
tags: [mapping, configuration, for-perf-debug, for-program-reasoning]
subsystem: legion
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/website-pages/mapper.md
  - raw/tutorials/10_custom_mappers.md
  - raw/website-pages/debugging.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion_mapping.h
  - https://github.com/StanfordLegion/legion/blob/master/runtime/mappers/default_mapper.h
  - https://github.com/StanfordLegion/legion/blob/master/runtime/mappers/default_mapper.cc
related:
  - wiki/concepts/task.md
  - wiki/concepts/physical-instance.md
  - wiki/concepts/operation-pipeline.md
  - wiki/concepts/control-replication.md
  - wiki/concepts/legion-prof.md
  - wiki/concepts/index-space-launch.md
  - wiki/concepts/default-mapper.md
  - wiki/concepts/sharding-functor.md
  - wiki/concepts/realm-machine-model.md
  - wiki/concepts/tunable-variable.md
  - wiki/concepts/task-variant.md
  - wiki/concepts/mapper-callback.md
  - wiki/concepts/mapper-context.md
  - wiki/concepts/select-task-options.md
  - wiki/concepts/slice-task.md
  - wiki/concepts/map-task.md
  - wiki/concepts/virtual-mapping.md
  - wiki/concepts/mapper-logging.md
  - wiki/concepts/automated-mapping.md
  - wiki/concepts/select-instance.md
---

## TL;DR
The mapper is the one place in a Legion application that decides **policy**: which processor a task runs on, which memory holds each physical instance, what layout (AOS/SOA/dimension order) the instance uses, when to steal work, when to inline. Mappers are user-written (or `DefaultMapper`-derived) callback objects, one per processor. The runtime calls them at specific pipeline stages and applies their answers. The confusion: mappers cannot make a program *incorrect* (the runtime enforces semantics regardless), but they can absolutely make it slow.

## Mental model
Legion's design principle is **"correctness is the runtime's job; performance is the mapper's job"**. Mappers are like the policy plug-in of an OS scheduler — the runtime gives them the candidates (ready tasks, valid instances, machine topology) and asks for the decision (where, what, when). Default mapper provides "sane defaults"; custom mappers encode domain knowledge ("GPU-bound kernels go to TOC_PROC", "halo cells go to zero-copy memory").

## Mechanism & API
A mapper subclass overrides callbacks from `Mapping::Mapper`. Most subclass `DefaultMapper` to inherit reasonable behavior:

```cpp
class MyMapper : public DefaultMapper {
public:
  MyMapper(MapperRuntime *rt, Machine machine, Processor local, const char *name)
    : DefaultMapper(rt, machine, local, name) {}
  void map_task(const MapperContext ctx, const Task &t,
                const MapTaskInput &in, MapTaskOutput &out) override;
};

void mapper_registration(Machine m, Runtime *rt,
                         const std::set<Processor> &local_procs) {
  for (auto p : local_procs)
    rt->replace_default_mapper(new MyMapper(rt->get_mapper_runtime(),
                                            m, p, "my_mapper"), p);
}
Runtime::add_registration_callback(mapper_registration);
```

Key callbacks (load-bearing for perf):
- `select_task_options` — initial proc, inline/stealable/replicate decisions, **`memoize` to enable tracing**.
- `slice_task` — split an index launch's domain across target processors (round-robin GPUs, etc.).
- `select_tasks_to_map` — back-pressure / priority gate.
- `map_task` — the big one. Pick instances, the variant (CPU vs GPU), and target processor.
- `postmap_task` — create prefetch copies in other memories.
- `select_sharding_functor` — under control replication, distribute index-launch points across shards.
- `select_steal_targets` / `permit_steal_request` — work-stealing.

The mapper queries the `Machine` model for processors (`LOC_PROC`/`TOC_PROC`/`IO_PROC`/`UTIL_PROC`/`PROC_GROUP`) and memories (`SYSTEM_MEM`/`GPU_FB_MEM`/`Z_COPY_MEM`/`REGDMA_MEM`/`SOCKET_MEM`/...) plus their affinities (bandwidth, latency).

## Invariants
- Mapper callbacks **must not block or perform long-running work**; the runtime cannot make progress while a callback is in flight.
- A `MapperContext` is **valid only inside the callback** that produced it. Never cache or cross callbacks.
- By default callbacks are **serialized per-mapper-instance**. Set `concurrent = true` to opt into reentrancy (requires application-level locking).
- Mappers receive **suggestions**, not requirements: the application is allowed to launch a task with `MapperID::DEFAULT_MAPPER_ID` and the mapper chooses every other thing. Tags and per-region requirement tags are how applications pass hints without coupling.
- A mapper **cannot violate program semantics** — it can move tasks anywhere, but the runtime still issues the copies and dependencies needed to keep results correct.

## Performance implications
- The mapper is the single biggest perf knob in any Legion app. `DefaultMapper` is fine for tutorials; production workloads typically subclass it.
- **Slow mapper callbacks bottleneck the runtime**: utility processors run mapping, and they're shared.
- Pick instances close to the executing processor (consult `proc_mem_affinity`). Cross-memory placements force DMAs visible as channel-row activity in `legion-prof.md`.
- `select_task_options::memoize = true` is what gates dynamic tracing. Without it, traces are not memoized — see `tracing.md`.
- **Automated mapping** (paper `automap2023.pdf`) can replace hand-written mappers in some workflows.

## Debug signals
- **`LoggingWrapper`**: wrap the mapper in `runtime/mappers/logging_wrapper.h` and run with `-level mapper=2`. Logs every callback's inputs and outputs.
- **Legion Prof**: tasks bouncing between processor rows from launch to launch → unstable mapper decisions.
- **Legion Prof channel rows** active for every task → instances are not co-located with processors.
- **GPU rows idle** while CPU rows are busy → mapper put GPU-eligible tasks on CPUs (missing variant or wrong `target_procs`).

## Failure modes
- [Mapper bouncing](../pitfalls/mapper-bouncing.md) — unstable processor placement.
- [Mapper stalls](../pitfalls/mapper-stalls.md) — slow callbacks block runtime progress.
- [GPU underutilization](../pitfalls/gpu-underutilization.md) — wrong variant or wrong target memory.
- [Excessive data movement](../pitfalls/excessive-data-movement.md) — instances placed far from compute.

## Source pointers
- **Mapper API header**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion_mapping.h
- **DefaultMapper**: https://github.com/StanfordLegion/legion/blob/master/runtime/mappers/default_mapper.h
- **LoggingWrapper**: https://github.com/StanfordLegion/legion/blob/master/runtime/mappers/logging_wrapper.h
- **Tutorial**: https://legion.stanford.edu/tutorial/custom_mappers.html
- **Reference**: https://legion.stanford.edu/mapper/

## Related
- `wiki/concepts/task.md` — what the mapper decides things about.
- `wiki/concepts/physical-instance.md` — what `map_task` creates / selects.
- `wiki/concepts/operation-pipeline.md` — when each callback fires.
- `wiki/concepts/control-replication.md` — sharding functor handoff.
- `wiki/concepts/legion-prof.md` — how to see what the mapper actually did.

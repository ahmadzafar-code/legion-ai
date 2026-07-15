---
title: Default Mapper
slug: default-mapper
summary: Legion's reference mapper implementation; provides sane defaults for every mapper callback and is the base class most application mappers subclass.
tags: [mapping, configuration, for-perf-debug, for-program-reasoning]
subsystem: legion
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/website-pages/mapper.md
  - raw/tutorials/10_custom_mappers.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/mappers/default_mapper.h
  - https://github.com/StanfordLegion/legion/blob/master/runtime/mappers/default_mapper.cc
related:
  - wiki/concepts/mapper.md
  - wiki/concepts/physical-instance.md
  - wiki/concepts/task.md
  - wiki/concepts/control-replication.md
  - wiki/concepts/tunable-variable.md
  - wiki/concepts/mapper-callback.md
  - wiki/concepts/select-task-options.md
  - wiki/concepts/map-task.md
  - wiki/concepts/mapper-logging.md
---

## TL;DR
`DefaultMapper` is the C++ class Legion ships as the reference mapper. It implements every callback in the `Mapping::Mapper` interface with reasonable defaults — pick the local processor of the task variant's kind, choose the best memory by affinity, use SOA layouts, allow stealing, etc. Custom mappers almost always **inherit** from `DefaultMapper` and override only the callbacks where they want application-specific policy. The confusion: `DefaultMapper` IS production-ready; "custom mapper" doesn't necessarily mean "from scratch" — it usually means "DefaultMapper + 2 overrides".

## Mental model
Think of `DefaultMapper` like libstdc++'s default `std::allocator`: complete, reasonable, and what 90% of programs ship with unchanged. A "custom mapper" is then like a custom allocator — you subclass, swap behavior for one or two operations, and inherit everything else. Both `AdversarialMapper` (random decisions, used for stress-testing the runtime) and `PartitioningMapper` (tunable-variable-driven) in the tutorial are `DefaultMapper` subclasses.

## Mechanism & API
```cpp
class MyMapper : public DefaultMapper {
public:
  MyMapper(MapperRuntime *rt, Machine m, Processor local, const char *name)
    : DefaultMapper(rt, m, local, name) {}

  void select_task_options(const MapperContext ctx, const Task &t,
                           SelectTaskOptionsOutput &out) override {
    DefaultMapper::select_task_options(ctx, t, out);  // start from defaults
    if (t.task_id == GPU_TASK_ID) {
      Machine::ProcessorQuery pq(machine); pq.only_kind(Processor::TOC_PROC);
      out.initial_proc = pq.first();
    }
  }
};
```

What `DefaultMapper` provides:
- **select_task_options**: local processor; `inline_task=false`, `stealable=false`, `memoize=true`.
- **slice_task**: round-robin distribution of an index launch across same-kind processors.
- **map_task**: pick a `target_proc` matching the task variant; pick a memory by `proc_mem_affinity`; create an SOA instance with `find_or_create_physical_instance`.
- **select_steal_targets / permit_steal_request**: conservative defaults — do not bother attempting steals unless idle.
- **report_profiling**: drops profiling data.
- **select_sharding_functor** (under control replication): the linear `0..N-1` sharding functor (`DEFAULT_SHARD_ID`).

Helper methods you can call from your subclass:
- `default_policy_select_target_memory(ctx, proc, req)` — best memory for a given (proc, region requirement).
- `default_policy_select_constraints(ctx, layout, mem, req)` — produces a sensible default layout constraint set.
- `default_policy_select_variant(ctx, task, proc)` — pick a task variant compatible with the target processor kind.
- `map_constrained_requirement` / `map_random_requirement` — used in the tutorial mappers.

## Invariants
- `DefaultMapper` is one instance **per processor** (the `local` argument). Subclasses preserve this; multiple instances of the subclass run on different processors of the same node.
- All `DefaultMapper` callbacks are **non-blocking** and **non-reentrant** by default. Subclasses inherit that contract.
- Helper methods (`default_policy_*`) are safe to call from any mapper callback; their `MapperContext` requirements match the calling callback's lifetime.
- `DefaultMapper` works correctly under control replication out of the box: it picks the linear sharding functor.

## Performance implications
- `DefaultMapper` is **sane but not optimal**. For tutorials, single-node debugging, and many production workloads, it's enough. Beyond that, override `map_task` to control instance placement and `slice_task` to control index-launch distribution.
- The single biggest perf win from a custom mapper is usually **steering GPU-eligible tasks to GPU processors and creating instances in GPU framebuffer memory** — `DefaultMapper` can't know which of your task IDs prefer GPU.
- `DefaultMapper` enables `memoize` for most tasks by default, so tracing (`tracing.md`) works without further configuration.
- Helper methods like `default_policy_select_target_memory` do non-trivial work (querying affinities). Cache results in the mapper constructor if you call them on the hot path.

## Debug signals
- **`LoggingWrapper`** (`runtime/mappers/logging_wrapper.h`): wraps any mapper to log every callback's input/output. Use during development.
- **`-level mapper=2`**: also enables internal mapper logging.
- **Mapper bouncing** between processors → `DefaultMapper` heuristic is unstable for your workload; override with a pinned policy.

## Failure modes
- [Mapper bouncing](../pitfalls/mapper-bouncing.md)
- [GPU underutilization](../pitfalls/gpu-underutilization.md) — `DefaultMapper` won't push tasks to GPU without help.
- [Mapper stalls](../pitfalls/mapper-stalls.md) — sometimes triggered by uncached helper-method calls in a subclass's `map_task`.

## Source pointers
- **Header**: https://github.com/StanfordLegion/legion/blob/master/runtime/mappers/default_mapper.h
- **Implementation**: https://github.com/StanfordLegion/legion/blob/master/runtime/mappers/default_mapper.cc
- **Logging wrapper**: https://github.com/StanfordLegion/legion/blob/master/runtime/mappers/logging_wrapper.h
- **Tutorial**: https://legion.stanford.edu/tutorial/custom_mappers.html

## Related
- `wiki/concepts/mapper.md` — the interface `DefaultMapper` implements.
- `wiki/concepts/physical-instance.md` — what `default_policy_*` produces.
- `wiki/concepts/task.md` — variants and processor kinds.
- `wiki/concepts/control-replication.md` — `DefaultMapper` picks the linear sharding functor.

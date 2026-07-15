---
title: Write a Custom Mapper
slug: write-a-custom-mapper
summary: A recipe for subclassing `DefaultMapper` and overriding the callbacks needed for application-specific placement; the standard path for fixing `pitfalls/gpu-underutilization` and `pitfalls/excessive-data-movement`.
tags: [for-perf-debug, mapping]
status: draft
created: 2026-05-15
updated: 2026-05-15
related:
  - wiki/concepts/mapper.md
  - wiki/concepts/default-mapper.md
  - wiki/concepts/mapper-callback.md
  - wiki/concepts/mapper-context.md
  - wiki/concepts/select-task-options.md
  - wiki/concepts/map-task.md
  - wiki/concepts/slice-task.md
  - wiki/concepts/mapper-logging.md
  - wiki/concepts/realm-machine-model.md
---

## Inputs

- An existing Legion / Regent application using the default mapper.
- A profile or symptom indicating the mapper is the bottleneck (e.g., GPU rows idle, channel rows busy).

## Steps

1. **Confirm the mapper is actually the problem**. Profile with `legion-prof.md` first. Mapper-related symptoms: `pitfalls/gpu-underutilization`, `pitfalls/excessive-data-movement`, `pitfalls/mapper-bouncing`, `pitfalls/mapper-stalls`. If the bottleneck is elsewhere (e.g., `runtime-overhead-dominates`), a custom mapper won't help.

2. **Subclass `DefaultMapper`** — not the base `Mapper`. `default-mapper.md` provides sane defaults for every callback; you override only what you need.
   ```cpp
   #include "default_mapper.h"
   #include "mappers/logging_wrapper.h"
   class MyMapper : public DefaultMapper {
   public:
     MyMapper(MapperRuntime *rt, Machine m, Processor p, const char *name)
       : DefaultMapper(rt, m, p, name) {
       // Cache machine-model queries here, NOT in callbacks.
       Machine::ProcessorQuery pq(m); pq.only_kind(Processor::TOC_PROC);
       gpus_.assign(pq.begin(), pq.end());
     }
     // override only what you need
   private:
     std::vector<Processor> gpus_;
   };
   ```
   Caching in the constructor (per `pitfalls/mapper-stalls.md`) is critical — `Machine` queries in hot paths kill performance.

3. **Override `select_task_options`** for processor steering (`select-task-options.md`):
   ```cpp
   void select_task_options(const MapperContext ctx, const Task &t,
                            SelectTaskOptionsOutput &out) override {
     DefaultMapper::select_task_options(ctx, t, out);  // inherit defaults
     if (t.task_id == GPU_TASK_ID && !gpus_.empty()) {
       out.initial_proc = gpus_[0];   // steer to a GPU
     }
   }
   ```
   The default for `out.memoize` from `DefaultMapper` is true — keep it that way unless you have a specific reason to opt out (and break `tracing.md`).

4. **Override `map_task`** for instance-placement decisions (`map-task.md`):
   ```cpp
   void map_task(const MapperContext ctx, const Task &t,
                 const MapTaskInput &in, MapTaskOutput &out) override {
     // 1. Pick variant for the target processor kind.
     // 2. Pick memory by proc_mem_affinity for the chosen target.
     // 3. Build a LayoutConstraintSet (instance-layout.md).
     // 4. Call find_or_create_physical_instance — NOT create_instance.
   }
   ```
   The four steps above are the standard sequence. Use `default_policy_select_target_memory(ctx, proc, req)` to skip step 2 with a sensible default.

5. **Override `slice_task`** for index-launch distribution if you need non-round-robin (`slice-task.md`).

6. **Register the mapper** in your registration callback, wrapped with `LoggingWrapper` during development:
   ```cpp
   void mapper_registration(Machine m, Runtime *rt,
                            const std::set<Processor> &procs) {
     for (auto p : procs) {
       auto *underlying = new MyMapper(rt->get_mapper_runtime(), m, p, "my");
       rt->replace_default_mapper(new LoggingWrapper(underlying), p);
     }
   }
   Runtime::add_registration_callback(mapper_registration);
   ```

7. **Run with `-level mapper=2`** and read the log (`mapper-logging.md` + `logger-categories.md`):
   ```bash
   ./app -level mapper=2 -logfile mapper_%.log
   ```
   Verify the chosen processor/variant/instance fields match your expectations for each task.

8. **Re-profile** with `legion-prof.md` — the symptoms you started with should be reduced or gone.

9. **Strip `LoggingWrapper` for production**: it has measurable overhead. Keep the underlying mapper.

## Outputs

- A `DefaultMapper` subclass with the minimum number of overrides needed.
- A re-profiled run confirming the symptom is resolved.
- A clean mapper-log audit showing decisions are stable across iterations (no `pitfalls/mapper-bouncing`).

## When to use

- Profile shows GPU rows empty despite GPU variants existing.
- Channel rows dominated by cross-memory copies.
- Tasks bouncing between processor kinds across iterations.
- Application-specific knowledge can pick placement better than `DefaultMapper`'s general heuristics.

## Related

- `wiki/concepts/mapper.md` — the interface.
- `wiki/concepts/default-mapper.md` — the base class.
- `wiki/concepts/mapper-callback.md` — non-blocking rules.
- `wiki/concepts/mapper-context.md` — the per-callback handle.
- `wiki/concepts/select-task-options.md` / `wiki/concepts/map-task.md` / `wiki/concepts/slice-task.md` — the most-overridden callbacks.
- `wiki/concepts/mapper-logging.md` — required during development.
- `wiki/concepts/realm-machine-model.md` — what to query (and cache).

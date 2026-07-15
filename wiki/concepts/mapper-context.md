---
title: Mapper Context
slug: mapper-context
summary: The opaque handle every mapper callback receives; required input to all `MapperRuntime` API calls; valid for the duration of one callback only.
tags: [mapping, configuration, for-program-reasoning]
subsystem: legion
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/website-pages/mapper.md
  - raw/tutorials/10_custom_mappers.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion_mapping.h
related:
  - wiki/concepts/mapper.md
  - wiki/concepts/mapper-callback.md
  - wiki/concepts/select-task-options.md
  - wiki/concepts/slice-task.md
  - wiki/concepts/map-task.md
---

## TL;DR
A `MapperContext` is an opaque per-callback handle the runtime hands to every mapper method. It identifies *which* callback is currently in flight and authorizes use of the `MapperRuntime` API (instance lookups, lock acquisition, mapper-event triggers). The handle is **valid only inside the callback that received it** — cache or cross-use is undefined behavior. The confusion: it's not a thread-local context the mapper owns. It's a token the runtime passes in, and reusing one beyond its callback's return is a recipe for crashes that only manifest under heavy concurrent mapping.

## Mental model
A `MapperContext` is the mapper's equivalent of a file descriptor returned by `open()` — opaque, scope-bounded, and required for every operation against the resource (here, the runtime's mapper interface). Where a file descriptor is invalid after `close()`, a `MapperContext` is invalid after the callback returns.

## Mechanism & API
The runtime synthesizes a fresh `MapperContext` per callback invocation and passes it as the first parameter:
```cpp
void MyMapper::select_task_options(const MapperContext ctx,
                                    const Task &task,
                                    SelectTaskOptionsOutput &output) {
  // ctx is valid only inside this body.
  Machine::ProcessorQuery pq(machine); pq.only_kind(Processor::TOC_PROC);
  output.initial_proc = pq.first();
}
```

Use it to call `MapperRuntime` methods:
```cpp
std::vector<VariantID> variants;
runtime->find_valid_variants(ctx, task.task_id, variants);
```

Most mapper-runtime APIs accept `ctx` as their first argument. They include:
- Variant queries: `find_valid_variants`, `find_execution_constraints`.
- Instance management: `find_or_create_physical_instance`, `acquire_instance`, `release_instance`.
- Synchronization: `create_mapper_lock`, `lock_mapper`, `unlock_mapper`, `destroy_mapper_lock`.
- Mapper events: `create_mapper_event`, `trigger_mapper_event`, `wait_on_mapper_event`.
- Profiling: `pack_profiling`, `unpack_profiling`.
- Inter-mapper messaging: `send_message`, `broadcast_message`.

## Invariants
- A `MapperContext` is **valid only inside the callback that produced it**. Storing it in a member variable and using it from another callback is undefined behavior.
- A context **uniquely identifies one callback invocation** — separate concurrent callbacks on the same mapper instance have distinct contexts.
- All `MapperRuntime` calls require the context; calling them without one (e.g., from a destructor) is undefined.
- A context is **not transferable across mapper instances** — each processor's mapper instance gets its own contexts.
- Some APIs (notably `wait_on_mapper_event`) can **block the callback**; while waiting, the context remains valid but the runtime may run other callbacks concurrently (subject to the mapper's reentrancy mode).

## Performance implications
- Context handling is essentially free — a pointer pass.
- The main perf consideration is **what you do with the context inside the callback**: heavyweight `MapperRuntime` calls (large instance queries, complex variant filtering) extend callback duration and risk `mapper-stalls`.
- Caching results of cheap context-driven queries (e.g., machine model, processor lists) in the mapper constructor avoids per-callback repetition.

## Debug signals
- **`LoggingWrapper`** records each callback's entry/exit with its context. Long callback duration usually means inefficient `MapperRuntime` usage inside.
- **Use-after-return crashes** in custom mappers are almost always cached `MapperContext` values.

## Failure modes
- Caching a `MapperContext` across callbacks → use-after-free crash (often manifests only under load).
- Calling a `MapperRuntime` API from a destructor or non-callback context → undefined behavior.

## Source pointers
- **Mapper API header**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion_mapping.h
- **Reference**: `raw/website-pages/mapper.md`

## Related
- `wiki/concepts/mapper.md` — host concept.
- `wiki/concepts/mapper-callback.md` — umbrella for callbacks that receive contexts.
- `wiki/concepts/select-task-options.md`, `wiki/concepts/slice-task.md`, `wiki/concepts/map-task.md` — specific callbacks.

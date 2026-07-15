---
title: Instance Fragmentation
slug: instance-fragmentation
summary: Many small, short-lived physical instances per region cause garbage-collection pressure, lower DMA throughput, and out-of-memory errors despite seemingly-sufficient `-ll:csize`/`-ll:fsize`.
tags: [for-perf-debug, instances, memory]
status: draft
created: 2026-05-15
updated: 2026-05-15
related:
  - wiki/concepts/physical-instance.md
  - wiki/concepts/instance-layout.md
  - wiki/concepts/mapper.md
  - wiki/concepts/map-task.md
  - wiki/concepts/legion-prof.md
  - wiki/concepts/equivalence-set.md
---

## Symptom

- **Memory rows** in Legion Prof show **many short slabs** instead of a few long ones — each task seems to create its own instance.
- Out-of-memory errors mid-run despite `-ll:csize` / `-ll:fsize` set to what looks like plenty.
- Compiling with `-DLEGION_GC` and running `tools/legion_gc.py` reports many allocations and immediate collections.
- **`-DTRACE_ALLOCATION`** + `-level allocation=2` logs show a high allocation rate per second.

## Cause

The mapper's `map_task.md` creates **fresh physical instances** instead of reusing existing valid ones. Three common patterns:

1. **Inconsistent `LayoutConstraintSet` between calls**: `find_or_create_physical_instance` only reuses an instance whose layout exactly matches the constraints. Slightly-different constraints (different field order, different dimension order, different alignment) force fresh allocation. See `instance-layout.md`.
2. **Always calling `create_instance` instead of `find_or_create_physical_instance`**: explicit fresh allocation regardless of what's valid. Common in mappers written from the AdversarialMapper template.
3. **Premature destruction**: explicitly destroying instances at the end of each task (`runtime->destroy_instance`) prevents the runtime from caching them across calls.

A secondary cause is **equivalence-set fragmentation** (`equivalence-set.md`): highly-aliased or finely-partitioned regions split the equivalence-set forest, forcing many distinct instances rather than one shared one.

In production runs, fragmentation often manifests as a **memory leak**: each iteration leaks a few KB-MB of instance state until the memory subsystem can no longer allocate, even though the *logical* data structure hasn't grown.

## Fix

- **Stabilize layout constraints across `map_task` calls**: same field order, same dimension order, same memory-kind. Pull constraint construction into a helper called from one place.
- **Always prefer `find_or_create_physical_instance`** over `create_instance` — the runtime reuses valid instances when constraints match.
- **Hold references on long-lived instances** by stashing them in the mapper's state with `runtime->acquire_instance` (and releasing later). This prevents the GC from collecting them between operations.
- **Coarsen partitions**: fewer subregions = fewer equivalence sets = fewer distinct instances. Often a partition with N=1000 colors fragments worse than N=100.
- **For long-running loops**, premap the working set once outside the loop and let the runtime keep those instances warm; map only ephemeral data inside.
- **Confirm the fix** with `tools/legion_gc.py` on a `-DLEGION_GC` build — allocation count per iteration should drop sharply.

## Underlying concepts

- `wiki/concepts/physical-instance.md` — what's fragmenting.
- `wiki/concepts/instance-layout.md` — the constraint set that gates reuse.
- `wiki/concepts/mapper.md` / `wiki/concepts/map-task.md` — where the allocation decision is made.
- `wiki/concepts/legion-prof.md` — where the symptom is visible (memory rows).
- `wiki/concepts/equivalence-set.md` — the secondary cause of forced-distinct instances.

---
title: slice_task
slug: slice-task
summary: Mapper callback for index-space task launches; divides the launch domain into slices, each assigned to a target processor (and optionally recursively re-sliced on that processor's node).
tags: [mapping, parallelism, for-perf-debug]
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
  - wiki/concepts/mapper-context.md
  - wiki/concepts/index-space-launch.md
  - wiki/concepts/map-task.md
  - wiki/concepts/sharding-functor.md
---

## TL;DR
`slice_task` runs once per index-space launch on the launching processor's mapper. The mapper inspects the launch `Domain` and returns a list of `TaskSlice`s — each pair of (sub-domain, target processor) determines which point tasks go where. Slices can **recursively re-slice** on the target node (a node-local mapper gets called again to subdivide further). The confusion: `slice_task` runs *before* per-point `map_task` and *separately* from the sharding functor under control replication — the functor decides per-shard ownership; `slice_task` decides per-processor ownership within a shard.

## Mental model
`slice_task` is the load balancer for an index launch. Where `sharding-functor.md` divides points across replicated shards (one logical decision visible to all shards), `slice_task` divides each shard's points across processors local to that shard. Together they answer "which CPU/GPU runs each point".

## Mechanism & API
Signature:
```cpp
void slice_task(const MapperContext ctx,
                const Task &task,
                const SliceTaskInput &input,
                SliceTaskOutput &output);
```

**Input** (`SliceTaskInput`):
- `input.domain` — the launch domain to be sliced.

**Output** (`SliceTaskOutput`):
- `output.slices` — `std::vector<TaskSlice>` covering the entire domain.
- `output.verify_correctness` — runtime sanity-checks the slices cover the domain exactly (default `false`).

**`TaskSlice` fields**:
- `slice.domain` — the sub-domain (a `Domain` covering some points).
- `slice.proc` — target processor for this slice.
- `slice.recurse` — if `true`, re-slice on the target node (the target's mapper's `slice_task` will be invoked).
- `slice.stealable` — points in this slice may be stolen.

**Round-robin GPU example** (from `raw/website-pages/mapper.md`):
```cpp
void MyMapper::slice_task(const MapperContext ctx, const Task &task,
                          const SliceTaskInput &in, SliceTaskOutput &out) {
  Machine::ProcessorQuery pq(machine); pq.only_kind(Processor::TOC_PROC);
  std::vector<Processor> gpus(pq.begin(), pq.end());

  Rect<1> rect = in.domain;
  for (size_t i = 0; i < gpus.size(); i++) {
    Point<1> lo = rect.lo + (rect.volume() * i / gpus.size());
    Point<1> hi = rect.lo + (rect.volume() * (i + 1) / gpus.size()) - 1;
    if (i == gpus.size() - 1) hi = rect.hi;
    TaskSlice s; s.domain = Domain(Rect<1>(lo, hi)); s.proc = gpus[i];
    s.recurse = false; s.stealable = false;
    out.slices.push_back(s);
  }
}
```

**Recursive slicing**: setting `recurse = true` defers re-slicing to the target node. Useful when the launching node doesn't know the target's processor layout (heterogeneous clusters).

## Invariants
- The slices' domains must **cover the input domain exactly** — no missing points, no overlap. The runtime checks under `verify_correctness = true` (debug aid).
- Slices assigned to the same processor are **bundled**: per-point tasks within one slice still get individual `map_task` calls.
- `recurse = true` defers slicing; the target node's mapper sees its assigned sub-domain in a subsequent `slice_task` call.
- A `TaskSlice` whose processor is on a different node triggers Realm messaging at index-launch dispatch time.
- All callback non-blocking and non-reentrant rules apply (`mapper.md`).

## Performance implications
- **Load balance** is set here. Skewed slices = some processors idle while others queue work (visible in Legion Prof per-processor row activity).
- For data-parallel work, **`DefaultMapper`'s round-robin slicing** is usually good enough. Custom slicing matters when point-task cost is non-uniform.
- Recursive slicing adds an extra Realm message round-trip; only use it when the launching mapper genuinely lacks information about the target node.
- Under control replication, `slice_task` runs **per shard** for its share of the points (after the sharding functor has partitioned).

## Debug signals
- **Legion Prof**: an index launch's point tasks are spread across processor rows according to `slice_task`. Concentration on a few processors → bad slicing.
- **`LoggingWrapper`** records the `output.slices` for each `slice_task` call.
- **Slice-coverage errors** (debug build with `verify_correctness=true`): the runtime catches missing/overlapping slices.

## Failure modes
- Skewed slicing → load imbalance, visible in Legion Prof.
- Slices that don't cover the domain → runtime error under `verify_correctness=true`.

## Source pointers
- **Header**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion_mapping.h
- **Reference**: `raw/website-pages/mapper.md`
- **Tutorial**: `raw/tutorials/10_custom_mappers.md`

## Related
- `wiki/concepts/mapper.md` — host.
- `wiki/concepts/mapper-callback.md` — callback model.
- `wiki/concepts/mapper-context.md` — `ctx`.
- `wiki/concepts/index-space-launch.md` — the call's trigger.
- `wiki/concepts/map-task.md` — the next callback for each point.
- `wiki/concepts/sharding-functor.md` — the higher-level partitioning under replication.

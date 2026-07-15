---
title: Region Instance (Realm)
slug: region-instance
summary: Realm's primitive for an actual block of typed, laid-out memory; the substrate a Legion physical instance wraps.
tags: [data-model, memory, instances, for-program-reasoning]
subsystem: realm
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/realm_04_region_instances.md
  - raw/tutorials/realm_07_copies_and_fills.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/realm/instance.h
related:
  - wiki/concepts/physical-instance.md
  - wiki/concepts/event.md
  - wiki/concepts/dma-system.md
  - wiki/concepts/realm-machine-model.md
---

## TL;DR
A `RegionInstance` is Realm's allocated memory buffer: it sits in one specific `Memory`, has a fixed `InstanceLayoutGeneric` (affine SOA or AOS), and is reached via typed `AffineAccessor`/`MultiAffineAccessor` views. Creation is asynchronous — `create_instance` returns a Realm `Event` that triggers when the buffer is ready. Legion's `physical-instance.md` is the higher-level wrapper that adds privilege-aware accessors, equivalence-set tracking, and mapper integration around a `RegionInstance`. The confusion: a `RegionInstance` *cannot be moved*. To put the same data in a different memory you create a second instance and `copy` to it; the DMA system does the rest.

## Mental model
`RegionInstance` is `malloc`'s output for Realm — a typed buffer in a chosen pool (`Memory`) with a known layout. Where `malloc` returns a `void*`, `create_instance` returns a handle + an event. Once triggered, the handle's accessors are valid for reads/writes until you destroy the instance.

## Mechanism & API
```cpp
Event create_event = RegionInstance::create_instance(
    inst,                // out: handle filled in
    *memories.begin(),   // which Memory to allocate from
    bounds,              // index space (typed, 1..N-D)
    field_sizes,         // map<FieldID, size_t>
    /*AOS=*/1,           // or 0 for SOA
    ProfilingRequestSet());

// later, after create_event triggers:
AffineAccessor<int, 1> acc(inst, FID1);
acc[point] = 42;
```

Accessor kinds:
- **`AffineAccessor`** — single-piece, fixed-stride; fastest. Used when the instance covers one contiguous index-space rectangle.
- **`MultiAffineAccessor`** — multi-piece; k-d tree lookup, logarithmic by piece count, constant-time for single-piece instances.
- **`GenericAccessor`** — slowest, supports remote memory via Realm messages. Avoid on hot paths.

Layout knobs:
- AOS vs SOA controlled by the `block_size` argument (`1` for AOS, `0` for SOA).
- Field sizes are per-field in bytes via `std::map<FieldID, size_t>`.

Lifecycle:
- `RegionInstance::destroy(wait_on)` — asynchronous; the runtime collects the buffer once all in-flight operations have completed.

## Invariants
- A `RegionInstance` lives in **exactly one** `Memory` (`realm-machine-model.md`). It cannot be moved; only copies between instances exist.
- `create_instance` is **asynchronous**; reading/writing the instance before the returned event triggers is undefined behavior.
- The layout is **fixed at creation**. To change layout you destroy and re-create.
- Accessor types are **typed**: a mismatch between `AffineAccessor<float>` and a field stored as `int` is undefined behavior (no runtime check in release builds).
- An instance can hold **multiple fields**, each with its own size; the layout determines whether they're interleaved (AOS) or separated (SOA).

## Performance implications
- AOS layouts are convenient for "touch many fields per point" iterations; SOA layouts vectorize better and reduce cache pressure for "touch one field across all points".
- `AffineAccessor` is dramatically faster than `GenericAccessor`. Restructure to use `AffineAccessor` where possible.
- Cross-memory access uses `GenericAccessor` and triggers Realm messages per element — pathological for hot loops. Issue an explicit copy and use a local instance instead.
- Instance creation cost is dominated by the memory allocator (`realm-machine-model.md` describes the pool kinds). Allocators are per-`Memory` and can fragment; see `pitfalls/instance-fragmentation.md`.

## Debug signals
- **Legion Prof memory rows**: each instance is a slab on its memory row. Many short slabs = churn (see `pitfalls/instance-fragmentation.md`).
- **`-DTRACE_ALLOCATION`** + `-level allocation=2`: logs every instance alloc/free.
- **Out-of-memory from `create_instance`**: bump `-ll:csize` / `-ll:fsize` for the offending memory kind.

## Failure modes
- [Instance fragmentation](../pitfalls/instance-fragmentation.md)
- [Excessive data movement](../pitfalls/excessive-data-movement.md) — many `copy` ops because instances aren't reused.

## Source pointers
- **Realm header**: https://github.com/StanfordLegion/legion/blob/master/runtime/realm/instance.h
- **Tutorial**: https://legion.stanford.edu/tutorial/realm/region_instances.html (mirrored at `raw/tutorials/realm_04_region_instances.md`)

## Related
- `wiki/concepts/physical-instance.md` — the Legion wrapper layered on top.
- `wiki/concepts/event.md` — what `create_instance` returns.
- `wiki/concepts/dma-system.md` — moves data between region instances.
- `wiki/concepts/realm-machine-model.md` — the `Memory` and `Processor` namespace instances live in.

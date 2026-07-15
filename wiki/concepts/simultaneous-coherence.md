---
title: SIMULTANEOUS Coherence
slug: simultaneous-coherence
summary: A coherence mode permitting fully concurrent conflicting accesses; the application supplies explicit acquire/release synchronization around the shared region. Used for hand-rolled shared scratch buffers and message queues.
tags: [data-model, coherence, synchronization, for-program-reasoning]
subsystem: legion
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/07_privileges.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
related:
  - wiki/concepts/coherence-mode.md
  - wiki/concepts/atomic-coherence.md
  - wiki/concepts/acquire-release.md
  - wiki/concepts/region-requirement.md
---

## TL;DR
`SIMULTANEOUS` is the weakest "still-correct" coherence: any number of conflicting accesses may run concurrently with no runtime-imposed ordering. The application uses **explicit acquire/release operations** (`acquire-release.md`) to define synchronization windows where concurrent access is allowed. Standard for hand-rolled shared message buffers, lock-free scratch areas, and any pattern where the application has its own synchronization story and wants the runtime to step aside. The confusion: `SIMULTANEOUS` is not "atomic but more so" — it's a fundamentally different contract. The runtime issues no synchronization for conflicting `SIMULTANEOUS` requirements; everything is on the application.

## Mental model
`SIMULTANEOUS` is `relaxed` memory ordering, with `acquire-release` ops as the application's hand-managed fences. Where `atomic-coherence.md` says "trust me, each op is atomic", `SIMULTANEOUS` says "trust me, I'm doing my own synchronization with explicit acquire/release". The runtime steps fully out of the way for the region's coherence.

## Mechanism & API
```cpp
RegionRequirement(shared_lr, READ_WRITE, SIMULTANEOUS, shared_lr);
```

The application must bracket concurrent access regions with explicit operations:
```cpp
runtime->acquire_region(ctx, AcquireLauncher(shared_lr, shared_lr, ...));
// concurrent tasks with SIMULTANEOUS access to shared_lr execute here.
runtime->release_region(ctx, ReleaseLauncher(shared_lr, shared_lr, ...));
```

`acquire-release.md` covers the launchers and semantics in detail.

**Combined behavior** (per `raw/tutorials/07_privileges.md`):
- Two `SIMULTANEOUS` requirements on the same data: non-interfering at the Legion level — concurrent execution permitted.
- Mixed with `EXCLUSIVE`/`ATOMIC` requirements: the stronger wins on the pair.
- The application is fully responsible for happens-before relationships between concurrent simultaneous accesses.

## Invariants
- `SIMULTANEOUS` accesses **may run concurrently**, ordered only by application-level acquire/release pairs.
- The runtime does **not** insert any synchronization edges for `SIMULTANEOUS` requirements (within the brackets).
- `SIMULTANEOUS` requires application-managed synchronization for correctness — using it without `acquire`/`release` is undefined behavior.
- The runtime still tracks the underlying physical instance's state; tasks see a *consistent* (if not ordered) view of memory.
- `SIMULTANEOUS` is **a strict superset** of `ATOMIC` in terms of allowed concurrency; correspondingly stricter on what the application must guarantee.

## Performance implications
- **Useful for genuinely lock-free or hand-synchronized patterns** — work-stealing queues, message buffers, shared scratch space.
- For most workloads, **`REDUCE` or `ATOMIC` are simpler and just as fast**. Use `SIMULTANEOUS` only when those don't fit.
- The runtime overhead of `SIMULTANEOUS` is minimal — it's essentially "let it run" with the application taking on the work.
- The cost is shifted to the application's synchronization code; bugs there cause silent races.

## Debug signals
- **`dataflow-graph.md`**: `SIMULTANEOUS` pairs have no edge between them. Edges only appear at `acquire`/`release` boundaries.
- **`in-order-execution.md`** (`-lg:inorder`) serializes everything including `SIMULTANEOUS` requirements; results that change between `-lg:inorder` and parallel runs are nearly certain to be `SIMULTANEOUS` synchronization bugs.
- **Heavy `SIMULTANEOUS` use without explicit acquire/release pairs in mapper/spy logs** = application is relying on undefined behavior.

## Failure modes
- Using `SIMULTANEOUS` without `acquire`/`release` → undefined behavior (race or worse).
- Acquire/release misnested across regions → application-level synchronization bug; very hard to debug.

## Source pointers
- **Legion API**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Tutorial**: https://legion.stanford.edu/tutorial/privileges.html

## Related
- `wiki/concepts/coherence-mode.md` — umbrella.
- `wiki/concepts/atomic-coherence.md` — the next-strongest mode.
- `wiki/concepts/acquire-release.md` — the required synchronization machinery.
- `wiki/concepts/region-requirement.md` — where coherence is set.

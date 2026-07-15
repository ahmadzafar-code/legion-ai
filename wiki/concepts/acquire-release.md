---
title: Acquire / Release
slug: acquire-release
summary: Explicit synchronization operations bracketing concurrent access to a SIMULTANEOUS-coherence region; the application's mechanism for defining happens-before relationships when the runtime steps out of the way.
tags: [data-model, coherence, synchronization, for-program-reasoning, for-correctness-debug]
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
  - wiki/concepts/simultaneous-coherence.md
  - wiki/concepts/coherence-mode.md
  - wiki/concepts/reservation.md
  - wiki/concepts/region-requirement.md
---

## TL;DR
`acquire_region` and `release_region` are runtime operations the application issues to bracket a window during which a region with `simultaneous-coherence.md` may be accessed concurrently. Inside the bracket, the runtime does not synchronize between conflicting requirements; outside the bracket, conflicting requirements are ordered by program issue order. The confusion: `acquire`/`release` are **not** for mutex-style mutual exclusion (`reservation.md` is). They mark *when* the runtime should and shouldn't enforce coherence — open the gates, do concurrent work, close the gates.

## Mental model
`acquire`/`release` are the *transaction boundaries* for a SIMULTANEOUS region. Before the acquire, the region's state is committed (ordered, coherent). During the acquire-release window, tasks read and write concurrently with application-managed synchronization. After the release, the region's state is committed again, observed by subsequent (ordered) operations.

In C++-atomic terms: acquire is "fence: prior ops happen-before everything in the bracket", release is "fence: everything in the bracket happens-before subsequent ops".

## Mechanism & API
```cpp
// Region declared with SIMULTANEOUS coherence:
RegionRequirement shared_req(shared_lr, READ_WRITE, SIMULTANEOUS, shared_lr);

// Open the concurrent-access window.
AcquireLauncher acquire(shared_lr, shared_lr);
acquire.add_field(FID_X);
runtime->acquire_region(ctx, acquire);

// Now launch concurrent tasks with SIMULTANEOUS-coherence requirements
// on shared_lr. Their access ordering is the application's responsibility.

// Close the window.
ReleaseLauncher release(shared_lr, shared_lr);
release.add_field(FID_X);
runtime->release_region(ctx, release);

// After release, subsequent SIMULTANEOUS or EXCLUSIVE accesses to shared_lr
// observe a coherent state again.
```

Inside the bracket, the application typically uses:
- A `reservation.md` to provide mutual exclusion when needed.
- Atomic CAS in the task body via `__sync_*` intrinsics or similar.
- A hand-rolled lock-free protocol.

The runtime's role is just to honor the bracket: it does not order operations inside, but it does flush state at the boundaries.

## Invariants
- `acquire_region` and `release_region` come in **matched pairs**. An unreleased acquire holds the region in concurrent-access mode indefinitely.
- Tasks launched between an acquire and release see `simultaneous-coherence.md` semantics for the bracketed region.
- Tasks launched outside the bracket see `exclusive-coherence.md` semantics — the runtime resumes synchronization.
- The application's per-task synchronization (reservations, atomics) is independent of the acquire/release brackets — they layer.
- Brackets may **nest** in some patterns (region of region), but the standard pattern is one bracket per concurrent-access phase.

## Performance implications
- The overhead of `acquire`/`release` themselves is small — they're operations in the pipeline like any other.
- The win is allowing concurrent execution of conflicting tasks during the bracket.
- For most applications, simpler patterns (`REDUCE` + reduction instance, `ATOMIC` + reservation) suffice. `acquire`/`release` shines when the application has a custom synchronization protocol it wants the runtime to step out of.
- Misuse (forgetting the release, or releasing too early) produces deadlocks or races; debug-cycle time is high.

## Debug signals
- **`dataflow-graph.md`**: `acquire_region` and `release_region` show up as their own operation nodes; the bracket they create is the region between them.
- **Forgotten release**: subsequent operations on the region hang forever waiting. Diagnose via `REALM_SHOW_EVENT_WAITERS` (see `freeze-on-error.md`).
- **Synchronization bugs inside the bracket**: race conditions visible under heavy concurrency, disappearing under `in-order-execution.md` (`-lg:inorder`).

## Failure modes
- Forgetting `release_region` → indefinite hang.
- Mismatched bracket (acquire without release, or vice versa) → runtime error or hang.
- Wrong field set on the launcher → partial bracket; mixed-coherence behavior on the un-bracketed fields.

## Source pointers
- **Legion API**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Tutorial**: https://legion.stanford.edu/tutorial/privileges.html

## Related
- `wiki/concepts/simultaneous-coherence.md` — the coherence mode this synchronizes.
- `wiki/concepts/coherence-mode.md` — umbrella.
- `wiki/concepts/reservation.md` — typical companion for per-op atomicity inside the bracket.
- `wiki/concepts/region-requirement.md` — where `SIMULTANEOUS` is declared.

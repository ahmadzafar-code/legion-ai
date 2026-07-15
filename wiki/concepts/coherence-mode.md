---
title: Coherence Mode
slug: coherence-mode
summary: A per-region-requirement modifier (EXCLUSIVE, ATOMIC, SIMULTANEOUS, RELAXED) that tells the runtime how strict the ordering between conflicting accesses on the same data must be.
tags: [data-model, coherence, for-program-reasoning, for-correctness-debug]
subsystem: legion
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/07_privileges.md
  - raw/publications/publications.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
related:
  - wiki/concepts/privilege.md
  - wiki/concepts/logical-region.md
  - wiki/concepts/dependence-analysis.md
  - wiki/concepts/reservation.md
  - wiki/concepts/region-requirement.md
  - wiki/concepts/exclusive-coherence.md
  - wiki/concepts/atomic-coherence.md
  - wiki/concepts/simultaneous-coherence.md
  - wiki/concepts/relaxed-coherence.md
  - wiki/concepts/acquire-release.md
---

## TL;DR
A coherence mode pairs with a privilege on every region requirement and controls how strictly the runtime serializes conflicting accesses. Four values: `EXCLUSIVE` (default, full program-order serialization), `ATOMIC` (concurrent OK if the application enforces atomicity), `SIMULTANEOUS` (truly concurrent with explicit `acquire`/`release` synchronization), `RELAXED` (weakest, no ordering guarantee). The confusion: most Legion programs only ever use `EXCLUSIVE`; the other modes exist to escape Legion's sequential semantics for specific patterns (locks, atomic counters, message buffers) where the application has its own synchronization story.

## Mental model
Coherence modes are to Legion privileges what memory-ordering attributes (`relaxed`/`acquire`/`release`/`seq_cst`) are to atomic operations in C++. `EXCLUSIVE` is "the runtime gives you sequential-program semantics"; the others progressively relax that contract in exchange for parallelism. The runtime trusts the application's choice — if you say `SIMULTANEOUS`, you take on the synchronization responsibility.

## Mechanism & API
Coherence is the third constructor argument of `RegionRequirement`:
```cpp
RegionRequirement(lr, READ_WRITE, EXCLUSIVE, lr);
RegionRequirement(lr, READ_WRITE, ATOMIC,    lr);
RegionRequirement(lr, READ_WRITE, SIMULTANEOUS, lr);
```

- **`EXCLUSIVE`** — default. Conflicting accesses are fully serialized in program order; this is what gives Legion its sequential-program illusion.
- **`ATOMIC`** — concurrent conflicting accesses are allowed; the application guarantees individual operations are atomic (e.g., backed by hardware atomics or a `Reservation`). Useful for atomic counters and lock-protected data.
- **`SIMULTANEOUS`** — fully concurrent; the application synchronizes explicitly via `acquire`/`release` operations on the region. Typical for hand-rolled message queues or shared scratch buffers.
- **`RELAXED`** — weakest; no ordering between conflicting accesses, no application-level guarantee. Rarely used; primarily for output regions where the result is intentionally non-deterministic.

`SIMULTANEOUS` access patterns rely on:
```cpp
runtime->acquire_region(ctx, AcquireLauncher(...));
// concurrent tasks run here
runtime->release_region(ctx, ReleaseLauncher(...));
```

Non-interference rules: two `READ_ONLY` requirements with any coherence are non-interfering. Two `REDUCE` with the same operator are non-interfering. Coherence weaker than `EXCLUSIVE` widens the non-interference predicate at the cost of serialization guarantees.

## Invariants
- Coherence weakens only the *ordering* contract; it does not weaken the *privilege* contract — `READ_ONLY ATOMIC` still cannot write.
- The default is `EXCLUSIVE`; you have to opt out explicitly.
- Two requirements with different coherence modes on overlapping data: the runtime uses the *stronger* of the two for the conflict (EXCLUSIVE wins).
- `SIMULTANEOUS` requires the application to issue `acquire`/`release` operations; using `SIMULTANEOUS` without them is undefined behavior.
- `RELAXED` makes the result intentionally non-deterministic; do not use unless determinism is genuinely irrelevant.

## Performance implications
- Stick with `EXCLUSIVE` until you have a specific reason. Weakening coherence only helps when the program's true semantics permit concurrency that the runtime's default analysis blocks.
- `ATOMIC` + a `Reservation` is the standard pattern for shared counters in Legion; using `EXCLUSIVE` there serializes everything pointlessly.
- See paper `visibility2023.pdf` (Visibility Algorithms for Dynamic Dependence Analysis and Distributed Coherence) for the implementation cost of coherence enforcement at scale.

## Debug signals
- **Legion Spy** dataflow graph: edges between two tasks reflect the effective coherence; a missing edge where you expected one is usually `ATOMIC` or `SIMULTANEOUS` without proper synchronization.
- **Race conditions** in production but not in `-lg:inorder` runs almost always involve `ATOMIC`/`SIMULTANEOUS` plus missing application-level synchronization.

## Failure modes
- `SIMULTANEOUS` without `acquire`/`release` → data race.
- `RELAXED` used inadvertently → non-deterministic results.

## Source pointers
- **Legion API**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Tutorial (privileges + coherence)**: https://legion.stanford.edu/tutorial/privileges.html
- **Paper (visibility/coherence)**: `raw/publications/pdfs/visibility2023.pdf`

## Related
- `wiki/concepts/privilege.md` — coherence pairs with privilege on every region requirement.
- `wiki/concepts/logical-region.md` — what is being accessed.
- `wiki/concepts/dependence-analysis.md` — where coherence is consumed.

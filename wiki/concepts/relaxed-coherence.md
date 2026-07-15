---
title: RELAXED Coherence
slug: relaxed-coherence
summary: The weakest coherence mode; concurrent conflicting accesses allowed with no synchronization guarantee from the runtime or the application; intentional non-determinism for cases where the result is order-independent.
tags: [data-model, coherence, for-program-reasoning]
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
  - wiki/concepts/simultaneous-coherence.md
  - wiki/concepts/atomic-coherence.md
  - wiki/concepts/region-requirement.md
---

## TL;DR
`RELAXED` is the weakest coherence mode: concurrent conflicting accesses are permitted with **no synchronization at all** — neither from the runtime nor (by convention) from the application. The result is intentionally non-deterministic. Used rarely — typically for output regions where the final ordering doesn't matter (e.g., logging streams) or for explicit max/min-effort accumulators where wrong intermediate values are acceptable. The confusion: `RELAXED` is **not** "fast" — it doesn't save runtime work compared to `SIMULTANEOUS`. It just signals "I do not care about ordering at all", which is rarely what you actually want.

## Mental model
`RELAXED` is `memory_order_relaxed` in C++ atomics: no synchronization, no ordering, no guarantee except that individual writes don't tear. The runtime is freed from any obligation to order accesses, and the application has explicitly accepted non-determinism. For most programs this is a footgun; for the right pattern (idempotent overwrites, logging) it's the right tool.

## Mechanism & API
```cpp
RegionRequirement(logging_lr, READ_WRITE, RELAXED, logging_lr);
```

The runtime issues no dependence edges between `RELAXED` requirements on the same data. The application explicitly accepts that:
- Writes may happen in any order.
- Reads may see any prior write (or a mix of bytes from multiple writes, in pathological hardware cases).
- Different runs may produce different results.

`RELAXED` is **rarely** the right choice — most "I don't care about ordering" patterns actually want `REDUCE` (with a max or sum operator), `ATOMIC` (with a reservation), or `SIMULTANEOUS` (with explicit acquire/release).

## Invariants
- `RELAXED` accesses have **no ordering guarantee** from any source.
- The runtime issues **no synchronization** for conflicting `RELAXED` requirements.
- The application is **not required** to provide its own synchronization (unlike `SIMULTANEOUS`), but the result is correspondingly less defined.
- Mixed with stronger coherence on the same data: the stronger wins on the pair.
- A program correct under `RELAXED` is correct under any stronger coherence (assuming the program doesn't depend on the non-determinism `RELAXED` provides).

## Performance implications
- `RELAXED` saves **no runtime cost** vs. `SIMULTANEOUS` — both omit synchronization. The choice is about *contract*, not *speed*.
- Most "perf-motivated" uses of `RELAXED` would be better served by `REDUCE` privilege (`reduce-privilege.md`) with the appropriate operator. Reductions are fast AND deterministic.
- For genuinely-order-independent patterns (e.g., "write any one of these values to this cell, I don't care which"), `RELAXED` is the right declaration.

## Debug signals
- **Non-deterministic results across runs** of code using `RELAXED` are *expected*, not bugs. Confirm via `in-order-execution.md` (`-lg:inorder`): results should be reproducible there.
- **`dataflow-graph.md`** shows no edges between `RELAXED` pairs.
- **Subtle bugs that "almost work"**: data structures whose validity depends on order but were declared `RELAXED`. Hard to catch; rely on code review.

## Failure modes
- Choosing `RELAXED` when the program actually needs ordering → non-deterministic wrong answers.
- Using `RELAXED` as a perf optimization when `REDUCE` would have been correct and faster.
- Hard-to-reproduce bugs in `RELAXED`-using code; the non-determinism makes them resist standard debugging.

## Source pointers
- **Legion API**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Tutorial**: https://legion.stanford.edu/tutorial/privileges.html

## Related
- `wiki/concepts/coherence-mode.md` — umbrella.
- `wiki/concepts/simultaneous-coherence.md` — the next-stronger mode.
- `wiki/concepts/atomic-coherence.md` — the explicit-atomicity sibling.
- `wiki/concepts/region-requirement.md` — where coherence is set.

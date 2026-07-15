---
title: ATOMIC Coherence
slug: atomic-coherence
summary: A coherence mode permitting concurrent conflicting accesses provided the application guarantees per-operation atomicity (typically via a `Reservation`); enables atomic-counter patterns and lock-protected shared state.
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
  - wiki/concepts/exclusive-coherence.md
  - wiki/concepts/reservation.md
  - wiki/concepts/privilege.md
  - wiki/concepts/region-requirement.md
---

## TL;DR
`ATOMIC` coherence tells the runtime "concurrent conflicting accesses on this region are OK — the application guarantees each operation is atomic by some means I take responsibility for". The standard means is to pair it with a `reservation.md`: tasks acquire the reservation, perform their atomic op, release. The confusion: `ATOMIC` does not make the access atomic — the application's synchronization does. `ATOMIC` only widens the runtime's non-interference predicate so it stops serializing the operations.

## Mental model
`ATOMIC` is `relaxed` ordering with application-side compare-exchange. The runtime stops trying to order conflicting accesses; instead, the application takes a lock (`reservation.md`) around each operation. Useful for shared counters, accumulators, work queues — patterns where many tasks update one data structure and contention is genuine but mutual exclusion is cheap.

## Mechanism & API
```cpp
RegionRequirement(shared_lr, READ_WRITE, ATOMIC, shared_lr);
```

The application's responsibility is to wrap each task's access with a `reservation.md`:
```cpp
Reservation res = Reservation::create_reservation();
// In each contributor task:
Event acquired = res.acquire(0, true, prev_event);
Event done = runtime->execute_task(ctx, my_atomic_op_launcher_with_atomic_coherence,
                                   /*precondition=*/acquired);
res.release(done);
```

The reservation provides the atomicity; `ATOMIC` coherence tells the runtime not to add its own serialization edges on top.

**Combined behavior** (per `raw/tutorials/07_privileges.md`):
- Two `READ_ONLY` + `ATOMIC` requirements: non-interfering (as with `EXCLUSIVE`).
- Two `READ_WRITE` + `ATOMIC` requirements on the same data: **non-interfering at the Legion level** (the runtime trusts the application's per-operation atomicity).
- One `READ_WRITE` + `ATOMIC` and one `READ_WRITE` + `EXCLUSIVE` on the same data: the stronger wins → `EXCLUSIVE` semantics; runtime serializes.

## Invariants
- `ATOMIC` only meaningfully differs from `EXCLUSIVE` for **conflicting** accesses; non-conflicting pairs are non-interfering under both.
- The runtime **trusts** the application's atomicity guarantee. If the application's individual operations aren't actually atomic, the result is undefined.
- `ATOMIC` is **not transitive**: just because A and B can run concurrently doesn't mean A+B+C+D form a consistent state at the end; the application's lock pattern must enforce that.
- The runtime does **not** insert dependence edges between `ATOMIC` pairs — visible in `dataflow-graph.md` as the *absence* of edges where `EXCLUSIVE` would have them.
- `ATOMIC` is independent of privilege: works with `READ_WRITE`, `WRITE_DISCARD`, etc. The privilege controls what the task does; coherence controls how the runtime orders it.

## Performance implications
- **The canonical pattern for shared counters / accumulators** when `REDUCE` doesn't fit (e.g., the update isn't commutative-associative or you want side-effecting behavior).
- The cost is the reservation acquire/release — bounded but non-zero, especially across nodes.
- Versus `REDUCE` privilege: `REDUCE` is faster when the update is commutative-associative, since the runtime can use reduction instances and tree-fold. Use `ATOMIC` when you need ordered or non-commutative atomic updates.

## Debug signals
- **`dataflow-graph.md`**: an absent edge between two same-region `ATOMIC` requirements confirms the widened non-interference. If you see edges, check whether one requirement is `EXCLUSIVE`.
- **Race conditions** in production but not under `in-order-execution.md` → `ATOMIC` requirements likely missing the corresponding `reservation.md` synchronization.
- **Reservation deadlocks** (the application's atomicity-providing locks) show up via `REALM_SHOW_EVENT_WAITERS` + `tools/detect_loops` (see `freeze-on-error.md` workflow).

## Failure modes
- `ATOMIC` requirements without application-level atomicity → data race.
- Inconsistent reservation acquisition order → deadlock.

## Source pointers
- **Legion API**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Tutorial**: https://legion.stanford.edu/tutorial/privileges.html

## Related
- `wiki/concepts/coherence-mode.md` — umbrella.
- `wiki/concepts/exclusive-coherence.md` — the default `ATOMIC` deviates from.
- `wiki/concepts/reservation.md` — the typical atomicity-provider.
- `wiki/concepts/privilege.md` — orthogonal to coherence.
- `wiki/concepts/region-requirement.md` — where coherence is set.

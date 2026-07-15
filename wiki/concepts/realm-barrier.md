---
title: Realm Barrier
slug: realm-barrier
summary: A multi-generation, count-arriving variant of a Realm Event; created with a participant count, advances phase only after that many `arrive()` calls. Supports built-in reductions across the arrivals.
tags: [synchronization, for-program-reasoning, for-perf-debug]
subsystem: realm
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/realm_12_barriers.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/realm/event.h
related:
  - wiki/concepts/event.md
  - wiki/concepts/user-event.md
  - wiki/concepts/reduction-instance.md
  - wiki/concepts/freeze-on-error.md
---

## TL;DR
A `Barrier` is a Realm `Event` variant that triggers only after a specified **number of arrivals**. Created with `Barrier::create_barrier(N)`, it triggers once `N` calls to `arrive()` have occurred. Critically, barriers have **multiple generations** — after triggering, `advance_barrier()` returns a fresh barrier for the next phase, letting the application reuse the same logical barrier for iterative synchronization (training loop, time-step, producer/consumer round-robin). Barriers can also incorporate a **reduction operator** so the arrivers collectively compute a value visible to the waiters. The confusion: barriers are typed `Event`, not a separate primitive — but they cannot be triggered by `trigger()` (which Events have); they trigger via `arrive()`.

## Mental model
A Realm barrier is `pthread_barrier_t` in distributed-async form, with two upgrades: (1) **phases** — you can advance to the next iteration without destroying and recreating the barrier; (2) **reductions** — the arrivers can carry a value and the barrier blends them under a registered operator (sum, min, max, ...). For iterative producer/consumer patterns where N writers produce a value and one reader reads the merged result every iteration, barriers are the right primitive.

## Mechanism & API

**Create a barrier with an expected arrival count**:
```cpp
Barrier reader_barrier = Barrier::create_barrier(num_readers);
```

**With a reduction** (combines arrival payloads):
```cpp
rt.register_reduction<ReductionOpIntAdd>(REDOP_ADD);
int init_value = 0;
Barrier writer_barrier = Barrier::create_barrier(
    num_writers, REDOP_ADD, &init_value, sizeof(init_value));
```

**Arrive (decrement the count)**:
```cpp
// Plain arrival:
writer_b.arrive(/*count=*/1, /*precondition=*/Event::NO_EVENT);

// Arrival with reduction value:
int my_contribution = 42;
writer_b.arrive(1, Event::NO_EVENT, &my_contribution, sizeof(my_contribution));
```

**Wait** (same as any Event):
```cpp
writer_b.wait();  // blocks until the barrier triggers (all arrived)
```

**Retrieve a reduction result** (after triggering):
```cpp
int result = 0;
bool ready = writer_b.get_result(&result, sizeof(result));
```

**Advance to the next phase**:
```cpp
writer_b = writer_b.advance_barrier();
reader_b = reader_b.advance_barrier();
// Now use writer_b / reader_b for the next iteration.
```

The advance produces a **fresh barrier handle** for the next phase. The old phase's event is still observable (it stays triggered) but new arrivals should target the new handle.

## Invariants
- A barrier triggers exactly once per phase, when its arrival count reaches zero.
- Phases must trigger **sequentially** — skipping a phase (e.g., advancing twice without arriving) causes deadlock. Per `raw/tutorials/realm_12_barriers.md`: "phases must trigger sequentially".
- A barrier has a maximum phase count (`MAX_PHASES`); exceeding it returns `NO_BARRIER`. Long-running iterative codes that consume phases must recreate the barrier periodically.
- Barrier destruction requires `barrier.destroy()` after the barrier is no longer needed (unlike normal events, which are auto-collected).
- A reduction barrier's `get_result` is valid only after the barrier triggers; calling it before returns `false`.

## Performance implications
- Barriers are **distributed primitives** — they work across nodes. The home node coordinates the arrival count; remote arrivers send active messages.
- For tight loops, the per-iteration cost of `arrive` + `advance` + `wait` is small but non-zero. Consider whether `event.md` chaining or a `completion-queue.md` is simpler.
- Reduction barriers fuse synchronization with a small computation — cheaper than a separate `Reservation` + manual sum.
- **Processor placement matters**: the tutorial shows 4 writer tasks on 1 processor execute sequentially (no benefit from barrier); on 4 separate processors they execute in parallel (full barrier speedup). Always match arrival count to actual processor count.

## Debug signals
- **`REALM_SHOW_EVENT_WAITERS=60+5`** dumps barriers along with regular events; an untriggered barrier visible in the dump indicates an arrival count mismatch.
- **Application hangs at an `advance_barrier()`** → typically a missed arrival, or a phase-skip bug.
- **`-level barrier=2`** logs per-arrival / per-phase events (when present in your Legion build).

## Failure modes
- Arrival count > expected → extra arrivals overflow into the next phase; surprising downstream behavior.
- Arrival count < expected → barrier never triggers; hang detectable via `REALM_SHOW_EVENT_WAITERS`.
- Phase skipping (advancing twice without arriving in between) → deadlock.
- Reusing a destroyed barrier → undefined behavior.

## Source pointers
- **Realm header**: https://github.com/StanfordLegion/legion/blob/master/runtime/realm/event.h
- **Tutorial**: `raw/tutorials/realm_12_barriers.md`

## Related
- `wiki/concepts/event.md` — base type; barriers are an Event variant.
- `wiki/concepts/user-event.md` — sibling event variant with manual trigger (single-shot).
- `wiki/concepts/reduction-instance.md` — alternative for accumulator patterns at higher granularity.
- `wiki/concepts/freeze-on-error.md` — debug aid for barrier-related hangs.

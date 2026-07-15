---
title: Reservation
slug: reservation
summary: Realm's non-blocking, distributed-aware atomicity primitive; supports exclusive and shared acquisition modes via deferred-execution `acquire`/`release` returning events.
tags: [synchronization, distributed, for-program-reasoning, for-correctness-debug]
subsystem: realm
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/realm_11_reservations.md
github:
  - https://github.com/StanfordLegion/legion/tree/master/runtime/realm
related:
  - wiki/concepts/event.md
  - wiki/concepts/coherence-mode.md
  - wiki/concepts/realm-machine-model.md
---

## TL;DR
A `Reservation` is Realm's lock primitive: `acquire(mode, exclusive, wait_on)` returns an `Event` that triggers once the reservation is granted; `release(event)` returns it once `event` triggers. Unlike a traditional mutex, neither call blocks the caller — execution continues, and the reservation participates in the Realm event graph. Reservations are **distributed-aware**: they work across nodes and serve as global locks. The confusion: reservations are non-blocking by design; you express the lock pattern by feeding the `acquire` event into your task's `spawn` precondition, then `release` once the task's completion event fires.

## Mental model
A `Reservation` is the deferred-execution sibling of `std::mutex`. Where `mutex.lock()` blocks the calling thread until acquired, `reservation.acquire()` returns a future-like event you splice into the precondition of your protected task. The runtime grants when ready; downstream operations execute when the grant event triggers. This composes cleanly with `event.md` — locking is just another edge in the event DAG.

## Mechanism & API
```cpp
Reservation res = Reservation::create_reservation();

// Exclusive (writer) acquisition:
Event acquired = res.acquire(/*mode=*/0, /*exclusive=*/true, start_event);
Event done = p.spawn(WRITER_TASK, &args, sizeof(args), acquired);
res.release(done);

// Shared (reader) acquisition — different non-zero mode value:
Event a = res.acquire(/*mode=*/1, /*exclusive=*/false, prev_event);
Event d = p.spawn(READER_TASK, &args, sizeof(args), a);
res.release(d);
```

Three things to notice:
1. `acquire(mode, exclusive, wait_on)` is the API. `mode = 0` is the conventional exclusive mode; non-zero modes can be used for shared acquisition. (The mode argument names a "type" of access; multiple holders of the same non-exclusive mode are compatible; switching modes requires the prior mode to drain.)
2. The acquire event flows into the protected task's spawn precondition.
3. `release` takes the *completion* event of the protected task, so the runtime releases automatically once the work is done.

Distributed behavior: reservations work across nodes. The Realm runtime forwards `acquire` requests to the home node of the reservation and granted events propagate back. This makes them usable as global locks.

## Invariants
- `acquire` and `release` are **non-blocking**: they return events; the caller continues immediately.
- A reservation is granted **once all preconditions trigger** AND **the current holder(s) release**.
- Exclusive mode (`exclusive=true`) requires no other holder; only the protected task runs while the reservation is held.
- Shared mode (`exclusive=false`) allows multiple holders **of the same mode value** simultaneously.
- **Realm is not obligated to grant in any specific order.** First-acquire-first-serve is not guaranteed.
- A reservation can only be held by **one node at a time** in shared mode — shared acquisition on different nodes is **not** supported.
- Multiple reservations acquired in an inconsistent order can deadlock, just like ordinary mutexes.

## Performance implications
- Reservations are coarse-grained; for hot per-element atomicity, use `REDUCE` privileges (`coherence-mode.md` + reduction instances).
- A shared-mode reservation lets many readers coexist with one writer.
- Forwarding cost is small but non-zero; cross-node `acquire` involves an active message round-trip.
- A long-held exclusive reservation serializes everything downstream — visible in Legion Prof as a long bar with downstream tasks queued behind it.

## Debug signals
- **`REALM_SHOW_EVENT_WAITERS=N+M`**: dumps event waiters; reservation-held events appear when a deadlock has formed.
- **`tools/detect_loops`** processes the event dump to find cycles — most reservation deadlocks show up here as a cycle.
- **Legion Prof**: a writer holding a reservation exclusively appears as a single colored bar with successor tasks queued behind it.
- **`-level reservation=2`**: logs reservation grants and releases.

## Failure modes
- Inconsistent reservation order → deadlock. Use `REALM_SHOW_EVENT_WAITERS` + `detect_loops` to debug.
- Shared-mode acquisition on different nodes → not supported; symptom is a hang. Move the work onto a single node or use a different synchronization pattern.

## Source pointers
- **Realm runtime**: https://github.com/StanfordLegion/legion/tree/master/runtime/realm
- **Tutorial**: https://legion.stanford.edu/tutorial/realm/reservation.html (mirrored at `raw/tutorials/realm_11_reservations.md`)
- **Paper (Realm)**: `raw/publications/pdfs/realm2014.pdf`

## Related
- `wiki/concepts/event.md` — what `acquire` and `release` produce.
- `wiki/concepts/coherence-mode.md` — `ATOMIC` coherence is typically backed by a reservation in the application.
- `wiki/concepts/realm-machine-model.md` — reservations have a "home node".

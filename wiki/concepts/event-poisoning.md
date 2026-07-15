---
title: Event Poisoning
slug: event-poisoning
summary: How Realm propagates failures through its event graph; a "poisoned" event signals that its producer encountered an error, and the poison cascades to all dependent operations unless consumers explicitly check for it.
tags: [synchronization, errors, for-correctness-debug]
subsystem: realm
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/realm_03_events.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/realm/event.h
related:
  - wiki/concepts/event.md
  - wiki/concepts/user-event.md
  - wiki/concepts/freeze-on-error.md
  - wiki/concepts/backtrace-mode.md
  - wiki/concepts/error-message-catalog.md
---

## TL;DR
An event in Realm has three possible states: **untriggered**, **triggered** (success), or **poisoned** (failure). A poisoned event signals that the operation producing it raised an error — a task crashed, an assertion failed, an instance allocation aborted. The poison **cascades automatically**: any operation that waited on a poisoned precondition becomes poisoned itself. Consumers can detect poison via `Event::has_triggered_faultaware()` rather than the standard `has_triggered()`. The confusion: by default, most application code uses `has_triggered()` and treats triggering as success — poison shows up as "task didn't run because its precondition failed" without any explicit handling. For correctness debugging, you need the fault-aware path.

## Mental model
Event poisoning is exception propagation for asynchronous Realm operations. Where C++ exceptions propagate up the call stack until caught, poison propagates forward through the event DAG until a consumer explicitly handles it. Most Realm/Legion code is "exception-unaware" — the application assumes operations succeed and reads the error via `LEGION_BACKTRACE=1` or `LEGION_FREEZE_ON_ERROR=1` after the runtime gives up. Fault-aware code can intercept poison and recover.

## Mechanism & API
A Realm `Event` has state distinguishing untriggered / triggered / poisoned (per `raw/tutorials/realm_03_events.md`):

```cpp
Event e = p.spawn(SOME_TASK, &args, sizeof(args));

// Standard: wait for trigger, treat trigger as success.
e.wait();  // returns when e is triggered OR poisoned; doesn't distinguish

// Fault-aware: check whether the event triggered successfully.
bool poisoned = false;
e.wait_faultaware(poisoned);
if (poisoned) {
  // Handle the failure — log, retry, abort, etc.
}

// Test without waiting:
if (e.has_triggered_faultaware(poisoned)) {
  // e has resolved (success or failure); poisoned tells which
}
```

**Cascading**: when an operation's precondition event is poisoned, the operation **does not run**; its own output event becomes poisoned. The poison flows forward through the event graph automatically.

**Sources of poison**:
- Task assertion failure or unhandled C++ exception in the body.
- Realm-side errors during op preparation (e.g., instance allocation failure).
- Explicit poison-triggering of a `user-event.md` via `UserEvent::trigger(precondition, /*poisoned=*/true)`.
- Cascading from any upstream poisoned event.

**Recovery patterns** (rare in Legion application code; more common in low-level Realm code):
- Catch the poison at a fault-aware checkpoint, log diagnostics, decide to abort or substitute fallback output.
- Re-launch the operation with retry semantics if the failure was transient.

By default Legion's runtime treats poisoned events as fatal — when the runtime's bookkeeping sees a poisoned event it should never observe, it prints diagnostics (assisted by `LEGION_BACKTRACE=1`) and freezes (`LEGION_FREEZE_ON_ERROR=1`) if configured.

## Invariants
- An event's poison state is **terminal** — once poisoned, it stays poisoned.
- Poison **cascades along event-DAG edges** automatically. Consumers don't need to opt in for the cascade to happen.
- The standard `wait()` and `has_triggered()` APIs are **not fault-aware** — they treat poison as if it were a trigger. Use the `_faultaware` variants when recovery is intended.
- Realm guarantees event handles remain valid in the poisoned state; the handle isn't reclaimed prematurely.
- A `user-event.md` can be intentionally poisoned via `UserEvent::trigger(precondition, /*poisoned=*/true)` — useful for signaling failure from an application-level error path.

## Performance implications
- Event-poisoning adds **no overhead** to the happy path — the state machine handles untriggered/triggered/poisoned with the same primitives.
- Fault-aware checks (`has_triggered_faultaware`) are roughly the same cost as non-fault-aware checks.
- Pervasive fault-aware code would clutter the application; most Legion programs leave fault handling to the runtime's default termination path.

## Debug signals
- **Application terminates abruptly** with `LEGION_BACKTRACE=1` showing a stack and an error message → the most common visible form of poisoning.
- **`REALM_SHOW_EVENT_WAITERS=60+5`** dumps pending events; poisoned events show their state in the dump.
- **An entire subgraph of operations didn't run** despite producers appearing to complete → upstream operation was poisoned; cascade reached the subgraph.
- **`LEGION_FREEZE_ON_ERROR=1`** combined with backtrace pinpoints which task body raised the underlying error.

## Failure modes
- Recovering from poison via `_faultaware` but not actually fixing the underlying error → infinite retry loops.
- Using `wait()` instead of `wait_faultaware()` when you intended to handle poison → silent absorption of failure; debugging much harder.

## Source pointers
- **Realm header**: https://github.com/StanfordLegion/legion/blob/master/runtime/realm/event.h
- **Tutorial**: `raw/tutorials/realm_03_events.md` (events basics; mentions the three-state model)

## Related
- `wiki/concepts/event.md` — base type whose state machine includes poisoning.
- `wiki/concepts/user-event.md` — can be intentionally poisoned.
- `wiki/concepts/freeze-on-error.md` — the runtime's default response to unhandled poison.
- `wiki/concepts/backtrace-mode.md` — companion for diagnosing the underlying task error.
- `wiki/concepts/error-message-catalog.md` — many error codes manifest as event poison.

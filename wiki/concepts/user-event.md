---
title: User Event
slug: user-event
summary: An application-triggered Realm event; created untriggered via `UserEvent::create_user_event()` and triggered explicitly via `trigger()`. Standard for hand-built synchronization patterns and for waking profiling-result waiters.
tags: [synchronization, for-correctness-debug, for-program-reasoning]
subsystem: realm
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/realm_03_events.md
  - raw/website-pages/debugging.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/realm/event.h
related:
  - wiki/concepts/event.md
  - wiki/concepts/event-poisoning.md
  - wiki/concepts/realm-profiling.md
  - wiki/concepts/freeze-on-error.md
---

## TL;DR
A `UserEvent` is a Realm `Event` the application creates and triggers itself. `UserEvent::create_user_event()` returns one in the untriggered state; later, `user_event.trigger()` (or `trigger(precondition_event)`) fires it. Used wherever the runtime's automatic event production doesn't fit — hand-rolled synchronization, profiling-result wake-ups, custom message-passing protocols. The confusion: a `UserEvent` is convertible to a normal `Event` and passable to any API that takes an Event. The "user" prefix just means the *trigger* is application-controlled rather than runtime-controlled.

## Mental model
A user event is `std::promise` paired with `std::future` for Legion's event substrate — create the promise (untriggered event), pass the future (event handle) around, fulfill the promise (trigger) when ready. Where the JS equivalent is `new Promise((resolve) => ...); resolve();`, the Realm equivalent is `UserEvent::create_user_event(); user_event.trigger();`.

## Mechanism & API
```cpp
UserEvent user_event = UserEvent::create_user_event();

// Hand the (cast-to-Event) handle to consumers; they can wait on it.
Event downstream = p.spawn(SOME_TASK, &args, sizeof(args), user_event);

// ... later, when the producer's work is done ...
user_event.trigger();

// Or trigger chained on another event:
Event prev_event = ...;
user_event.trigger(prev_event);  // fires when prev_event triggers
```

`UserEvent::trigger()` may be called from any task on any node. The runtime forwards trigger requests to the event's home node, which propagates the triggered state to all subscribers (per `raw/tutorials/realm_03_events.md`: active-message count is bounded `2*N - 2` for N subscribers).

**Common patterns**:
- **Profiling synchronous wait** (`realm-profiling.md`): create a `UserEvent` in the calling task, pass to the profiling task, trigger it from the profiling task body, wait on it in the caller.
- **Cross-shard handshake**: one shard creates a user event; others wait on it; the first shard triggers when its work is at a known point.
- **Initialization completion**: a long async setup task triggers a user event; downstream work waits on the event.

## Invariants
- A `UserEvent` triggers **exactly once**; subsequent `trigger()` calls are no-ops (typically) or errors (in debug builds).
- Once triggered, the event stays triggered forever.
- `UserEvent` is castable to `Event`; all Event APIs accept it.
- The trigger can be conditioned on another event (`trigger(prev)`) — the user event fires when `prev` does.
- User events can **form cycles** if the application chains them carelessly. The runtime cannot detect application-level event cycles automatically — diagnose with `REALM_SHOW_EVENT_WAITERS` + `tools/detect_loops`.

## Performance implications
- User events are **cheap**: a 64-bit handle + a state slot on the home node.
- The trigger cost is one active message per subscriber; for short subscriber lists, negligible.
- **Cycles are the main risk** — a user event that depends (directly or transitively) on its own trigger hangs the runtime.

## Debug signals
- **`REALM_SHOW_EVENT_WAITERS=60+5`** dumps all pending events including untriggered user events. If a user event appears in the dump indefinitely, its trigger condition is never met (likely a cycle or a missing trigger call).
- **`tools/detect_loops`** processes the event-waiter dump; cycles involving user events show up clearly.
- **Application hangs without a runtime error** are usually missing user-event triggers.

## Failure modes
- Forgetting to call `trigger()` → consumer waits forever.
- Cyclic dependence via user events → hang detectable only via `REALM_SHOW_EVENT_WAITERS`.
- Triggering the same user event twice → debug-build assertion; silent in release.

## Source pointers
- **Realm header**: https://github.com/StanfordLegion/legion/blob/master/runtime/realm/event.h
- **Tutorial**: `raw/tutorials/realm_03_events.md`
- **Debug pattern**: `raw/website-pages/debugging.md` (REALM_SHOW_EVENT_WAITERS)

## Related
- `wiki/concepts/event.md` — base type; user events are a specialization.
- `wiki/concepts/event-poisoning.md` — error propagation through events (also applies to user events).
- `wiki/concepts/realm-profiling.md` — common use case for user events.
- `wiki/concepts/freeze-on-error.md` — debug aid when user-event-driven hangs occur.

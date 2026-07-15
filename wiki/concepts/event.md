---
title: Event
slug: event
summary: Realm's primitive for asynchronous completion; a lightweight handle that triggers when some underlying work finishes, and that other work can wait on.
tags: [synchronization, execution, for-program-reasoning]
subsystem: realm
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/00_tutorial_index.md
  - raw/publications/publications.md
  - raw/website-pages/debugging.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/realm/realm.h
  - https://github.com/StanfordLegion/legion/tree/master/runtime/realm
related:
  - wiki/concepts/task.md
  - wiki/concepts/physical-instance.md
  - wiki/concepts/operation-pipeline.md
  - wiki/concepts/future.md
  - wiki/concepts/region-instance.md
  - wiki/concepts/dma-system.md
  - wiki/concepts/reservation.md
  - wiki/concepts/realm-machine-model.md
  - wiki/concepts/user-event.md
  - wiki/concepts/event-poisoning.md
  - wiki/concepts/realm-barrier.md
  - wiki/concepts/completion-queue.md
---

## TL;DR
An `Event` is Realm's universal completion primitive. Every task launch, copy, fill, instance creation, and inter-node message produces an `Event` that triggers when the underlying work finishes. Events compose: `Event::merge_events({a, b, c})` returns an event that triggers when all three have. Legion's higher-level abstractions — `Future`, `PhysicalRegion::wait_until_valid()`, task completion — are built on events. The confusion: events are *not* threads or locks; they are *write-once trigger handles* that the runtime uses to express all dependencies.

## Mental model
Events are promises in the JavaScript-promise sense: created untriggered, triggered exactly once, queryable for completion. The whole Realm runtime is a graph of events; tasks and copies are nodes that consume some events and produce one. Where MPI would use blocking send/recv or asynchronous handles, Realm uses events.

## Mechanism & API
Core operations (in `runtime/realm/`):
- Most Realm operations return an `Event`: `Processor::spawn(task_id, args, precondition_event)`, `RegionInstance::create_instance(...)`, `IndexSpace::copy(...)`, etc.
- `Event::merge_events({e1, e2, ...})` — fan-in. Returns an event that triggers when all inputs have.
- `UserEvent` — application-triggered event for hand-built synchronization: `UserEvent::create_user_event()` + `trigger()` (or `trigger(event)` to chain).
- `Event::wait()` — block until triggered. Tasks should rarely call this; let dependence analysis do the waiting.
- `Barrier` — multi-generational, like a count-down latch that can be re-armed.

In Legion, you almost never touch events directly — `Future`, `PhysicalRegion`, `FutureMap` wrap them. They surface in profiling, error messages, and Legion Spy event-graph output.

## Invariants
- An event triggers **exactly once** and **monotonically** — once triggered, it stays triggered.
- An event is **untyped**: it just signals "the producer is done"; payload (return values, instance contents) is carried separately.
- An event can be **poisoned** if its producer errors out; downstream consumers can detect poisoning via `Event::has_triggered_faultaware()`.
- `merge_events({})` returns `Event::NO_EVENT`, which is already triggered.
- An event can be passed across nodes; Realm's active-message layer transports it.

## Performance implications
- Events are cheap (a 64-bit handle + state lookup). Creating many is fine.
- A **cycle in the event graph** is a deadlock; detect with `REALM_SHOW_EVENT_WAITERS=N+M` and `tools/detect_loops`.
- Excessive `Event::wait()` calls on the critical path serialize work — typically a Legion correctness pattern, not a perf one (let dependences do the waiting).
- `Futures` are Legion-level events plus a result buffer; same cost rules apply.

## Debug signals
- **Legion Spy event graph** (`-lg:spy -e`): every event and its waiters. Where Legion Prof shows *time*, Spy shows *causality*.
- **`REALM_SHOW_EVENT_WAITERS=60+5`**: after 60s, dump all pending event waiters every 5s. Used with `tools/detect_loops` to find deadlocks.
- **Backtrace mode** (`LEGION_BACKTRACE=1`) — when an event-related assertion fires, the stack tells you which operation produced/awaited it.

## Failure modes
- Event cycles → application hangs. Use `REALM_SHOW_EVENT_WAITERS` + `tools/detect_loops`.

## Source pointers
- **Realm header**: https://github.com/StanfordLegion/legion/blob/master/runtime/realm/realm.h
- **Realm runtime tree**: https://github.com/StanfordLegion/legion/tree/master/runtime/realm
- **Tools**: https://github.com/StanfordLegion/legion/tree/master/tools (`detect_loops`)
- **Paper (Realm)**: `raw/publications/pdfs/realm2014.pdf`

## Related
- `wiki/concepts/task.md` — `Future` is Legion's wrapper around a completion event + result.
- `wiki/concepts/physical-instance.md` — instances become valid at an event.
- `wiki/concepts/operation-pipeline.md` — stages 5–7 produce/consume events.
- `wiki/concepts/user-event.md` — application-triggered event subtype.
- `wiki/concepts/event-poisoning.md` — three-state model (untriggered/triggered/poisoned).
- `wiki/concepts/realm-barrier.md` — multi-arrival, multi-generation event subtype.
- `wiki/concepts/completion-queue.md` — set-of-events with "fire on any" semantics.

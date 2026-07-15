---
title: Completion Queue
slug: completion-queue
summary: A scalable mechanism for waiting on completion of any event in a set; comparable to `MPI_Testany`. The standard tool for "react when *any* of these N operations finishes" patterns.
tags: [synchronization, for-program-reasoning, for-perf-debug]
subsystem: realm
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/realm_10_completion_queue.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/realm/event.h
related:
  - wiki/concepts/event.md
  - wiki/concepts/user-event.md
  - wiki/concepts/realm-barrier.md
---

## TL;DR
A `CompletionQueue` is Realm's "set of events with notify-on-any-triggered" primitive. Add events via `add_event`, get a notification event via `get_nonempty_event` that triggers as soon as **any** of the added events fires, pop the triggered events via `pop_events`. Equivalent to `MPI_Testany` / `select(fds)`. The confusion: it's not a queue with push/pop ordering — it's a *set of events*, and pop returns the triggered ones in arbitrary order. Use it for "wake up when any worker is done so I can hand it more work" patterns.

## Mental model
Completion queue is `epoll` for Realm events: register a bunch of futures, get notified when any of them resolves, process the resolved ones, repeat. Where `Event::merge_events(...)` produces an event that fires when **all** inputs trigger, completion queue fires when **any** input triggers. Different operators, different patterns: merge for fan-in, completion queue for first-of-N work-stealing or progress reporting.

## Mechanism & API

**Create**:
```cpp
CompletionQueue cq = CompletionQueue::create_completion_queue(max_size);
// max_size = 0 means unbounded; non-zero caps the queue.
```

**Add events** (typically the events returned from `spawn` calls):
```cpp
for (int i = 0; i < num_workers; i++) {
  Event e = p.spawn(WORKER_TASK, &args, sizeof(args));
  cq.add_event(e);
}
```

**Wait for at least one event to trigger**:
```cpp
Event nonempty = cq.get_nonempty_event();
// nonempty triggers when the CompletionQueue has at least one triggered event ready to pop.
nonempty.wait();
```

**Pop triggered events**:
```cpp
std::vector<Event> popped(batch_size);
size_t got = cq.pop_events(&popped[0], batch_size);
// `got` is the actual number popped (may be less than batch_size).
```

**Destroy when done**:
```cpp
cq.destroy(/*precondition=*/Event::NO_EVENT);
```

**Common idiom — worker pool with adaptive dispatch**:
```cpp
CompletionQueue cq = CompletionQueue::create_completion_queue(0);
for (int i = 0; i < initial_batch; i++)
  cq.add_event(p.spawn(WORK_TASK, ..., ...));

while (work_remaining()) {
  cq.get_nonempty_event().wait();
  std::vector<Event> done(8);
  size_t n = cq.pop_events(&done[0], 8);
  for (size_t i = 0; i < n; i++) {
    // Worker `done[i]` finished; spawn another task to fill its slot.
    cq.add_event(p.spawn(WORK_TASK, ..., ...));
  }
}
cq.destroy();
```

## Invariants
- `pop_events` returns events in **arbitrary order** — the queue is a *set*, not a FIFO. Don't assume program-order or arrival-order.
- `get_nonempty_event` returns `NO_EVENT` if the queue already has triggered events ready (no need to wait). Always check both cases.
- The queue is **thread-safe**: concurrent `add_event` / `pop_events` / `get_nonempty_event` calls work as expected.
- `max_size = 0` is unbounded; non-zero caps the number of *triggered* events that can be held. Hitting the cap stalls `add_event` callers — sometimes a deliberate back-pressure mechanism.
- The queue can be passed as a task argument; tasks may add events to it from any node.

## Performance implications
- Adding an event to the queue is O(1) average; the queue uses internal Realm data structures sized to scale across nodes.
- For very large worker pools (10K+ events), bounded queues with back-pressure reduce memory.
- Compared to `Event::merge_events({...}).wait()`, completion queue lets you start processing finished events while others are still running — useful for adaptive workload patterns where dispatching new work depends on what completed.

## Debug signals
- **Hangs on `get_nonempty_event().wait()`** → none of the added events triggered. Either all workers are stuck (check upstream events with `REALM_SHOW_EVENT_WAITERS`) or no events were actually added.
- **Memory growth** in long-running adaptive dispatchers → bounded queue may be appropriate; or, you may be neglecting to pop triggered events.
- **`-level cq=2`** logs per-operation queue activity (where present).

## Failure modes
- Forgetting to destroy → minor resource leak; runtime collects at shutdown but mid-run reclaim doesn't happen.
- Assuming pop order matches add order → fragile code that breaks under concurrency.
- Bounded queue without explicit overflow handling → `add_event` blocks; can deadlock if the dispatcher is also the only popper.

## Source pointers
- **Realm header**: https://github.com/StanfordLegion/legion/blob/master/runtime/realm/event.h
- **Tutorial**: `raw/tutorials/realm_10_completion_queue.md`

## Related
- `wiki/concepts/event.md` — what the queue holds.
- `wiki/concepts/user-event.md` — sibling event primitive (single-shot).
- `wiki/concepts/realm-barrier.md` — sibling sync primitive (multi-arrival multi-generation).

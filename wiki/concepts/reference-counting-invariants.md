---
title: Reference Counting Invariants
slug: reference-counting-invariants
summary: The runtime's rules for keeping distributed reference counts consistent — resource references for handles, valid references for in-flight reachability, monotone increments before decrements per phase.
tags: [memory, distributed, for-correctness-debug, for-program-reasoning]
subsystem: legion
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/youtube_transcripts/runtime_school_2023/transcripts/006_Legion_Runtime_Internals_-_Lesson_6_-_Region_Tree_Nodes_and_Reference_Counting_Invariants.txt
github:
  - https://github.com/StanfordLegion/legion/tree/master/runtime/legion
related:
  - wiki/concepts/distributed-collectable.md
  - wiki/concepts/garbage-collection.md
  - wiki/concepts/region-tree.md
  - wiki/concepts/region-tree-node.md
---

## TL;DR
Reference counting in Legion's runtime is governed by a small set of invariants that keep counts consistent across processes despite asynchronous active-message delivery: **(1)** every collectable has two kinds of reference count (resource and valid), **(2)** within each phase, all increments are observed before any decrement, **(3)** the home node serializes all count changes, and **(4)** valid references prevent collection even when resource references are zero. Per Runtime School Lesson 6, getting these right is what makes the GC sound. The confusion: these are runtime-implementation invariants — application code doesn't manipulate counts directly, but understanding them is essential when debugging GC reports.

## Mental model
The invariants are the "memory model" of Legion's distributed reference counter. Like a C++ atomic's memory order, they specify what behaviors are allowed and what behaviors imply a bug. The runtime authors enforce them in `runtime/legion/`; debugging tools (`tools/legion_gc.py`) check for violations and report them as bugs.

## Mechanism & API
Per Runtime School Lesson 6:

**Two reference kinds**:
1. **Resource references** — someone holds a usable handle. Application handles, parent-child relationships in the region tree, mapper data.
2. **Valid references** — an in-flight operation can still need the object's data. Added by physical analysis when an operation begins, removed when the operation completes.

**The phase invariant**:
- Reference-count changes are processed by the home node in **phases**.
- Within a phase, **all increments are observed before any decrement is processed**. This prevents a temporarily-zero state where the home node would erroneously trigger collection while a new reference is in flight.
- The home node serializes the phase; non-home nodes send change requests and the home applies them in order.

**The home-node invariant**:
- The home node is the **single source of truth** for an object's reference counts. All other replicas track local approximations and report changes.
- Collection decisions are made only by the home.

**The valid-reference invariant**:
- A `ValidDistributedCollectable` (e.g., `region-tree-node.md`) cannot be collected while its valid-reference count is positive — even if resource-reference count is zero.
- This protects objects whose handle has been logically destroyed but whose data is still being used by in-flight operations.

**Lifecycle states**:
- `inactive` — created but not yet in use.
- `active` — referenced.
- `invalid` (for `ValidDistributedCollectable`) — resource-count zero but valid-count nonzero; cannot be collected yet.
- `deletable` — all counts zero; collection in progress.

The runtime fires `notify_active` / `notify_inactive` / `notify_valid` / `notify_invalid` callbacks at state transitions; subclasses override these to update their own derived state.

## Invariants
- **Resource and valid reference counts are independent**: an object can be `invalid` (resource=0, valid>0) for an arbitrarily long time.
- **Phase ordering**: all increments processed before any decrement within each phase; otherwise temporary zeros could trigger erroneous collection.
- **Home-node authority**: only the home decides collection. Non-home nodes' local counts are advisory.
- **Monotonicity per phase**: counts don't oscillate within a phase, simplifying the protocol.
- **Cross-node consistency**: active-message delivery is reliable; the home eventually sees every increment and decrement.

## Performance implications
- The phase protocol adds latency to remote count changes (one round trip per phase) but eliminates the need for distributed consensus on collection.
- Long-lived objects amortize the overhead; short-lived objects pay for it on every creation/collection cycle.
- Most application code doesn't notice the invariants — they're internal implementation. But mapper code that calls `acquire_instance` / `release_instance` is interacting with them directly.

## Debug signals
- **`tools/legion_gc.py`** reports invariant violations as runtime bugs — premature collection, never-collected objects, reference-count underflow.
- **Hangs on shutdown** sometimes indicate a leaked valid reference — an in-flight operation that was never marked complete.
- **Application-level mistakes** (forgetting to release an `acquire_instance`) show up as objects with nonzero `valid` count at end-of-run; the GC tool catches them.

## Failure modes
- Mapper code that calls `acquire_instance` but not `release_instance` → object lifetime extended indefinitely.
- Runtime bug producing reference-count underflow → premature collection; classic use-after-free.

## Source pointers
- **Runtime tree**: https://github.com/StanfordLegion/legion/tree/master/runtime/legion
- **Lecture**: `raw/youtube_transcripts/runtime_school_2023/transcripts/006_..._Region_Tree_Nodes_and_Reference_Counting_Invariants.txt`

## Related
- `wiki/concepts/distributed-collectable.md` — the base class these invariants govern.
- `wiki/concepts/garbage-collection.md` — what the invariants enable.
- `wiki/concepts/region-tree.md` — primary user of the invariants.
- `wiki/concepts/region-tree-node.md` — uses the `ValidDistributedCollectable` variant.

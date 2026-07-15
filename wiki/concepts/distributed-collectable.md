---
title: Distributed Collectable
slug: distributed-collectable
summary: The base class for every runtime object that participates in Legion's distributed garbage collection; provides reference-counted lifetimes that work across processes, with an extra "valid reference" state for in-flight access.
tags: [memory, distributed, for-correctness-debug, for-program-reasoning]
subsystem: legion
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/youtube_transcripts/runtime_school_2023/transcripts/005_Legion_Runtime_Internals_-_Lesson_5_-_Distributed_Collectable_Objects.txt
  - raw/youtube_transcripts/runtime_school_2023/transcripts/006_Legion_Runtime_Internals_-_Lesson_6_-_Region_Tree_Nodes_and_Reference_Counting_Invariants.txt
github:
  - https://github.com/StanfordLegion/legion/tree/master/runtime/legion
related:
  - wiki/concepts/garbage-collection.md
  - wiki/concepts/reference-counting-invariants.md
  - wiki/concepts/region-tree.md
  - wiki/concepts/region-tree-node.md
  - wiki/concepts/equivalence-set.md
---

## TL;DR
`DistributedCollectable` is the base class virtually every runtime-managed Legion object inherits from. It provides **distributed reference counting** (counts kept consistent across processes), supports an extra **"valid reference" state** that protects objects from collection while in-flight operations can still reach them, and integrates with Legion's `garbage-collection.md` mechanism. Per the Runtime School Lesson 5 transcript: it's the universal collectable infrastructure. The confusion: this isn't `std::shared_ptr` — it has to work across nodes and survive network message latency. The home-node concept (one node owns the canonical count) is how that's resolved.

## Mental model
`DistributedCollectable` is the distributed analog of `std::enable_shared_from_this` plus a reference counter: each instance has a known home node that owns the canonical count, and replicas on other nodes track local references and report changes to the home. When the home sees the total count hit zero, it triggers collection.

## Mechanism & API
The class hierarchy (per Runtime School Lesson 5 + 6):
- `DistributedCollectable` — base; tracks reference counts, owns the home-node-management protocol.
- `ValidDistributedCollectable` — adds the **valid reference** state for objects whose data must remain reachable during in-flight access (e.g., `region-tree-node.md`).
- Concrete subclasses: `PhysicalInstanceManager`, `IndexTreeNode`, `FieldSpaceNode`, `RegionTreeNode`, `EquivalenceSet`, plus mapper state and miscellaneous metadata.

**Reference kinds**:
- **Resource references** — someone holds a handle (an application-level handle, a child object, a mapper state).
- **Valid references** (the extra state) — an in-flight operation can still need the object's data; collection is blocked until the operation completes.

Plain reference counting handles the first kind; the valid-reference state is the extension that prevents premature collection during the gap between "logical end-of-life" and "all operations using it have completed".

**Home node**:
- Each `DistributedCollectable` is created on some node — its **home node**.
- Replicas may exist on other nodes; they hold pointers and contribute local reference counts.
- Reference-count changes from non-home nodes are sent to the home via active messages.
- Collection is initiated by the home node when the total count reaches zero.

**Inheriting from this** (typical pattern in `runtime/legion/`):
```cpp
class MyRuntimeObject : public DistributedCollectable {
  // override notify_active() / notify_inactive() / notify_invalid() ...
};
```

## Invariants
- Every collectable runtime object has a **single home node**, fixed at creation.
- Reference counts are **monotone across each phase** — increments before decrements within a phase.
- Cross-node reference changes go through Realm active messages; the home node serializes them.
- A `ValidDistributedCollectable` cannot be collected while it has any valid references — even if resource references are zero.
- Reference counting bugs are detected by `garbage-collection.md` tooling (`-DLEGION_GC`, `tools/legion_gc.py`).

## Performance implications
- **Reference count operations are atomic** but typically lock-free; per-op cost is small.
- Cross-node count updates are the only expensive operation — bundled in active messages when possible.
- Long-lived collectables (e.g., a top-level region used throughout the run) have stable counts and incur no ongoing cost.
- Short-lived collectables (created, used, freed in one operation) pay the full creation + collection round-trip.

## Debug signals
- **`tools/legion_gc.py`** (built from `-DLEGION_GC` logs) names which collectables leaked, which had reference errors.
- **Memory growth** in long-running apps → some collectable kind is accumulating; identify via the GC tool.
- **Lifecycle errors** (use-after-free, double-free) are runtime bugs; report via the standard channels with logs.

## Failure modes
- Holding a stale handle (e.g., destroyed region, kept a Future too long) → object stays alive past natural lifetime.
- A runtime bug that mis-counts references → leak or premature free; rare but possible — report via debug logs.

## Source pointers
- **Runtime tree**: https://github.com/StanfordLegion/legion/tree/master/runtime/legion (`runtime.cc`, `distributed_collectable.h`)
- **Lecture**: `raw/youtube_transcripts/runtime_school_2023/transcripts/005_..._Distributed_Collectable_Objects.txt`

## Related
- `wiki/concepts/garbage-collection.md` — the mechanism this underlies.
- `wiki/concepts/reference-counting-invariants.md` — the rules.
- `wiki/concepts/region-tree.md` — the most common collectable in user-facing code.
- `wiki/concepts/region-tree-node.md` — derived collectable.
- `wiki/concepts/equivalence-set.md` — also collectable.

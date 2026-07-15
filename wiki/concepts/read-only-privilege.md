---
title: READ_ONLY Privilege
slug: read-only-privilege
summary: A privilege declaring a task only reads the region; multiple concurrent READ_ONLY requirements on the same region are non-interfering — the runtime can share one valid instance across all of them.
tags: [data-model, dependence-analysis, for-program-reasoning]
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
  - wiki/concepts/privilege.md
  - wiki/concepts/region-requirement.md
  - wiki/concepts/non-interference.md
  - wiki/concepts/write-discard-privilege.md
---

## TL;DR
`READ_ONLY` declares that a task only reads the named region's fields — never writes. Two `READ_ONLY` requirements on the same region with the same field set are **non-interfering** (per `non-interference.md`); the runtime parallelizes them and may have them share one physical instance. The confusion: `READ_ONLY` is a *promise*, not a *type-system check* — write inside a `READ_ONLY` task in C++ Legion and the runtime trusts you (Regent's type system rejects it at compile time; `-DPRIVILEGE_CHECKS` catches it dynamically).

## Mental model
`READ_ONLY` is `const&` for Legion regions. Multiple consumers may take it concurrently; the runtime, knowing none of them mutate, can let them all observe the same buffer. The non-interference predicate (`non-interference.md`) returns "no conflict" for any two `READ_ONLY/READ_ONLY` pair with overlapping points + fields, so the runtime parallelizes them aggressively.

## Mechanism & API
```cpp
RegionRequirement(input_lr, READ_ONLY, EXCLUSIVE, input_lr);
```

Inside the task body:
```cpp
const FieldAccessor<READ_ONLY, double, 1> acc(regions[0], FID_X);
double v = acc[point];   // OK
acc[point] = 0.0;        // UB; -DPRIVILEGE_CHECKS catches at runtime
```

**Non-interference behavior** (from `raw/tutorials/07_privileges.md`):
- Two `READ_ONLY` requirements on the same region: **non-interfering**. May run concurrently.
- `READ_ONLY` vs. any write privilege (`READ_WRITE`, `WRITE_DISCARD`, `REDUCE`): conflicts (the writer must finish before the reader can start, and vice versa).
- Coherence weaker than `EXCLUSIVE` does not buy `READ_ONLY` anything — readers are already non-interfering at `EXCLUSIVE`.

**Subset rules**: a subtask's `READ_ONLY` requirement is a valid subset of any parent privilege (RO, RW, WD, REDUCE). Use this freely.

## Invariants
- A `READ_ONLY` task **must not** write the field/region. Writing is undefined behavior in release builds; `-DPRIVILEGE_CHECKS` (`privilege-checks.md`) catches it at the first access.
- Two `READ_ONLY` requirements on the same region with the same field set are **always** non-interfering.
- The runtime may share one physical instance across all concurrent `READ_ONLY` consumers — visible in `legion-prof.md` as a single memory-row slab with many concurrent task bars consuming it.
- `READ_ONLY` is compatible with all coherence modes; the most common pairing is `EXCLUSIVE`.

## Performance implications
- The **canonical privilege for fan-out workloads**: producer task does the work with `READ_WRITE` or `WRITE_DISCARD`, then many `READ_ONLY` consumers run in parallel.
- One-shared-instance sharing **eliminates copies** the runtime would otherwise emit to give each reader its own materialized view.
- For multi-stage stencil-style workloads, declaring intermediate reads as `READ_ONLY` (instead of leaving them as `READ_WRITE` by accident) is a major perf knob.

## Debug signals
- **`-DPRIVILEGE_CHECKS`** catches accidental writes inside a `READ_ONLY` task.
- **`dataflow-graph.md`**: an absent edge between two `READ_ONLY` tasks on the same region confirms non-interference. A present edge → check field sets or coherence.
- **Legion Prof**: concurrent `READ_ONLY` consumers appear as overlapping bars on different processors, all consuming the same memory-row instance.

## Failure modes
- Writing inside a `READ_ONLY` task → undefined behavior (UB in release; caught by `-DPRIVILEGE_CHECKS`).
- Declaring `READ_WRITE` instead of `READ_ONLY` for a read-only consumer → unnecessary serialization against other readers.

## Source pointers
- **Legion API**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Tutorial**: https://legion.stanford.edu/tutorial/privileges.html

## Related
- `wiki/concepts/privilege.md` — umbrella.
- `wiki/concepts/region-requirement.md` — where this is set.
- `wiki/concepts/non-interference.md` — why multiple ROs run concurrently.
- `wiki/concepts/write-discard-privilege.md` — the dual: producer-side perf privilege.

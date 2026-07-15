---
title: WRITE_DISCARD Privilege
slug: write-discard-privilege
summary: A privilege that declares "this task writes the region; nothing the prior writers produced needs to be preserved"; the runtime can elide initialization copies and allocate fresh instances.
tags: [data-model, dependence-analysis, for-perf-debug, for-program-reasoning]
subsystem: legion
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/07_privileges.md
  - raw/website-pages/debugging.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
related:
  - wiki/concepts/privilege.md
  - wiki/concepts/region-requirement.md
  - wiki/concepts/physical-instance.md
  - wiki/concepts/non-interference.md
  - wiki/concepts/privilege-checks.md
---

## TL;DR
`WRITE_DISCARD` is a privilege variant of `READ_WRITE` that adds one guarantee: **the task does not depend on the prior contents of the region**. Pass `WRITE_DISCARD` to a region requirement and the runtime is licensed to (a) eliminate read-after-write dependences on prior writers, and (b) allocate a fresh physical instance instead of populating one from existing valid data. The confusion: `WRITE_DISCARD` is not "writes faster than READ_WRITE" — it's a stronger *promise* the application makes. Break the promise (actually read inside a `WRITE_DISCARD` task) and you get undefined behavior in release builds, with no diagnostic unless `-DPRIVILEGE_CHECKS` is on.

## Mental model
`WRITE_DISCARD` is `mut!` with no read. Where `READ_WRITE` says "I will read and write these bytes", `WRITE_DISCARD` says "I will only write — please don't bother preserving what was here". This is the privilege equivalent of `__attribute__((noinit))` on a memory buffer: it tells the optimizer to skip initialization.

## Mechanism & API
Construct a region requirement with `WRITE_DISCARD`:
```cpp
RegionRequirement(input_lr, WRITE_DISCARD, EXCLUSIVE, input_lr);
```

What the runtime is licensed to do under `WRITE_DISCARD` (per the tutorial and physical-analysis sections of related pages):
- **Elide the initialization copy**: if a prior writer produced the only valid instance and this task uses `WRITE_DISCARD`, the runtime may allocate a fresh empty instance for this task instead of copying from the prior valid one.
- **Remove the RAW edge**: the new task does not depend on any prior write's *data*; only on the prior write's *completion* (so the instance can be reused). In practice this often means the task can start as soon as the runtime decides to map it.
- **Use a reduction instance unchanged**: if the prior data was a reduction-instance accumulator, `WRITE_DISCARD` lets the runtime drop the reduction altogether.

Inside the task, use an accessor that matches:
```cpp
const FieldAccessor<WRITE_DISCARD, double, 1> acc(regions[0], FID_X);
acc[point] = value;   // OK
double v = acc[point];   // UB unless `-DPRIVILEGE_CHECKS` catches it
```

**Non-interference with other operations**: `WRITE_DISCARD` is **not friendlier than `READ_WRITE`** for the non-interference predicate. Two `WRITE_DISCARD` requirements on overlapping points conflict (just like two `READ_WRITE`s would); the gain is in *intra-operation* runtime decisions, not in inter-operation parallelism.

## Invariants
- A `WRITE_DISCARD` task **must not read** the field/region it discards. Reading is undefined behavior; `-DPRIVILEGE_CHECKS` catches it dynamically.
- The runtime may but is not required to elide the init copy. Production runs will; debug runs may not.
- `WRITE_DISCARD` is compatible with all coherence modes; usually paired with `EXCLUSIVE`.
- `WRITE_DISCARD` propagates the same subset-privilege rules to subtasks as `READ_WRITE`.
- It is **not** the same as `WRITE_ONLY` in some other systems — Legion's `WRITE_DISCARD` is specifically about discarding *prior contents*; the task must still write every point it claims to write.

## Performance implications
- **Often the single largest single-line perf change** for init-style tasks. A DAXPY-style init that uses `WRITE_DISCARD` instead of `READ_WRITE` can save an entire region-sized copy on the cold path.
- **Removes RAW edges** from the operation graph — visible in `dataflow-graph.md` as an absent edge to prior writers.
- Combined with **disjoint partitions** and **per-field requirements**, `WRITE_DISCARD` is what gets you the DAXPY tutorial's fully-parallel field-wise init.
- The runtime is free to allocate fresh instances — this is a **cost** when memory is tight, since old instances may be reclaimed and rebuilt. For long-running iterative code, prefer reusing instances with `READ_WRITE` over allocating fresh ones with `WRITE_DISCARD` if you can.

## Debug signals
- **`-DPRIVILEGE_CHECKS`** — catches reads under `WRITE_DISCARD`. Always enable when changing privilege declarations.
- **`dataflow-graph.md`** — an absent edge between prior-writer and current `WRITE_DISCARD` task confirms the RAW elimination.
- **Wrong output that only appears in `-DPRIVILEGE_CHECKS=0` (release)** → suspect a `WRITE_DISCARD` that secretly reads. Rebuild with the flag and re-run.

## Failure modes
- Reading inside a `WRITE_DISCARD` task → undefined behavior. Usually manifests as wrong output in release builds.
- Forgetting that `WRITE_DISCARD` conflicts with other writes on the same points/fields → no parallelism gain.

## Source pointers
- **Tutorial**: https://legion.stanford.edu/tutorial/privileges.html (mirrored at `raw/tutorials/07_privileges.md`)
- **Legion API header**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Debug flag**: `raw/website-pages/debugging.md` (`-DPRIVILEGE_CHECKS`)

## Related
- `wiki/concepts/privilege.md` — host umbrella.
- `wiki/concepts/region-requirement.md` — where `WRITE_DISCARD` is set.
- `wiki/concepts/physical-instance.md` — what fresh allocation operates on.
- `wiki/concepts/non-interference.md` — `WRITE_DISCARD`'s relationship with parallelism.
- `wiki/concepts/privilege-checks.md` — runtime check that catches misuse.

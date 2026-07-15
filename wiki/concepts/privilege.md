---
title: Privilege
slug: privilege
summary: A per-task declaration of how it accesses a logical region (READ_ONLY, READ_WRITE, REDUCE, WRITE_DISCARD); the predicate Legion uses to decide which tasks may run in parallel.
tags: [data-model, dependence-analysis, for-program-reasoning, for-perf-debug]
subsystem: legion
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/07_privileges.md
  - raw/website-pages/debugging.md
  - raw/website-pages/overview.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
related:
  - wiki/concepts/logical-region.md
  - wiki/concepts/task.md
  - wiki/concepts/partition.md
  - wiki/concepts/operation-pipeline.md
  - wiki/concepts/coherence-mode.md
  - wiki/concepts/dependence-analysis.md
  - wiki/concepts/logical-analysis.md
  - wiki/concepts/regent-type-system.md
  - wiki/concepts/region-requirement.md
  - wiki/concepts/non-interference.md
  - wiki/concepts/field-level-non-interference.md
  - wiki/concepts/privilege-checks.md
  - wiki/concepts/bounds-checks.md
  - wiki/concepts/write-discard-privilege.md
  - wiki/concepts/read-only-privilege.md
  - wiki/concepts/read-write-privilege.md
  - wiki/concepts/reduce-privilege.md
---

## TL;DR
A privilege is a per-task, per-region-requirement annotation telling the runtime what kind of access the task will make. Four values: `READ_ONLY`, `READ_WRITE`, `REDUCE`, `WRITE_DISCARD`. The runtime uses privileges (plus coherence and field sets) to compute non-interference between tasks — that's what produces implicit parallelism. The confusion: privileges are not enforced like file permissions; they are *contracts* the runtime trusts. Get them wrong and you get UB, not an error (unless you compile with `-DPRIVILEGE_CHECKS`).

## Mental model
Privileges are operand types for the out-of-order scheduler in `operation-pipeline.md`. `READ_WRITE` is "true dependency" hazard fuel; `READ_ONLY` is "shared input"; `REDUCE` is "commutative-associative output". `WRITE_DISCARD` says "I will overwrite, you can throw away the prior contents" — the optimizer treats it like a register rename, eliding init copies and reduction folding. If you've ever annotated `__restrict` in C or specified `mutable`/`immutable` in Rust, privileges are the same idea raised to the level of partitioned distributed data.

## Mechanism & API
A region requirement carries both a logical region (or partition + projection) and a privilege:
```cpp
RegionRequirement(input_lr, READ_ONLY, EXCLUSIVE, input_lr);
RegionRequirement(output_lr, WRITE_DISCARD, EXCLUSIVE, output_lr);
```

Four privileges:
- `READ_ONLY` — pure read. Concurrent `READ_ONLY`s on the same region are non-interfering.
- `READ_WRITE` — full mutation. Conflicts with any other access on overlapping fields/points.
- `WRITE_DISCARD` — write without depending on prior contents. Runtime may **elide the init copy** from a prior physical instance because the data is logically uninitialized at task entry.
- `REDUCE` (with a `ReductionOpID`) — apply a commutative-associative operator. Concurrent `REDUCE`s with the *same* operator are non-interfering and fold via a reduction tree.

Privileges pair with **coherence** (`EXCLUSIVE`/`ATOMIC`/`SIMULTANEOUS`/`RELAXED`) and a **field set**. Non-interference is the AND of region disjointness, field disjointness, and compatible privilege/coherence.

Subtasks may only request a **subset** of their parent's privileges. There is no way to "create" a privilege from nothing.

## Invariants
- Two `READ_ONLY` requirements on the same region are non-interfering and may execute concurrently.
- Two `REDUCE` requirements on the same region with the same operator and disjoint fields are non-interfering.
- `WRITE_DISCARD` declares **no read dependence** on prior writes; the runtime is free to drop pending init copies. Writing a task with `WRITE_DISCARD` that secretly reads is undefined behavior in release builds.
- Subtask privileges ⊆ parent task privileges (always, no exceptions).
- Privileges cannot be stored, passed as values, or aliased; they transfer **only** via region requirements on launchers.
- The C++ API trusts the application. Regent's type system can statically prove privilege correctness; C++ requires `-DPRIVILEGE_CHECKS` for dynamic verification.

## Performance implications
- **Privilege precision is the #1 perf knob.** A `READ_WRITE` on a whole region forces a hard dependence on every prior writer; a `WRITE_DISCARD` on a single field of a single subregion forces only the writers of that field/subregion.
- `WRITE_DISCARD` lets the runtime allocate a fresh instance instead of copying from an existing one — often the largest single perf win in DAXPY-style init tasks.
- `REDUCE` enables **reduction instances** (per-replica accumulators) that fold at the end; without it, parallel updates serialize.
- Three forms of non-interference combine multiplicatively: region (disjoint subregions), field (disjoint fields in the same region), privilege (`RO/RO` or `REDUCE/REDUCE` same-operator). Use all three.

## Debug signals
- **Legion Spy** dataflow graph shows the privilege on each edge between tasks; spurious edges (a task pair that should be parallel but isn't) are usually privilege-too-broad.
- **`-DPRIVILEGE_CHECKS`** (recompile) — every accessor checks the actual privilege of its field at runtime; throws on mismatch.
- **`-DBOUNDS_CHECKS`** — sibling check: a privilege is meaningless if the access is out of the requested region.
- **Error messages** about parent/child privilege mismatch surface during dependence analysis; see `raw/website-pages/error_messages.md`.

## Failure modes
- [Over-broad privileges create false dependencies](../pitfalls/false-dependencies-overbroad-privileges.md) — the most common Legion perf bug.

## Source pointers
- **Header**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Tutorial**: https://legion.stanford.edu/tutorial/privileges.html
- **Debug flags**: https://github.com/StanfordLegion/legion/tree/master/runtime/legion (compile with `-DPRIVILEGE_CHECKS`)

## Related
- `wiki/concepts/logical-region.md` — what privileges are taken on.
- `wiki/concepts/task.md` — where privileges are declared.
- `wiki/concepts/partition.md` — disjoint subregions × disjoint privileges = max parallelism.
- `wiki/concepts/operation-pipeline.md` — where the non-interference check happens.

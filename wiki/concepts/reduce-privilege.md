---
title: REDUCE Privilege
slug: reduce-privilege
summary: A privilege that declares a task applies a commutative-associative reduction operator (registered with a ReductionOpID); multiple concurrent REDUCE requirements with the same operator are non-interfering and fold into a tree.
tags: [data-model, dependence-analysis, parallelism, for-program-reasoning, for-perf-debug]
subsystem: legion
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/07_privileges.md
  - raw/tutorials/realm_08_reductions.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
related:
  - wiki/concepts/privilege.md
  - wiki/concepts/region-requirement.md
  - wiki/concepts/reduction-instance.md
  - wiki/concepts/non-interference.md
  - wiki/concepts/read-only-privilege.md
  - wiki/applications/circuit.md
---

## TL;DR
`REDUCE` is a privilege that declares "I will apply a registered commutative-associative reduction operator to this region/field". The privilege carries a `ReductionOpID` identifying the operator; multiple concurrent `REDUCE` requirements with the **same** operator on overlapping points are **non-interfering** and the runtime can issue a tree-reduction to fold them. The confusion: `REDUCE` is not "any write" — the application must register the operator first via `register_reduction`, and using two different operators on the same data forces serialization.

## Mental model
`REDUCE` is OpenMP's `reduction(+:x)` lifted to distributed Legion: the application declares the operator, the runtime materializes per-shard partial accumulators (`reduction-instance.md`), each contributor folds into its own copy, and the runtime tree-merges at the end. The whole game is "many concurrent updates without locking, because the operator is mathematically combine-able".

## Mechanism & API
**Register the operator** (once, before `Runtime::start`):
```cpp
class SumReduction {
public:
  typedef double LHS;
  typedef double RHS;
  static const double identity = 0.0;
  template <bool EXCLUSIVE>
  static void apply(LHS &lhs, RHS rhs) {
    if constexpr (EXCLUSIVE) lhs += rhs;
    else __sync_fetch_and_add_double(&lhs, rhs);  // atomic for shared use
  }
  template <bool EXCLUSIVE>
  static void fold(RHS &rhs1, RHS rhs2) {
    if constexpr (EXCLUSIVE) rhs1 += rhs2;
    else __sync_fetch_and_add_double(&rhs1, rhs2);
  }
};

Runtime::register_reduction_op<SumReduction>(REDOP_SUM);
```

The contract (per `raw/tutorials/realm_08_reductions.md`):
- `LHS` / `RHS` type definitions.
- `apply(lhs, rhs)` — combines a `LHS` accumulator with a new `RHS` value.
- `fold(rhs1, rhs2)` — combines two `RHS` values (used by the tree-merge).
- `identity` — neutral element (zero for sum, infinity for min, etc.).
- Each method has an exclusive and a non-exclusive variant; non-exclusive uses atomics for concurrent reducers.

**Use the privilege**:
```cpp
RegionRequirement(output_lr, REDOP_SUM, EXCLUSIVE, output_lr);
```
(The third argument is the `ReductionOpID`, not a privilege enum — REDUCE is *encoded by* having a non-zero redop ID.)

**Non-interference behavior** (per `raw/tutorials/07_privileges.md`):
- Two `REDUCE` requirements on the same region with the **same** operator: non-interfering.
- Two `REDUCE` requirements with **different** operators: conflict.
- `REDUCE` vs. any other privilege (RO, RW, WD): conflict.

Tutorials show this in DAXPY-style reductions and accumulator patterns.

## Invariants
- The reduction operator **must be associative**; non-associative ops (like subtraction) produce non-deterministic results.
- The runtime trusts the registered operator's correctness; a bug in `apply`/`fold` is silent.
- Each `REDUCE` requirement carries the operator ID; the runtime checks at submit time that the ID is registered.
- Non-exclusive `apply`/`fold` paths must use atomics; the runtime invokes them when reductions race.
- Identity is required and used to initialize fresh `reduction-instance.md`s.
- A `REDUCE` task **observes the identity, not prior writes** — like `WRITE_DISCARD`, the prior contents are not visible.

## Performance implications
- **The standard way to express concurrent commutative-associative updates** without serialization. Far better than `READ_WRITE` + reservation for "many tasks update the same scalar/array" patterns.
- The runtime allocates **reduction instances** (`reduction-instance.md`) per shard/replica; the tree-fold at the end is automatic.
- Combined with **index launches** + **same operator**, point tasks run fully in parallel; the runtime gathers their per-shard accumulators.
- A `REDUCE` task can use **atomic accessors** if multiple concurrent same-op reducers hit the same instance.

## Debug signals
- **`dataflow-graph.md`**: same-op `REDUCE` tasks don't have edges between them; different-op or `READ_WRITE`-mixed have edges.
- **`-DPRIVILEGE_CHECKS`** catches writes outside the registered operator's semantics.
- **Legion Prof memory rows** show reduction instances as their own kind of slab; channel rows show the eventual fold copies.
- **Wrong reduction results** → suspect a non-associative operator or a bug in `apply`/`fold`. Test on a single processor first.

## Failure modes
- Forgetting to call `register_reduction_op` → runtime error at first `REDUCE` requirement.
- Two `REDUCE` requirements with different operators on the same data → unintended serialization.
- Non-associative operator → non-deterministic results between runs.

## Source pointers
- **Legion API**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Tutorial (Legion privileges)**: https://legion.stanford.edu/tutorial/privileges.html
- **Tutorial (Realm reductions)**: `raw/tutorials/realm_08_reductions.md`

## Related
- `wiki/concepts/privilege.md` — umbrella.
- `wiki/concepts/region-requirement.md` — where this is set.
- `wiki/concepts/reduction-instance.md` — the storage backing `REDUCE` tasks.
- `wiki/concepts/non-interference.md` — why same-op `REDUCE`s are concurrent.
- `wiki/concepts/read-only-privilege.md` — sibling perf-friendly privilege.

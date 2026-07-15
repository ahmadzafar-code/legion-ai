---
title: False Dependencies from Over-Broad Privileges
slug: false-dependencies-overbroad-privileges
summary: Tasks serialize on each other because at least one requested a privilege wider than its actual access — most often READ_WRITE on a whole region when only a field or subregion is touched.
tags: [for-perf-debug, dependence-analysis, data-model]
status: draft
created: 2026-05-15
updated: 2026-05-15
related:
  - wiki/concepts/privilege.md
  - wiki/concepts/logical-region.md
  - wiki/concepts/partition.md
  - wiki/concepts/legion-spy.md
  - wiki/concepts/legion-prof.md
  - wiki/workflows/debug-correctness-bug.md
---

## Symptom
- Two or more tasks that *should* be independent run sequentially.
- **Legion Prof critical-path view** (`a`) shows a single chain where you expect a fan-out.
- **Legion Spy dataflow graph** (`-lg:spy` then `legion_spy.py -dez`) shows an edge between tasks whose data sets don't actually overlap.
- Wall-clock time scales with the number of tasks, not with the number of processors.

## Cause
The Legion runtime computes non-interference per region-requirement. Two requirements interfere if:
1. Their logical regions overlap (same region or aliased subregions), AND
2. Their field sets overlap, AND
3. Their privileges conflict (anything paired with `READ_WRITE` conflicts; two `READ_ONLY` or two same-operator `REDUCE` don't conflict).

A common bug pattern:
- Requesting `READ_WRITE` on the whole top-level region when the task only writes one field.
- Requesting privileges on a parent region when only a subregion is touched.
- Reusing the same `RegionRequirement` across launches without clearing fields between them.

The runtime trusts the application's declarations; broader privileges → broader conflict → false dependence.

## Fix
- **Narrow the field set.** `req.privilege_fields.clear()` then `add_field(FID_X)` rather than carrying every field forward.
- **Use partitioning.** Replace `RegionRequirement(parent_lr, ...)` with `RegionRequirement(partition, projection_id, ...)` so each point task touches a disjoint subregion. See `wiki/concepts/partition.md`.
- **Prefer `WRITE_DISCARD` over `READ_WRITE`** whenever the task overwrites unconditionally. The runtime drops init copies and removes RAW edges.
- **Prefer `REDUCE`** for commutative-associative updates; multiple `REDUCE` ops with the same operator are non-interfering.
- **Verify** with `legion_spy.py -dez` after the fix: the offending edge should be gone.

## Underlying concepts
- `wiki/concepts/privilege.md` — the contract being declared.
- `wiki/concepts/logical-region.md` — what privileges are taken on.
- `wiki/concepts/partition.md` — how to narrow the logical region.
- `wiki/concepts/legion-spy.md` — how to see the false edge.
- `wiki/concepts/legion-prof.md` — how to see the serial chain in time.

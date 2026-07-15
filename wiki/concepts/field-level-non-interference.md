---
title: Field-Level Non-Interference
slug: field-level-non-interference
summary: The form of non-interference that lets the runtime parallelize operations on the same logical region when their field sets are disjoint; the reason DAXPY-style field-wise parallelism works without partitioning.
tags: [dependence-analysis, data-model, parallelism, for-program-reasoning, for-perf-debug]
subsystem: legion
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/07_privileges.md
  - raw/tutorials/08_partitioning.md
github:
  - https://github.com/StanfordLegion/legion/tree/master/runtime/legion
related:
  - wiki/concepts/non-interference.md
  - wiki/concepts/region-requirement.md
  - wiki/concepts/logical-region.md
  - wiki/concepts/privilege.md
  - wiki/concepts/partition.md
---

## TL;DR
Field-level non-interference is the second of the three non-interference axes (`non-interference.md`): two operations on the **same logical region with overlapping points** can still be non-interfering if their **field sets are disjoint**. The classic example: initializing fields X and Y of the same region in parallel via two task launches with `WRITE_DISCARD` privilege on different fields. The confusion: this only works when the field sets are *truly* disjoint at the level the runtime sees — clearing and re-adding fields between launcher reuses is what makes this safe in idiomatic Legion code.

## Mental model
Think of a logical region as a database table and a field as a column. Two write operations to disjoint columns of the same table can run concurrently without locking, even though they hit the same rows. Field-level non-interference is the same principle: same region (rows), disjoint columns (fields), full parallelism.

## Mechanism & API
The DAXPY tutorial pattern (`raw/tutorials/07_privileges.md`) is the canonical example:
```cpp
// Initialize field X
init_launcher.region_requirements[0].privilege_fields.clear();
init_launcher.region_requirements[0].add_field(FID_X);
runtime->execute_task(ctx, init_launcher);

// Initialize field Y (parallel with X)
init_launcher.region_requirements[0].privilege_fields.clear();
init_launcher.region_requirements[0].add_field(FID_Y);
runtime->execute_task(ctx, init_launcher);
```

Both launches request `WRITE_DISCARD` on the **same logical region** but disjoint field sets (`{FID_X}` vs `{FID_Y}`). The runtime's `logical-analysis.md` check:
- Region overlap: yes (same region).
- Field overlap: no.
- → non-interfering.

The two initialization tasks run in parallel (modulo mapper placement).

**Reusing launchers correctly**:
- Clear `privilege_fields` (and `instance_fields` for the corresponding output requirement) between launches that should target different field sets.
- Add only the fields you want for this specific launch.
- The tutorial code does exactly this with `.clear()` + `add_field()`.

**Combination with region non-interference**:
- Disjoint partitions × disjoint fields × compatible privileges = maximum parallelism. Index launches over a partition with each point task using narrow field requirements push all three axes.

## Invariants
- Field overlap is computed on the literal field sets in the region requirements. The runtime does not infer "you said field X but only wrote half of it".
- Field-level non-interference works for any privilege combination as long as fields are disjoint — even two `READ_WRITE` operations are independent if they hit different fields.
- A **field that appears in both `privilege_fields` and is privileged differently** in the two requirements still causes interference if they overlap on that field.
- Fields in different `FieldSpace`s are inherently disjoint (different namespaces).
- The `MAX_FIELDS` (default 512) limit is per `FieldSpace`; field-level non-interference works fine up to that limit.

## Performance implications
- **The standard way to get parallelism on shared data structures** without partitioning. Particularly useful when the data layout makes partitioning awkward but the access pattern is naturally per-field.
- For mixed-field workloads (initialize X, initialize Y, then DAXPY reading both, then check reading Z), field-level non-interference parallelizes the two inits with each other but correctly serializes them against the DAXPY (which reads both fields).
- Combines multiplicatively with `region non-interference` (`partition.md`) and `privilege non-interference` (RO/RO or REDUCE/REDUCE same op). Use all three.

## Debug signals
- **Legion Spy dataflow graph**: if two same-region operations show no edge between them, field-level non-interference fired. If they're edge-connected when you expected parallel, the field sets probably overlap.
- **Forgetting to clear `privilege_fields`** between launcher reuses is the most common bug here — the second launch sees both old + new fields, falsely interferes with prior writers of the old fields.
- **`-DPRIVILEGE_CHECKS`** + accessor use confirms which fields the task is actually touching.

## Failure modes
- Forgetting to clear `privilege_fields` → stale fields in the requirement → false interference.
- Field promotion across `FieldSpace`s confusing the application (different FSes = different field IDs even if numerically equal).

## Source pointers
- **Tutorial (DAXPY field-level NI)**: https://legion.stanford.edu/tutorial/privileges.html
- **Implementation tree**: https://github.com/StanfordLegion/legion/tree/master/runtime/legion (`region_tree.cc`, `legion_analysis.cc`)

## Related
- `wiki/concepts/non-interference.md` — umbrella concept.
- `wiki/concepts/region-requirement.md` — the field set lives here.
- `wiki/concepts/logical-region.md` — same region, multiple fields.
- `wiki/concepts/privilege.md` — works with any privilege if fields are disjoint.
- `wiki/concepts/partition.md` — sister axis (region non-interference).

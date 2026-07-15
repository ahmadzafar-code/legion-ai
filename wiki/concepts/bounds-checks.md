---
title: Bounds Checks
slug: bounds-checks
summary: Compile flag `-DBOUNDS_CHECKS` that turns every accessor operation into a check that the point being accessed falls within the region the task was granted; catches out-of-bounds bugs that release builds silently corrupt memory through.
tags: [debugging, configuration, for-correctness-debug]
subsystem: legion
layer: tooling
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/website-pages/debugging.md
  - raw/tutorials/07_privileges.md
github:
  - https://github.com/StanfordLegion/legion/tree/master/runtime/legion
related:
  - wiki/concepts/privilege-checks.md
  - wiki/concepts/region-requirement.md
  - wiki/concepts/logical-region.md
  - wiki/concepts/partition.md
---

## TL;DR
`-DBOUNDS_CHECKS` is a compile-time flag that instruments every `FieldAccessor::operator[]` with a check that the point being accessed lies within the region the task was granted. Without it, an out-of-bounds access silently corrupts memory (Legion's accessors compute offsets into the physical instance and can wander into adjacent fields or neighboring instances). With it, the runtime throws an error at the first out-of-bounds access, pointing at the responsible task. The confusion: bounds-checks complements `privilege-checks.md` — privileges control *what* you can access; bounds-checks controls *where* you can access. Most production bugs need both flags on.

## Mental model
`-DBOUNDS_CHECKS` is `-fsanitize=bounds` for Legion regions. Each indexed accessor operation becomes a guarded operation that asserts the point is in the requested region's domain before computing the memory offset. The cost is a per-access branch; the benefit is catching off-by-one and partition-mismatch bugs at the *first* offending access.

## Mechanism & API
Enable at build time:
```bash
CC_FLAGS=-DBOUNDS_CHECKS make
./app
```

What it catches (per `raw/website-pages/debugging.md`):
- A task indexing outside the bounds of any of its requested region requirements.
- Common with partition mismatches: task expects subregion `i` but the launcher accidentally passed subregion `j`.
- Off-by-one in loop bounds over a partition's color space.
- Iterator bugs where the loop walks beyond `rect.hi` of the granted subregion.

**Combine with**:
- `-DPRIVILEGE_CHECKS` for full access-correctness coverage.
- `DEBUG=1` for runtime assertions.
- `LEGION_BACKTRACE=1` for a stack trace at the violation.

The flag works for all accessor kinds — `FieldAccessor`, `MultiRegionAccessor`, etc.

## Invariants
- The check is **strictly conservative**: only real out-of-bounds accesses trigger.
- Adds **no semantic change** to correct programs.
- Catches **declared-region bounds**, not the actual instance bounds (the actual instance may be larger if `-DFULL_SIZE_INSTANCES` is in play, but the check uses the task's region requirement).
- Fires **inside the task body**, naming the responsible task.
- Independent of `-DPRIVILEGE_CHECKS` — runs separately; usually you want both.

## Performance implications
- **Per-access overhead**: every accessor operator pays a bounds branch. Substantial in tight loops.
- Do not measure perf with this flag on.
- Use during development and CI; strip for `DEBUG=0` release builds.
- Cheaper than `-DFULL_SIZE_INSTANCES`, more targeted.

## Debug signals
- **"Bounds check violation in task X" error** at runtime = an indexed access went outside the granted region. Fix the loop bound or the region requirement.
- **Programs that succeed without but fail with `-DBOUNDS_CHECKS`** → real bounds bug previously masked by garbage from adjacent memory.
- **Combination with partition mismatches**: the bounds error often points at a mistaken use of `lp[wrong_color]` instead of `lp[task->index_point]` in an index task.

## Failure modes
- Caught by this check: out-of-bounds within the task's declared regions.
- Not caught: declared-privilege violations (`privilege-checks.md`); non-disjoint disjoint partitions (`partition-checks.md`); cross-partition accesses where the partition is just wrong.

## Source pointers
- **Reference**: `raw/website-pages/debugging.md`
- **Implementation**: https://github.com/StanfordLegion/legion/tree/master/runtime/legion (accessor classes)
- **Tutorial (where accessor patterns are introduced)**: `raw/tutorials/07_privileges.md`

## Related
- `wiki/concepts/privilege-checks.md` — sibling correctness check; usually paired.
- `wiki/concepts/region-requirement.md` — what defines the legal bounds.
- `wiki/concepts/logical-region.md` — the domain of the region.
- `wiki/concepts/partition.md` — partition-mismatch bounds bugs land here.

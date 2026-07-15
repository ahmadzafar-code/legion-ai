---
title: Privilege Checks
slug: privilege-checks
summary: Compile flag `-DPRIVILEGE_CHECKS` that turns every accessor operation into a dynamic check that the task actually requested the privilege for the field being accessed.
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
  - wiki/concepts/privilege.md
  - wiki/concepts/region-requirement.md
  - wiki/concepts/bounds-checks.md
  - wiki/concepts/debug-mode.md
---

## TL;DR
`-DPRIVILEGE_CHECKS` is a compile-time flag that instruments every `FieldAccessor` operation with a runtime check: does the current task's region requirement *actually* declare the privilege (RO / RW / WD / REDUCE) it's trying to use on the field being touched? Without this flag, the runtime trusts the application's accessor type and access pattern; with it, mismatches throw a clear error pointing at the offending task. The confusion: Regent's static type system already enforces this at compile time; `-DPRIVILEGE_CHECKS` is for C++ Legion programs where there's no static checker.

## Mental model
`-DPRIVILEGE_CHECKS` is `-fsanitize=privilege` for Legion's C++ API. Each `FieldAccessor<RO, double, 1>::operator[]` becomes a guarded operation that asserts the task did indeed declare RO on this field before performing the load. The cost is a per-access branch; the benefit is catching the bug at the *first* offending access rather than discovering wrong output downstream.

## Mechanism & API
Enable at build time:
```bash
CC_FLAGS=-DPRIVILEGE_CHECKS make
./app
```

What this does (per `raw/website-pages/debugging.md`):
- Every accessor constructor and operator records the privilege and field it claims.
- The check compares against the task's region requirements' actual declared privilege.
- A mismatch — e.g., a task with `READ_ONLY` privilege using a `READ_WRITE` accessor, or accessing a field not in `privilege_fields` — throws an error identifying the task and the privilege violation.

**Typical patterns the flag catches**:
- A task launched with `READ_ONLY` on a region whose body uses `FieldAccessor<READ_WRITE, ...>` (the field is mutated without permission).
- A task whose region requirement lists `{FID_X, FID_Y}` but the body accesses `FID_Z` (field not granted).
- A `WRITE_DISCARD` task that actually reads the field (silent UB in release).

**Combine with**:
- `-DBOUNDS_CHECKS` to also catch out-of-bounds within the requested region.
- `LEGION_BACKTRACE=1` to get a stack trace at the violation site.

## Invariants
- The check is **strictly conservative**: it never flags a valid access; mismatches identify real bugs.
- Adds **no semantic change** to correct programs — just instrumentation.
- Catches **declared privilege violations**, not runtime data-race violations (those are a `partition-checks` and `legion-spy` concern).
- The check fires **inside the task body**, naming the responsible task — the error message tells you where to look.
- Reset by removing the flag at compile time; runtime cannot toggle.

## Performance implications
- **Per-access overhead**: every `FieldAccessor` operator pays a branch and a check. Substantial in tight loops; do not measure perf with this flag on.
- Use during development and CI; strip for production builds (`DEBUG=0` without `-DPRIVILEGE_CHECKS`).
- Regent users have no need for this flag — the type system gives the same guarantees at compile time.

## Debug signals
- **"Privilege violation in task X" error** at runtime = a declared/actual mismatch, identified by task name and field ID. Fix the region requirement or the accessor type.
- **Programs that succeed under `-DPRIVILEGE_CHECKS` but fail without** → likely a `WRITE_DISCARD`-but-actually-reads bug that's UB in release but happens to give the right answer in debug.
- **Programs that succeed without but fail with `-DPRIVILEGE_CHECKS`** → real privilege bug; previously masked by accidentally correct output.

## Failure modes
- Caught by this very check: declared/actual privilege mismatches.
- Not caught: out-of-bounds within the declared region (needs `-DBOUNDS_CHECKS`); non-disjoint disjoint partitions (needs `-lg:partcheck`); silent data races from coherence misuse.

## Source pointers
- **Reference**: `raw/website-pages/debugging.md`
- **Implementation**: https://github.com/StanfordLegion/legion/tree/master/runtime/legion (accessor classes under `legion.h` / `legion.inl`)
- **Tutorial (privileges)**: `raw/tutorials/07_privileges.md`

## Related
- `wiki/concepts/privilege.md` — what's being checked.
- `wiki/concepts/region-requirement.md` — where the declared privilege lives.
- `wiki/concepts/bounds-checks.md` — sibling correctness check.

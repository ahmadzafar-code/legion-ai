---
title: READ_WRITE Privilege
slug: read-write-privilege
summary: The default mutation privilege; declares a task both reads and writes the region; conflicts with everything else on overlapping points+fields and forces the runtime to preserve the prior contents on entry.
tags: [data-model, dependence-analysis, for-program-reasoning, for-perf-debug]
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
  - wiki/concepts/read-only-privilege.md
  - wiki/concepts/write-discard-privilege.md
---

## TL;DR
`READ_WRITE` is the default mutation privilege: the task may read the region and may write it. The runtime treats it as a hard mutator — it conflicts with every other access (including other `READ_WRITE`s) on overlapping points and fields, and it forces the runtime to preserve the region's prior contents (the task is allowed to read them). The confusion: `READ_WRITE` is **not** what you want for most "mutating" tasks. If the task overwrites without reading, prefer `write-discard-privilege.md` and gain init-copy elision + a removed RAW edge.

## Mental model
`READ_WRITE` is `mut&` in Rust: full read-and-write access, exclusive — no one else can hold any privilege on overlapping data while you have it. The most conservative mutation privilege; the runtime cannot make assumptions that would let it skip work.

## Mechanism & API
```cpp
RegionRequirement(io_lr, READ_WRITE, EXCLUSIVE, io_lr);
```

Inside the task body:
```cpp
const FieldAccessor<READ_WRITE, double, 1> acc(regions[0], FID_X);
double v = acc[point];   // OK
acc[point] = v + 1.0;    // OK
```

**Non-interference behavior** (per `raw/tutorials/07_privileges.md`):
- `READ_WRITE` vs. anything else on overlapping points+fields: **conflicts**. Forces serialization.
- The runtime preserves prior contents — if a prior writer left data, the runtime ensures it's in the instance when this task begins (issuing a copy if needed).
- Coherence weaker than `EXCLUSIVE` widens the non-interference predicate (`coherence-mode.md`) but transfers ordering responsibility to the application.

**When to use it (per Legion idiom)**:
- The task genuinely reads-then-writes (most "update" kernels).
- The task writes only some points and reads others (you can't decompose into per-point requirements).
- The instance must outlive the task (you want the runtime to keep the buffer warm).

**When NOT to use it**:
- The task overwrites every point unconditionally → use `WRITE_DISCARD` for the init-copy-elision win.
- The task only reads → use `READ_ONLY` so concurrent readers run in parallel.
- The task only reduces → use `REDUCE` with a `ReductionOpID` for parallel folds.

## Invariants
- A `READ_WRITE` task may read and write any field listed in `privilege_fields`. Reading or writing other fields is UB (caught by `privilege-checks.md`).
- `READ_WRITE` with `EXCLUSIVE` coherence is **always** serializing with any other access on overlapping data.
- Subtask privileges must be subsets: from a `READ_WRITE` parent, any of RO/RW/WD/REDUCE on a subset of points+fields is allowed.
- A `READ_WRITE` task **observes** the most recent writes by its predecessors — the runtime materializes them in the instance before the task runs.
- `READ_WRITE` instances are reused across calls when possible; the runtime may keep a hot copy in fast memory.

## Performance implications
- **The most conservative privilege** — gives the runtime no extra room to optimize.
- The single most common perf bug under `READ_WRITE` is using it when `WRITE_DISCARD` would suffice (`pitfalls/false-dependencies-overbroad-privileges.md` indirectly).
- The forced-preserve-prior-contents semantics means an init copy on every cold path; switch to `WRITE_DISCARD` for that case.
- Compatible with all other perf knobs (tracing, control replication, sharding).

## Debug signals
- **`dataflow-graph.md`**: an edge from a prior writer to a current `READ_WRITE` task always exists; an unexpected absence means a privilege misdeclaration.
- **Heavy channel-row activity** before a `READ_WRITE` task in Legion Prof → the runtime is materializing prior writes. If the task doesn't read, switch to `WRITE_DISCARD`.
- **`-DPRIVILEGE_CHECKS`** catches accidental access to fields not in `privilege_fields`.

## Failure modes
- Using `READ_WRITE` for what's logically a `WRITE_DISCARD` → unnecessary copies on cold paths; perf hit.
- Using `READ_WRITE` for what's logically `READ_ONLY` → unnecessary serialization against other readers.

## Source pointers
- **Legion API**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Tutorial**: https://legion.stanford.edu/tutorial/privileges.html

## Related
- `wiki/concepts/privilege.md` — umbrella.
- `wiki/concepts/region-requirement.md` — where this is set.
- `wiki/concepts/non-interference.md` — why `READ_WRITE` conflicts with so much.
- `wiki/concepts/read-only-privilege.md` — relax to this when no writes happen.
- `wiki/concepts/write-discard-privilege.md` — relax to this when no reads happen.

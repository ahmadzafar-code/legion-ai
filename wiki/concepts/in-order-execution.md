---
title: In-Order Execution
slug: in-order-execution
summary: Runtime flag `-lg:inorder` that forces the runtime to execute every operation strictly in launch order, eliminating all parallelism; the standard tool for reproducing timing-dependent bugs.
tags: [debugging, configuration, execution, for-correctness-debug]
subsystem: legion
layer: tooling
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/website-pages/debugging.md
github:
  - https://github.com/StanfordLegion/legion/tree/master/runtime/legion
related:
  - wiki/concepts/operation-pipeline.md
  - wiki/concepts/non-interference.md
  - wiki/concepts/freeze-on-error.md
  - wiki/concepts/debug-mode.md
---

## TL;DR
`-lg:inorder` is a runtime flag that tells Legion to execute every operation in **strict program order**, ignoring the parallelism that logical analysis would normally extract. Useful for: (a) reproducing bugs whose symptoms depend on timing, (b) confirming that a wrong-answer issue is not caused by a missing dependence, (c) simplifying debugging by making execution deterministic. The confusion: `-lg:inorder` is enormously slower than the parallel default — it's a debugging crutch, not a fix. If your program needs it to be correct, you have a real bug (false reliance on an undeclared dependence).

## Mental model
`-lg:inorder` is `--single-thread` for Legion: serialize everything, eliminate concurrency, get reproducibility back. The same kind of tool a JVM debugger gives you when chasing a race — you trade parallelism for predictability. Once the bug is identified, you fix it and remove the flag.

## Mechanism & API
Pass at runtime:
```bash
./app -lg:inorder
```

What this does (per `raw/website-pages/debugging.md`):
- Every operation waits for the previous operation to complete before mapping or executing.
- No two operations run concurrently, even when logical analysis would normally find them non-interfering.
- Equivalent in effect to manually putting a fence between every launch.

**When to use**:
- A bug whose symptoms vary across runs but is hard to pin down. Run with `-lg:inorder`; if symptoms disappear, the bug is timing-dependent (race condition, missing dependence, mapper non-determinism).
- A wrong-answer issue where you suspect the runtime is reordering something illegally. `-lg:inorder` removes that possibility; if the answer is still wrong, the bug is in the application's logic.
- Stepping through a program in `gdb` and wanting predictable ordering — every launch you step over completes before the next begins.

**Companion flags**:
- `-ll:cpu 1` — single processor reduces noise further; combine for deepest determinism.
- `-lg:delay N` — delay startup by N seconds to attach a debugger before execution.
- `LEGION_BACKTRACE=1` — stack at any failure.
- `DEBUG=1` build — assertion coverage.

## Invariants
- `-lg:inorder` produces **the same logical sequence** as the default — it just removes parallelism. A program correct under the default is correct under `-lg:inorder`; the inverse is not generally true.
- The flag does **not** change the operation graph the runtime builds — only the schedule.
- All other debug aids compose: privilege/bounds/partition checks, freeze-on-error, backtrace, debug build.
- Has no effect on multi-node communication patterns beyond what serializing operations implies (no concurrent collectives, no parallel data movement).
- A program that depends on `-lg:inorder` for correctness has a real bug — see "Failure modes" below.

## Performance implications
- **Massive slowdown** — eliminates all parallelism. Often 10-100× the wall-clock time of a parallel run.
- **Do not measure perf under this flag**; useless for any optimization analysis.
- Useful only for debugging.

## Debug signals
- **Bug disappears under `-lg:inorder`** = timing-dependent. Likely a missing dependence (`pitfalls/false-dependencies-overbroad-privileges.md` in reverse — privilege too narrow), aliased partition (`pitfalls/non-disjoint-disjoint-partition.md`), or coherence misuse.
- **Bug still present under `-lg:inorder`** = not timing-dependent. Bug is in the application's logic; trace via `dataflow-graph.md` and `debug-mode.md` assertions.
- **Wrong answer under `-lg:inorder` but right answer without** → suspicious — unlikely; reproduce carefully, may indicate flag interaction.

## Failure modes
- A program that "works only under `-lg:inorder`" → missing dependence somewhere. The default schedule is exposing a real race. Find the missing privilege, add the missing fence, or fix the coherence declaration.
- Treating `-lg:inorder` as a perf fix → it's not a fix; it's a debug tool.

## Source pointers
- **Reference**: `raw/website-pages/debugging.md`
- **Runtime tree**: https://github.com/StanfordLegion/legion/tree/master/runtime/legion (scheduler in `runtime.cc`)

## Related
- `wiki/concepts/operation-pipeline.md` — what `-lg:inorder` serializes.
- `wiki/concepts/non-interference.md` — the predicate that the flag overrides.
- `wiki/concepts/freeze-on-error.md` — complementary debug aid.
- `wiki/concepts/debug-mode.md` — pair with for fullest coverage.

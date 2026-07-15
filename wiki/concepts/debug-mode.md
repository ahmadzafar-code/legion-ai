---
title: Debug Mode
slug: debug-mode
summary: The make-time flag `DEBUG=1` that enables Legion's full runtime assertions and consistency checks; the first thing to try when an application misbehaves.
tags: [debugging, configuration, for-correctness-debug]
subsystem: legion
layer: tooling
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/website-pages/debugging.md
  - raw/website-pages/getting_started.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/Makefile
related:
  - wiki/concepts/privilege-checks.md
  - wiki/concepts/bounds-checks.md
  - wiki/concepts/partition-checks.md
  - wiki/concepts/freeze-on-error.md
  - wiki/concepts/backtrace-mode.md
  - wiki/concepts/delay-start.md
  - wiki/concepts/logger-categories.md
  - wiki/concepts/error-message-catalog.md
---

## TL;DR
`DEBUG=1 make` builds Legion with full runtime assertions enabled: privilege checks, region containment checks, partition checks, internal consistency invariants, and many others. Release builds (`DEBUG=0`, the perf default) strip all of these for speed. The website-debugging.md docs say it plainly: **for any Legion application that's not behaving as expected, the first debugging technique should always be to compile in debug mode**. The confusion: `DEBUG=1` does not enable `-DPRIVILEGE_CHECKS` and `-DBOUNDS_CHECKS` — those are separate `CC_FLAGS` that need to be added on top.

## Mental model
`DEBUG=1` is Legion's `assert()` switch — turns on every internal consistency check the runtime authors thought of, at the cost of substantial perf overhead. It's the foundational debug build and the precondition for sensible behavior of more targeted flags (privilege/bounds/partition checks layered on top).

## Mechanism & API
At build time:
```bash
DEBUG=1 make
./app
```

What `DEBUG=1` activates (per `raw/website-pages/debugging.md`):
- Internal runtime assertions across `runtime/legion/`, `runtime/realm/`, and `runtime/mappers/`.
- Consistency checks on task launches, region requirements, partition operations.
- Sanity checks the release path strips for performance.

What it does **not** activate (require separate flags):
- `-DPRIVILEGE_CHECKS` — accessor-level privilege verification (`privilege-checks.md`).
- `-DBOUNDS_CHECKS` — accessor-level bounds verification (`bounds-checks.md`).
- `-DLEGION_SPY` — Legion Spy logging (the visualization-mode `-lg:spy` runtime flag works in any build).
- `-DFULL_SIZE_INSTANCES` — full-region instance allocation for catching layout bugs.

Standard debug-build configuration:
```bash
CC_FLAGS="-DPRIVILEGE_CHECKS -DBOUNDS_CHECKS" DEBUG=1 make
./app -lg:partcheck
LEGION_BACKTRACE=1 LEGION_FREEZE_ON_ERROR=1 ./app
```

## Invariants
- `DEBUG=1` adds **no semantic change** to correct programs — only adds verification.
- Programs that fail under `DEBUG=1` but succeed in release are nearly always real bugs that happen to produce correct output without verification.
- Programs that succeed under `DEBUG=1` but fail in release suggest non-determinism or relied-on debug-mode side-effects (rare; investigate).
- Debug builds are typically **2-10× slower** than release; do not measure perf in this mode.
- Compatible with all other debug aids: privilege/bounds/partition checks, freeze-on-error, backtrace, in-order execution.

## Performance implications
- `DEBUG=1` builds are **much slower** than release. Use only for development and debugging.
- Use `DEBUG=0` (release) for all performance measurement; otherwise `legion-prof.md` data is dominated by check overhead.
- Profiling under `DEBUG=1` is **strongly discouraged** per `raw/website-pages/profiling.md` — the timing is dominated by assertion overhead, not application work.

## Debug signals
- **Assertion failure printed at runtime** in a debug build → real bug; the message identifies the file and line. Combine with `LEGION_BACKTRACE=1` for a stack.
- **Program runs slower under debug** but **gives correct results** → expected; just confirms release-build correctness as far as it goes.
- **Program crashes only under debug** → an internal invariant violation; report to Legion maintainers with the assertion text and a minimal repro.

## Failure modes
- Forgetting to switch back to `DEBUG=0` for perf measurement → misleading profiles.
- Conflating `DEBUG=1` with `-DPRIVILEGE_CHECKS`/`-DBOUNDS_CHECKS` — they're orthogonal; for full coverage add both.

## Source pointers
- **Reference**: `raw/website-pages/debugging.md`
- **Build system**: https://github.com/StanfordLegion/legion (`Makefile`)
- **Getting started**: `raw/website-pages/getting_started.md`

## Related
- `wiki/concepts/privilege-checks.md` — compose with `DEBUG=1` for access-correctness coverage.
- `wiki/concepts/bounds-checks.md` — likewise for bounds.
- `wiki/concepts/partition-checks.md` — runtime flag; orthogonal to build mode.
- `wiki/concepts/freeze-on-error.md` — env-var debug aid; compose with `DEBUG=1`.

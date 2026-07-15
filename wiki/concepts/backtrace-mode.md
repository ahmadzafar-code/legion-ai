---
title: Backtrace Mode
slug: backtrace-mode
summary: Environment variable `LEGION_BACKTRACE=1` (or `REALM_BACKTRACE=1`, same effect) that prints a call-stack at any runtime error or signal; the cheapest "what was happening when it crashed?" debug aid.
tags: [debugging, configuration, for-correctness-debug]
subsystem: legion
layer: tooling
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/website-pages/debugging.md
github:
  - https://github.com/StanfordLegion/legion/tree/master/runtime
related:
  - wiki/concepts/freeze-on-error.md
  - wiki/concepts/debug-mode.md
  - wiki/concepts/legion-spy.md
  - wiki/concepts/event-poisoning.md
---

## TL;DR
`LEGION_BACKTRACE=1` (alias: `REALM_BACKTRACE=1`) tells the Legion runtime to print a full call-stack on any runtime assertion failure, signal, or segfault. It's the first env var to set when chasing a crash — costs nothing on the happy path, and turns "Segmentation fault" into "here's where it died". The confusion: this is independent of `DEBUG=1`. Backtrace mode works in any build, including release; it just adds stack-trace printing at error sites.

## Mental model
`LEGION_BACKTRACE=1` is `addr2line` baked into the runtime's signal handlers. When something goes wrong, instead of a bare process exit, you get a frames-list pointing at the failing code path. For multi-node debugging this is the cheapest first move; for `freeze-on-error.md` workflows it's the standard companion.

## Mechanism & API
Set at runtime:
```bash
LEGION_BACKTRACE=1 ./app
```

For MPI launches:
```bash
mpirun -np 4 -x LEGION_BACKTRACE=1 ./app
```

Both `LEGION_BACKTRACE` and `REALM_BACKTRACE` are accepted spellings, with identical effect (per `raw/website-pages/debugging.md`).

What it adds:
- Signal handlers (`SIGSEGV`, `SIGABRT`, etc.) print a backtrace before letting the process exit.
- Runtime assertion failures print a backtrace before aborting.
- The backtrace is **per-thread** for the failing thread.

**Companion env vars and flags**:
- `LEGION_FREEZE_ON_ERROR=1` — pause the process for gdb attach. Pair with `LEGION_BACKTRACE=1` so you see *both* the trace and can step through.
- `REALM_SHOW_EVENT_WAITERS=N+M` — dump pending event waiters (good for deadlocks, where there is no signal but the program hangs).
- `-ll:force_kthreads` — force Realm threads to be kernel-visible; necessary for `thread apply all bt` in gdb to see them.

**Demangling**: stacks include mangled C++ names. Pipe through `c++filt` if you want human-readable signatures:
```bash
LEGION_BACKTRACE=1 ./app 2>&1 | c++filt
```

## Invariants
- `LEGION_BACKTRACE=1` triggers only on **runtime-recognized failures** (signals, assertions). A wrong-answer bug with no crash will not produce output.
- Adds **no semantic change**; pure observation.
- Compatible with any build: debug or release.
- Compatible with all other debug aids: privilege/bounds/partition checks, freeze-on-error, in-order execution.
- Multi-node MPI launches require the variable to be **propagated** (`-x LEGION_BACKTRACE=1`).

## Performance implications
- Zero overhead on the happy path — only registers signal handlers at startup.
- Safe to leave on for long-running production jobs as a post-mortem capability.
- No effect on `legion-prof.md` measurements.

## Debug signals
- **Backtrace prints "[task X]" or "[realm runtime]" frames** at the top → identifies which subsystem hit the failure.
- **No backtrace despite a crash** → either the env var didn't propagate (check MPI `-x`) or the crash was in a stripped third-party library outside Legion's signal handler.
- **Combined with `LEGION_FREEZE_ON_ERROR=1`**: backtrace prints, then process freezes; attach gdb for live inspection.

## Failure modes
- Forgetting `-x` in MPI launches → backtrace never fires; debug looks like it isn't working.
- Stripped binary → backtrace prints offsets without symbol names; rebuild with `-g` or use `addr2line` to decode.

## Source pointers
- **Reference**: `raw/website-pages/debugging.md`
- **Runtime tree (signal handlers)**: https://github.com/StanfordLegion/legion/tree/master/runtime

## Related
- `wiki/concepts/freeze-on-error.md` — pair with for live post-mortem.
- `wiki/concepts/debug-mode.md` — assertion coverage that backtrace will then surface.
- `wiki/concepts/legion-spy.md` — for correctness-analysis post-mortem.

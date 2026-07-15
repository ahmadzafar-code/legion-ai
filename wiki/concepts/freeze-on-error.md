---
title: Freeze on Error
slug: freeze-on-error
summary: Environment variable `LEGION_FREEZE_ON_ERROR=1` that pauses the offending process at the moment of a fatal error and prints its PID; the standard multi-node debugger-attach workflow.
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
  - wiki/concepts/legion-spy.md
  - wiki/concepts/privilege-checks.md
  - wiki/workflows/debug-perf-bottleneck.md
  - wiki/concepts/debug-mode.md
  - wiki/concepts/in-order-execution.md
  - wiki/concepts/backtrace-mode.md
  - wiki/concepts/delay-start.md
  - wiki/concepts/event-poisoning.md
---

## TL;DR
`LEGION_FREEZE_ON_ERROR=1` is an environment variable that tells Legion to **pause** the process when it hits a fatal error, print its PID, and wait. You attach a debugger (`gdb -p <PID>`) and inspect the live state at the exact moment of failure. The confusion: this is most useful for **multi-node distributed runs**, where bugs reproduce on specific nodes under specific timing. Without freeze-on-error, the offending process exits before you can attach.

## Mental model
`LEGION_FREEZE_ON_ERROR` is "post-mortem debugger trigger built into the runtime": instead of dumping a core and exiting, the runtime stops in place so you can step into a debugger and look at thread state, memory, and pending event waiters. Combined with `LEGION_BACKTRACE=1` for a stack trace and the pending-event dumps from `REALM_SHOW_EVENT_WAITERS`, it gives you a full forensic view.

## Mechanism & API
Set at runtime:
```bash
LEGION_FREEZE_ON_ERROR=1 ./app
```

For MPI launches:
```bash
mpirun -np 4 -x LEGION_FREEZE_ON_ERROR=1 ./app
```

What happens when a fatal error occurs (per `raw/website-pages/debugging.md`):
1. The runtime prints a message indicating the process is frozen, along with the PID.
2. The process stays in an infinite wait — Legion's threads halt rather than letting the process exit.
3. From a shell on the same node:
   ```bash
   gdb -p <PID>
   ```
   The debugger attaches; you can `bt`, inspect threads (`info threads`, `thread apply all bt`), examine memory, etc.
4. After investigation, `continue` or `kill` from gdb to let the process proceed or terminate.

**Companion environment variables**:
- `LEGION_BACKTRACE=1` (or `REALM_BACKTRACE=1`, same effect) — prints a stack trace at the error site.
- `REALM_SHOW_EVENT_WAITERS=60+5` — dumps pending event waiters after 60s and every 5s thereafter (useful when the bug is a deadlock rather than a hard error).
- `-ll:force_kthreads` runtime flag — makes Realm threads visible to gdb (otherwise some are user-level fibers).

**Typical workflow** for hard-to-reproduce distributed bugs:
1. Reproduce locally first with `LEGION_BACKTRACE=1` to get a stack.
2. If multi-node only: run with `LEGION_FREEZE_ON_ERROR=1` on the suspect node.
3. SSH into the frozen node, attach gdb, inspect.

## Invariants
- `LEGION_FREEZE_ON_ERROR=1` triggers **only on fatal runtime errors** (assertions, signals, internal checks) — not on application logical errors like a wrong-answer result.
- The process stays frozen indefinitely; the user must `kill` or `continue` it after debugging.
- Useful with **any** other debug aid (privilege/bounds checks, debug mode, partition checks). Combine freely.
- The freeze is **per-process**: in MPI runs, only the failing process freezes; other ranks continue (and may hang waiting on it).
- No effect on correct programs that don't hit a fatal error.

## Performance implications
- Zero overhead on the happy path — only triggered at fatal error.
- Production setting; safe to leave on for long-running jobs where you want post-mortem capability.

## Debug signals
- **"Process frozen, PID = <N>"** message in the application's stderr → the trigger has fired; attach gdb.
- **No freeze despite an apparent error**: the error wasn't fatal to the runtime (e.g., a wrong return value isn't fatal). Use other tools.
- **Multi-node hang** with `LEGION_FREEZE_ON_ERROR=1` on → one node is frozen for debug, others are waiting on it. Expected.

## Failure modes
- Forgetting to `-x LEGION_FREEZE_ON_ERROR=1` for the MPI launcher → environment variable doesn't propagate → freeze doesn't fire.
- Attaching gdb without `-ll:force_kthreads` → some Realm threads invisible; missing context.

## Source pointers
- **Reference**: `raw/website-pages/debugging.md`
- **Runtime tree**: https://github.com/StanfordLegion/legion/tree/master/runtime (error-handling paths)

## Related
- `wiki/concepts/legion-spy.md` — for the correctness-analysis side of post-mortem investigation.
- `wiki/concepts/privilege-checks.md` — most common source of fatal errors that trigger freeze.
- `wiki/workflows/debug-perf-bottleneck.md` — workflow that uses these together.

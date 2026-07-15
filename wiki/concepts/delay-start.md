---
title: Delay Start
slug: delay-start
summary: Runtime flag `-lg:delay N` that sleeps for N seconds at startup before running the top-level task; the standard way to attach gdb before the program begins executing.
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
  - wiki/concepts/backtrace-mode.md
  - wiki/concepts/debug-mode.md
---

## TL;DR
`-lg:delay N` makes the Legion runtime sleep for N seconds at startup before launching the top-level task. The window lets you locate the process's PID and attach gdb (`gdb -p <PID>`) so you can set breakpoints, watch variables, and step through execution from the very beginning. The confusion: this is **not** for debugging crashes — that's `freeze-on-error.md`. Delay-start is for *pre-execution* attach, when you want to set breakpoints in code that runs immediately and would otherwise execute before you can attach.

## Mental model
`-lg:delay` is the "gdb-before-`main`" workflow: instead of running and missing what you wanted to watch, you tell the runtime to wait N seconds so you can attach in peace. Standard tool for breaking in custom mapper code, top-level task startup logic, or any code that runs near the beginning of execution.

## Mechanism & API
Pass at runtime:
```bash
./app -lg:delay 30
```

The runtime prints something like `Delaying start by 30 seconds; PID = 12345`, then sleeps. During the wait:
```bash
gdb -p 12345
(gdb) break my_function
(gdb) continue
```

Multi-node:
```bash
mpirun -np 4 ./app -lg:delay 60
```
Each node prints its own PID; attach to whichever node hosts the code you want to inspect.

**Combine with `LEGION_FREEZE_ON_ERROR=1`** for the full pre/post-execution attach workflow:
```bash
LEGION_FREEZE_ON_ERROR=1 ./app -lg:delay 30
```
- Attach pre-execution at PID printed at startup.
- If the process subsequently hits a runtime error, it freezes; re-use the same gdb session.

**Combine with `-ll:force_kthreads`** so all Realm threads are kernel-visible to gdb:
```bash
./app -lg:delay 30 -ll:force_kthreads
```

## Invariants
- `-lg:delay N` blocks only at startup, before the top-level task begins. Subsequent execution is unaffected.
- The delay value is **per-process**; multi-node jobs delay every process by the same amount.
- The runtime prints its PID before sleeping, so the user can find it.
- Compatible with every other debug aid; no semantic effect on the program.
- Setting `N = 0` is the default (no delay).

## Performance implications
- Adds **N seconds** to startup. Acceptable for debug runs.
- Do not use in production runs or perf measurements — pure debug aid.

## Debug signals
- **"Delaying start by N seconds; PID = X"** at startup → the trigger fired; attach gdb to `X`.
- **No PID printed despite `-lg:delay`** → the flag wasn't parsed (check spelling, position before `--`).
- **Attached gdb sees no threads of interest** → use `-ll:force_kthreads` to expose Realm fibers.

## Failure modes
- Forgetting `-ll:force_kthreads` → many Realm threads invisible to gdb.
- Setting N too short → process is already past the function of interest when you attach.

## Source pointers
- **Reference**: `raw/website-pages/debugging.md`
- **Runtime tree**: https://github.com/StanfordLegion/legion/tree/master/runtime

## Related
- `wiki/concepts/freeze-on-error.md` — post-execution complement.
- `wiki/concepts/backtrace-mode.md` — pair with for stack-on-failure plus pre-execution attach.
- `wiki/concepts/debug-mode.md` — full assertion coverage to surface bugs you'd watch with gdb.

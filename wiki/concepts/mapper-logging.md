---
title: Mapper Logging
slug: mapper-logging
summary: The `LoggingWrapper` class and `-level mapper=2` runtime flag that together log every mapper callback's inputs and outputs; the standard tool for debugging custom mapper behavior.
tags: [mapping, debugging, tooling, for-perf-debug, for-correctness-debug]
subsystem: legion
layer: tooling
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/website-pages/mapper.md
  - raw/website-pages/debugging.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/mappers/logging_wrapper.h
related:
  - wiki/concepts/mapper.md
  - wiki/concepts/mapper-callback.md
  - wiki/concepts/default-mapper.md
  - wiki/concepts/legion-prof.md
  - wiki/concepts/logger-categories.md
  - wiki/concepts/logging-wrapper.md
---

## TL;DR
`LoggingWrapper` is a `Mapper` subclass that wraps another `Mapper` instance, intercepting every callback and writing detailed logs of inputs and outputs before delegating. Combined with `-level mapper=2` at runtime, you get a complete trace of which decisions the mapper made for every task, copy, and inline operation. The confusion: `LoggingWrapper` is not on by default. You replace your mapper's registration with `new LoggingWrapper(new MyMapper(...))` to enable it; without that wrap, no callback logging happens regardless of `-level`.

## Mental model
`LoggingWrapper` is the mapper equivalent of `strace`: a transparent intermediary that records every "system call" the runtime makes into the mapper and every response the mapper gives back. Wrap, run, read logs, unwrap when done. Standard development-loop tool for anyone writing a custom mapper.

## Mechanism & API
**Wrap your mapper at registration**:
```cpp
#include "mappers/logging_wrapper.h"

void mapper_registration(Machine machine, Runtime *runtime,
                         const std::set<Processor> &local_procs) {
  for (auto p : local_procs) {
    auto *underlying = new MyMapper(runtime->get_mapper_runtime(), machine, p, "my");
    runtime->replace_default_mapper(new LoggingWrapper(underlying), p);
  }
}
```

**Run with**:
```bash
./app -level mapper=2 -logfile mapper_%.log
```

`-level mapper=2` sets the logging category `mapper` to level 2 (`debug`). Output goes to per-node files (`mapper_0.log`, `mapper_1.log`, ...) via the `%` substitution in `-logfile`.

**What gets logged**:
- For every callback (`select_task_options`, `slice_task`, `map_task`, `select_tasks_to_map`, etc.): inputs (task ID, region requirements, valid instances), outputs (chosen instances, target procs, priorities, slices), elapsed time inside the callback.
- Cross-mapper messages and event waits.

**`LoggingWrapper` is itself a `Mapper`** — it implements every callback, logs, and delegates to the wrapped instance. Compose with other wrappers freely.

**Tactical pattern**: keep the wrapper in place until you understand the mapper's behavior, then remove it for production runs to avoid log-write overhead.

## Invariants
- `LoggingWrapper` is a **pure pass-through** for behavior — it changes only what's logged, not what the mapper decides.
- The wrapper takes **ownership** of the wrapped pointer; do not also delete the inner mapper.
- Logging level `mapper=2` (debug) is the minimum that captures full callback IO. `mapper=3` (info) is a quieter mode.
- Multi-node runs need `-logfile pattern_%.log` so per-node logs go to separate files.
- The wrapped mapper sees normal callbacks — wrapping does not affect the mapper's own `MapperContext` usage.

## Performance implications
- **Logging is not free.** Per-callback log writes cost real time, especially with `mapper=2`. Strip the wrapper for perf measurement.
- Useful even with `DEBUG=0` builds; the wrapper is a Legion library class, not a build-time check.
- Can be left in for distributed runs (`mpirun ...`) — each node writes its own log.

## Debug signals
- **Empty logs despite `-level mapper=2`**: the wrapper isn't in place. Verify `replace_default_mapper(new LoggingWrapper(...), p)`.
- **Logs but no useful info**: the level is too coarse — bump to `-level mapper=2` (or `mapper=1` for the verbose `spew` level).
- **First callback that returns unexpected output**: typically the source of a downstream bug. Read in chronological order.
- **Specific recipes** (per `raw/website-pages/debugging.md`):
  - GPU-eligible task on CPU rows → search for the `chosen_variant` and `target_procs` in the relevant `map_task` log entry.
  - Mapper bouncing between processors → grep the `target_procs` field across iterations of the same task ID.
  - Slow callbacks → wrapper logs include elapsed times; look for outliers.

## Failure modes
- Forgetting the wrapper but expecting mapper logs → no output, confusion. Always confirm wrapper registration first.
- Leaving `LoggingWrapper` in a perf-measurement build → measurement skewed.

## Source pointers
- **Header**: https://github.com/StanfordLegion/legion/blob/master/runtime/mappers/logging_wrapper.h
- **Reference (debugging)**: `raw/website-pages/debugging.md`
- **Reference (mapper)**: `raw/website-pages/mapper.md`

## Related
- `wiki/concepts/mapper.md` — what's being logged.
- `wiki/concepts/mapper-callback.md` — the methods captured by the wrapper.
- `wiki/concepts/default-mapper.md` — typical wrap target.
- `wiki/concepts/legion-prof.md` — pair with this for end-to-end mapper-decision debugging.

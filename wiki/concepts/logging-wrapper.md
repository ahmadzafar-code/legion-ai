---
title: LoggingWrapper
slug: logging-wrapper
summary: A drop-in mapper subclass that intercepts every callback and logs inputs and outputs; the standard development tool for debugging custom mapper behavior.
tags: [mapping, debugging, tooling, for-perf-debug, for-correctness-debug]
subsystem: legion
layer: tooling
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/website-pages/debugging.md
  - raw/website-pages/mapper.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/mappers/logging_wrapper.h
related:
  - wiki/concepts/mapper.md
  - wiki/concepts/mapper-callback.md
  - wiki/concepts/mapper-logging.md
  - wiki/concepts/logger-categories.md
  - wiki/concepts/default-mapper.md
---

## TL;DR
`LoggingWrapper` is a concrete `Mapper` subclass shipped in `runtime/mappers/logging_wrapper.h`. You wrap any other mapper instance with it at registration time, and every mapper callback (its inputs, outputs, elapsed time) gets logged via the `mapper` category. The wrapper is a **pure pass-through** for behavior — it changes only what's logged, not what the mapper decides. The confusion: this is the *concept page* for the class; `mapper-logging.md` is the broader page on mapper-debug workflow that includes `-level mapper=2` configuration. Use them together.

## Mental model
`LoggingWrapper` is the decorator pattern applied to Legion's mapper interface: intercept every callback, log it, delegate. The wrapped mapper sees normal callbacks; the user sees a trace of every decision. It's a development-only tool — strip it for production runs because the per-callback log overhead is real.

## Mechanism & API
**Wrap at registration**:
```cpp
#include "mappers/logging_wrapper.h"

void mapper_registration(Machine machine, Runtime *runtime,
                         const std::set<Processor> &local_procs) {
  for (auto p : local_procs) {
    auto *underlying = new MyMapper(runtime->get_mapper_runtime(),
                                    machine, p, "my");
    runtime->replace_default_mapper(new LoggingWrapper(underlying), p);
  }
}
Runtime::add_registration_callback(mapper_registration);
```

The wrapper **takes ownership** of the wrapped pointer; do not also delete the inner mapper.

**Run with `-level mapper=2`** to actually see the logs (see `logger-categories.md`):
```bash
./app -level mapper=2 -logfile mapper_%.log
```

**What gets logged**:
- Every callback (`select_task_options`, `slice_task`, `select_tasks_to_map`, `map_task`, `select_sharding_functor`, `select_steal_targets`, etc.) — inputs, outputs, elapsed time.
- Cross-mapper messages.
- Mapper-event triggers and waits.

The wrapper is itself a full `Mapper` implementation — it implements every callback, logs, and delegates. You can compose: wrap a wrapped mapper.

## Invariants
- `LoggingWrapper` is **semantically transparent** — it does not change the wrapped mapper's decisions.
- The wrapper inherits the wrapped mapper's **concurrency mode** (default-serialized or `concurrent`).
- Output goes to the `mapper` log category; default level is too coarse to see callbacks. Set `-level mapper=2`.
- Per-node log files require `-logfile pattern_%.log` so output doesn't interleave.
- The wrapper's overhead is per-callback log-write cost; do not measure perf with it active.

## Performance implications
- **Not free** — measurable per-callback overhead, especially at `mapper=2` verbosity.
- Strip for performance measurement.
- Keep during development; unwrap before production.

## Debug signals
- **Mapper bouncing** (`pitfalls/mapper-bouncing.md`) → grep `target_procs` across iterations of the same task ID.
- **GPU underutilization** (`pitfalls/gpu-underutilization.md`) → search the `chosen_variant` and `target_procs` for the failing task.
- **`pitfalls/mapper-stalls.md`** → check per-callback elapsed times; outliers identify the slow callbacks.
- **Empty logs despite `-level mapper=2`**: the wrapper isn't actually in place. Confirm registration.

## Failure modes
- Forgetting `replace_default_mapper(new LoggingWrapper(...), p)` and expecting logs → no output.
- Leaving the wrapper in for production runs → measurable overhead.

## Source pointers
- **Header**: https://github.com/StanfordLegion/legion/blob/master/runtime/mappers/logging_wrapper.h
- **Reference (debugging)**: `raw/website-pages/debugging.md`
- **Reference (mapper)**: `raw/website-pages/mapper.md`

## Related
- `wiki/concepts/mapper.md` — what's being wrapped.
- `wiki/concepts/mapper-callback.md` — what's being intercepted.
- `wiki/concepts/mapper-logging.md` — broader workflow concept (wrapping + level + log routing).
- `wiki/concepts/logger-categories.md` — `-level mapper=N` and `-logfile pattern_%.log` mechanism.
- `wiki/concepts/default-mapper.md` — typical wrap target.

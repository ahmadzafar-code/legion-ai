---
title: Logger Categories
slug: logger-categories
summary: Legion's category-based logging infrastructure with six severity levels (spew/debug/info/print/warning/error); configured at runtime via `-level cat=N` and routed via `-logfile pattern_%.log`.
tags: [debugging, configuration, tooling, for-correctness-debug, for-perf-debug]
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
  - wiki/concepts/mapper-logging.md
  - wiki/concepts/legion-spy.md
  - wiki/concepts/legion-prof.md
  - wiki/concepts/debug-mode.md
---

## TL;DR
Legion has a category-based logger: code registers a category (e.g., `default_mapper`, `tasks`, `legion_spy`, `dma`, `replication`) and emits at one of six severity levels (spew, debug, info, print, warning, error). At runtime you set per-category levels via `-level cat=N` and route output to per-node files via `-logfile pattern_%.log`. The confusion: there's a default level for "all other categories" — `-level legion_spy=2,3` sets `legion_spy` to debug (2) **and** every other category to info (3). The trailing number is the default.

## Mental model
Logger categories are Legion's `RUST_LOG` / `glog --vmodule`: per-subsystem severity dials, with a global fallback. Useful for "show me everything the mapper did but nothing else" or "trace replication while keeping the rest quiet".

## Mechanism & API
**Creating a category** (in C++ code):
```cpp
static Realm::Logger log_mapper("default_mapper");

void MyMapper::map_task(...) {
  log_mapper.debug() << "Choosing variant " << vid << " on " << proc;
}
```

**Severity levels** (per `raw/website-pages/debugging.md`):
| Level | Name | When to emit |
|---|---|---|
| 1 | spew | extremely detailed tracing |
| 2 | debug | per-operation detail |
| 3 | info | per-phase summary |
| 4 | print | high-level structure (defaults to user terminal) |
| 5 | warning | non-fatal issues |
| 6 | error | failures |

**Runtime configuration**:
```bash
./app -level tasks=4,legion_spy=2,3 -logfile prof_%.log
```
Parsed as:
- `tasks=4` → set the `tasks` category to level 4 (print).
- `legion_spy=2` → set the `legion_spy` category to level 2 (debug).
- `,3` (trailing) → default for every *other* category is level 3 (info).

**Log routing**:
- `-logfile pattern_%.log` → each node's output goes to `pattern_0.log`, `pattern_1.log`, etc. The `%` substitutes the node index.
- Without `-logfile`, output goes to stderr.

**Built-in category names you'll see**:
- `tasks` — task scheduling decisions.
- `mapper` — mapper-callback flow.
- `legion_spy` — Spy logging when paired with `-lg:spy`.
- `dma` — Realm DMA system operations.
- `replication` — control-replication shard activity.
- `trace` — `tracing.md` record/replay events.
- `legion_gc` — garbage-collection events (paired with `-DLEGION_GC` build).
- `allocation` — instance allocation tracing (paired with `-DTRACE_ALLOCATION` build).
- `default_mapper` — `DefaultMapper`'s own logs.

## Invariants
- Each category has **independent** verbosity; setting one does not affect others.
- The trailing-number default applies only to **uncovered** categories.
- Log files are per-node when `-logfile pattern_%.log` is used; for single-node runs the `%` substitutes `0`.
- Output is line-buffered; long-running programs flush periodically.
- Categories are case-sensitive.

## Performance implications
- Higher-verbosity levels generate more output → more I/O cost. Setting to `mapper=1` (spew) on a hot mapper can slow runs significantly.
- For perf measurement, leave categories at default or set `=4` (print) for minimal noise.
- For *debugging*, `mapper=2` + `tasks=2` is the standard pairing.

## Debug signals
- **Mapper logs missing despite expected output** → wrap the mapper with `mapper-logging.md`'s `LoggingWrapper` AND set `-level mapper=2`. Both are required.
- **Massive log files** → some category is at spew (level 1). Find the culprit; lower the level.
- **Inconsistent log content between runs** → a category you didn't explicitly set might be inheriting the trailing-default; pin it explicitly.

## Failure modes
- Setting `-level cat=N` for a category that doesn't exist → silently ignored.
- Forgetting the `-logfile pattern_%.log` for multi-node → interleaved per-rank output is hard to read.

## Source pointers
- **Reference**: `raw/website-pages/debugging.md`
- **Runtime (Realm Logger)**: https://github.com/StanfordLegion/legion/tree/master/runtime

## Related
- `wiki/concepts/mapper-logging.md` — `LoggingWrapper` + `-level mapper=2` is the standard pairing.
- `wiki/concepts/legion-spy.md` — `-level legion_spy=2` produces Spy logs.
- `wiki/concepts/legion-prof.md` — orthogonal but often run together.
- `wiki/concepts/debug-mode.md` — pair with this for full diagnostic coverage.

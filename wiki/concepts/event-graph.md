---
title: Event Graph
slug: event-graph
summary: Legion Spy's `-e` output; renders the post-mapping Realm event graph (per-point dependencies, copies, fills, task spawns) of an application; the precise causality view that complements Legion Prof's timeline.
tags: [profiling, debugging, tooling, for-perf-debug, for-correctness-debug]
subsystem: cross
layer: tooling
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/website-pages/debugging.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/tools/legion_spy.py
related:
  - wiki/concepts/legion-spy.md
  - wiki/concepts/event.md
  - wiki/concepts/dataflow-graph.md
  - wiki/concepts/physical-analysis.md
  - wiki/concepts/timeline-view.md
---

## TL;DR
The event graph is Legion Spy's `-e` output: a `dot` rendering of the **Realm event graph** that physical analysis built for the run. Every node is an operation or a Realm event; every edge is a precondition. Where the `dataflow-graph.md` shows logical-analysis output (one node per logical operation), the event graph shows post-mapping precision — each point task gets its own node, each copy/fill is shown explicitly, and you can trace any individual point's preconditions back through the graph. The confusion: the event graph can be much larger than the dataflow graph for the same program. An index launch is one dataflow node but N event-graph nodes plus all the per-point copies physical analysis emitted.

## Mental model
The event graph is what Realm actually executed; the dataflow graph is what Legion intended. When the two disagree, the difference is what physical analysis decided — extra copies for instance placement, fan-out from index launches, fills the runtime inserted for `WRITE_DISCARD` semantics. Reading the event graph tells you *why this exact run took the shape it did* in Legion Prof's `timeline-view.md`.

## Mechanism & API
**Capture** (per `raw/website-pages/debugging.md`):
```bash
./app -lg:spy -logfile spy_%.log
legion/tools/legion_spy.py -ez spy_*.log
```

Flags:
- `-e` — emit the event graph.
- `-z` — read gzipped logs (if `USE_ZLIB=1`).
- Combine with `-d` (`-dez`) to also emit `dataflow-graph.md`.

Output: a `dot` file plus rendered `.pdf`/`.png` graphs in the output directory.

**Reading the graph**:
- Nodes are operations (tasks, copies, fills, reductions) or named events.
- Edges are preconditions: `A → B` means B's start event waits on A's completion event.
- Per-point fan-out for index launches: a single index launch shows as N nodes (one per point) plus any preceding fan-out copies.
- Color/style conventions: tasks vs copies vs fills typically render differently; refer to the `tools/legion_spy.py` source for the exact dot styling.

**Pairing with profiles**: load `-lg:prof` and `-lg:spy` logs together to overlay event-graph causality on the timeline:
```bash
./app -lg:prof <N> -lg:spy -lg:prof_logfile prof_%.gz -logfile spy_%.log
```

**Naming objects for readable graphs**:
```cpp
runtime->attach_name(is, "input_is");
runtime->attach_name(lr, "input_lr");
```
Names appear on the corresponding event-graph nodes.

## Invariants
- The event graph is the **runtime's actual schedule** — every dependency in it was a real wait the application paid.
- The graph is a DAG; cycles indicate a bug (in either the runtime or the application's coherence usage).
- Nodes correspond 1:1 with Realm events; some events have many waiters (a single `merge_events` precondition).
- Per-point dependencies emerge here — see `physical-analysis.md`. They are not in the logical dataflow graph.
- Naming a region/partition via `attach_name` only changes the labels; it has no semantic effect.

## Performance implications
- The event graph is the **causality complement** to the timeline view's wall-clock view. Together they answer "what happened, when, and why".
- Excess edges between supposedly-parallel point tasks indicate physical-analysis decided they conflict (often instance-aliasing or coherence) — investigate `physical-instance.md` placement.
- Heavy fill/copy nodes between launches → over-broad `physical-instance.md` lifetime or unintended cross-memory placement.
- Long chains in the event graph correspond to `critical-path.md` chains in the timeline; matching the two confirms where the bottleneck is.

## Debug signals
- **Cycle in the graph** = deadlock or coherence misuse. Combine with `REALM_SHOW_EVENT_WAITERS` + `tools/detect_loops` for runtime-side diagnosis.
- **Edges between same-region point tasks** of an index launch when you expected parallelism → physical analysis decided they overlap; check partition disjointness (`partition-checks.md`) or coherence (`coherence-mode.md`).
- **Many small fill nodes preceding a task** → `WRITE_DISCARD` not declared where it could be; the runtime is initializing the instance because the task didn't promise to overwrite.

## Failure modes
- Capturing event-graph logs on a long-running program → enormous files; constrain to a representative window or run a smaller input.
- Reading the graph without knowing what `attach_name` was called → labels are runtime-generated IDs, hard to map back to source.

## Source pointers
- **Tool**: https://github.com/StanfordLegion/legion/blob/master/tools/legion_spy.py
- **Reference**: `raw/website-pages/debugging.md`

## Related
- `wiki/concepts/legion-spy.md` — host tool.
- `wiki/concepts/event.md` — the primitive each graph node represents.
- `wiki/concepts/dataflow-graph.md` — Spy's complementary `-d` output (logical view).
- `wiki/concepts/physical-analysis.md` — what produces the event graph.
- `wiki/concepts/timeline-view.md` — pair with this for time-and-causality debugging.

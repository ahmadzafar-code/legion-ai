---
title: Tunable Variable
slug: tunable-variable
summary: A mapper-supplied, machine-dependent constant that an application queries via `get_tunable_value`; the standard way to parameterize a program on hardware properties (CPU count, GPU count, partition factor) without baking them into the code.
tags: [configuration, mapping, for-program-reasoning]
subsystem: legion
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/10_custom_mappers.md
  - raw/website-pages/mapper.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
related:
  - wiki/concepts/mapper.md
  - wiki/concepts/default-mapper.md
  - wiki/concepts/future.md
  - wiki/concepts/task.md
---

## TL;DR
A tunable variable is a typed constant the application asks the mapper for at runtime — "how many subregions should I partition into?" or "what's a good GPU batch size on this machine?" — via `runtime->get_tunable_value(ctx, TUNABLE_ID, mapper_id)`. The mapper answers via the `select_tunable_value` callback, computing the value from `Machine` queries. The return is a `Future` so the answer can be deferred. The confusion: tunables are *not* runtime configuration flags (`-ll:*`/`-lg:*`) — those configure the runtime; tunables configure the *application* based on the configured runtime.

## Mental model
Tunables are configuration constants whose values you want a *policy* (the mapper) to choose, not a value you want hard-coded. Where `-ll:cpu 16` tells Realm to create 16 CPU processors, a `SUBREGION_TUNABLE` lets your app ask "given the configuration I'm actually running on, into how many subregions should I partition this data?" — and have the mapper consult `Machine` to answer.

## Mechanism & API
**Application side**:
```cpp
enum TunableIDs { SUBREGION_TUNABLE = 0 };

int num_subregions = runtime->get_tunable_value(
    ctx, SUBREGION_TUNABLE, PARTITIONING_MAPPER_ID
).get_result<size_t>();
```

**Mapper side** (in a `DefaultMapper` subclass):
```cpp
void PartitioningMapper::select_tunable_value(
    const MapperContext ctx, const Task &task,
    const SelectTunableInput &in, SelectTunableOutput &out) override {
  if (in.tunable_id == SUBREGION_TUNABLE) {
    Machine::ProcessorQuery pq(machine); pq.only_kind(Processor::LOC_PROC);
    size_t cpus = std::distance(pq.begin(), pq.end());
    runtime->pack_tunable<size_t>(cpus, out);
  }
}
```

`get_tunable_value` returns a `Future`; calling `get_result<T>()` blocks until the mapper has responded. The future representation means tunable queries can be issued early and consumed later, overlapping with other work.

## Invariants
- Tunable IDs are application-defined enum values; namespace them per mapper.
- The mapper is **the authority**: tunables only make sense when the application has at least one custom mapper that responds to them.
- Returned values are **typed**; the application must `get_result<T>()` with the matching type.
- The mapper's response is computed once per query; cache on the application side if you query the same tunable many times.
- Tunable evaluation runs in mapper-context — same non-blocking rules apply (`mapper.md`).

## Performance implications
- A few cheap tunables at startup are free. Frequent runtime queries pile up in the mapper-callback path; cache.
- The standard use is **partition count at startup** — answer is queried once and used to size the partition for the entire run.
- Tunable values that depend on input data (file size, problem dimensions) are fine; they just take longer to resolve and so are queried earlier.

## Debug signals
- **`LoggingWrapper`** logs every `select_tunable_value` call and its returned value.
- **Mismatched `get_result<T>` type**: assertion or garbage value in debug build; UB in release.

## Failure modes
- Querying a tunable ID the mapper doesn't recognize → mapper falls into its default branch (usually returning 0 or an error), the application proceeds with bad data.
- Forgetting to call `get_result` (the future is fetched lazily) → never observes the value.

## Source pointers
- **Legion API**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Tutorial (PartitioningMapper)**: https://legion.stanford.edu/tutorial/custom_mappers.html (mirrored at `raw/tutorials/10_custom_mappers.md`)
- **Mapper reference**: `raw/website-pages/mapper.md`

## Related
- `wiki/concepts/mapper.md` — host of the `select_tunable_value` callback.
- `wiki/concepts/default-mapper.md` — provides a sensible default that often returns sane values for stock tunables.
- `wiki/concepts/future.md` — what `get_tunable_value` returns.
- `wiki/concepts/task.md` — typically queried from the top-level task.

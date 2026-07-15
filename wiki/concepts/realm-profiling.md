---
title: Realm Profiling
slug: realm-profiling
summary: Realm's built-in per-operation profiling API; attach a `ProfilingRequestSet` to any task spawn, copy, fill, or instance creation to receive timestamps, processor usage, memory usage, and copy paths back via a callback task.
tags: [profiling, tooling, for-perf-debug]
subsystem: realm
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/realm_13_profiling.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/realm/profiling.h
related:
  - wiki/concepts/legion-prof.md
  - wiki/concepts/timeline-view.md
  - wiki/concepts/event.md
  - wiki/concepts/region-instance.md
  - wiki/concepts/dma-system.md
---

## TL;DR
Realm has its own per-operation profiling API independent of `legion-prof.md`. You build a `ProfilingRequestSet`, add measurement types (`OperationTimeline`, `OperationProcessorUsage`, `InstanceTimeline`, `InstanceMemoryUsage`, `OperationCopyInfo`, `OperationMemoryUsage`), pass the set to any spawning API (`Processor::spawn`, `RegionInstance::create_instance`, `IndexSpace::copy`/`fill`), and Realm launches a **profiling task** with the results when the operation completes. The confusion: this is the substrate Legion Prof is built on — Legion Prof captures these measurements automatically via the `-lg:prof` flag, but you can use the raw API directly for **application-level profiling** that Legion Prof doesn't expose (e.g., dynamic load balancing based on real task timings).

## Mental model
Realm profiling is like `perf_event_open` for Legion ops — request specific measurements at op-issue time, receive them via callback when the op completes. Where Legion Prof produces a profile file for offline analysis, the Realm API delivers measurements **in-program** so the application can react. Standard use case: a dynamic load-balancing mapper that uses real measured task times to make placement decisions.

## Mechanism & API

**Build the request set**:
```cpp
ProfilingRequestSet prs;
prs.add_request(profile_proc, COMPUTE_PROF_TASK,
                &task_result, sizeof(ComputeProfResultWrapper))
   .add_measurement<ProfilingMeasurements::OperationTimeline>()
   .add_measurement<ProfilingMeasurements::OperationProcessorUsage>();
```

The request says: "when the operation completes, launch `COMPUTE_PROF_TASK` on `profile_proc` with `&task_result` as the payload, and include these measurements".

**Pass to the spawning API**:
```cpp
worker_procs[0].spawn(COMPUTE_TASK, &args, sizeof(args), prs).wait();
```

Most Realm operations accept a `ProfilingRequestSet` argument — task spawns, `create_instance`, `IndexSpace::copy`, `fill`, etc. The runtime guarantees **exactly one `ProfilingResponse` per request** when the operation finishes.

**Inside the profiling task**:
```cpp
void compute_prof_task(const void *args, size_t arglen, ...) {
  ProfilingResponse resp(args, arglen);

  const ComputeProfResultWrapper *result =
      static_cast<const ComputeProfResultWrapper *>(resp.user_data());

  ProfilingMeasurements::OperationTimeline timeline;
  if (resp.get_measurement(timeline)) {
    metrics->ready_time    = timeline.ready_time;
    metrics->start_time    = timeline.start_time;
    metrics->complete_time = timeline.complete_time;
  }
}
```

**Available measurements** (per `raw/tutorials/realm_13_profiling.md`):

| Measurement | Captures |
|---|---|
| `OperationTimeline` | timestamps for ready / start / complete |
| `OperationProcessorUsage` | which processor ran the op |
| `InstanceTimeline` | instance create / ready / destroy times |
| `InstanceMemoryUsage` | memory location + size of an instance |
| `OperationCopyInfo` | source/dest memories + bytes for a copy/fill |
| `OperationMemoryUsage` | memory consumption during a copy/fill |

**Best-practice routing**: use `UTIL_PROC` (`processor-kinds.md`) for the profiling-task target, not application processors — profile-task overhead must not pollute application timing.

**Synchronous waiting**: for code that needs to *block* on a profiling result (rare), pair the request with a `user-event.md`. The profiling task triggers the event; the caller waits on it.

## Invariants
- Exactly **one `ProfilingResponse`** per `ProfilingRequest`.
- The profiling task receives the response **after** the operation completes; you cannot get timing data before the op has finished.
- Profile-task overhead is **not measured** by the operation it profiles (no Heisenberg effect on the measured timestamps).
- Measurements not present in `resp.get_measurement(...)` return `false` — always check the return value.
- Profiling requests apply per-operation; one request set can attach measurements to many op kinds (the runtime ignores measurements that don't apply).

## Performance implications
- The cost of a single profiling request is small (a callback + a few timestamps); large numbers of profiled ops add up but rarely dominate.
- The **profiling task itself** is a task; route it to `UTIL_PROC` so it doesn't displace application work.
- Dynamic load-balancing using profiling results can deliver large wins — the tutorial demonstrates 38ms → 21ms (45% reduction) by replacing round-robin with profile-guided placement.
- `legion-prof.md` enables a comprehensive set of these measurements via `-lg:prof` and writes them to per-node log files; the raw API gives finer-grained control.

## Debug signals
- **Missing measurement in response** → the operation kind doesn't support it (e.g., asking for `OperationCopyInfo` on a task spawn). Check the operation type.
- **Profiling task target on `LOC_PROC`** → application contention; move it to `UTIL_PROC`.
- **Wildly varying per-op timings across runs** → run with profiling and inspect `OperationTimeline` to localize the variance (queueing vs. execution).

## Failure modes
- Forgetting `add_request`/`add_measurement` → no measurement collected; the response returns `false` for `get_measurement`.
- Storing the payload in a stack buffer that goes out of scope → use-after-free in the profiling task. Pass via heap or value-copy.

## Source pointers
- **Realm header**: https://github.com/StanfordLegion/legion/blob/master/runtime/realm/profiling.h
- **Tutorial**: `raw/tutorials/realm_13_profiling.md`

## Related
- `wiki/concepts/legion-prof.md` — built on this API; captures these measurements via `-lg:prof`.
- `wiki/concepts/timeline-view.md` — what Legion Prof renders from the measurements.
- `wiki/concepts/event.md` — what every spawn / copy / fill returns; profiling completes alongside it.
- `wiki/concepts/region-instance.md` — instance-creation profiling targets these.
- `wiki/concepts/dma-system.md` — `OperationCopyInfo` measurements come from this.

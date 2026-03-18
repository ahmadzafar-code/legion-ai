id: expensive_mapper_calls
title: Mapper calls take too long and block the scheduling loop
source:
  - "017 - Michael Bauer (live demo), Section 4: Performance Debugging Methodology"
  - "Transcript 003 (Scheduling and Mapper Calls), Transcript 002, Category 3"
confidence: high
user_type: all

symptoms:
  what_you_see: |
    In Legion Prof, utility processor timelines show long-duration mapper call tasks
    (especially select_task_to_map, map_task, select_task_options, or other mapper
    callbacks). Gaps appear on application processors between task executions because
    the next batch of tasks cannot be mapped until the current mapper call completes.
    The scheduling pipeline appears serialized rather than pipelined. GPU/CPU gaps
    correlate temporally with these long mapper calls. The mapper calls are individually
    visible in Legion Prof because "Legion actually profiles all your mapper calls."

  key_metrics: |
    - Individual mapper call durations in the millisecond+ range
    - select_task_to_map duration significantly longer than task execution time
    - Application processor idle gaps correlated with mapper call durations
    - Utility processor busy primarily with mapper calls rather than
      dependence analysis or Replay Physical Trace tasks
    - Low overall task throughput despite having ready tasks

  distinguishing_features: |
    Unlike runtime_limited_no_tracing (where utility is saturated with
    dependence analysis), here the utility processors are busy specifically with
    MAPPER calls. Legion Prof distinguishes these. Unlike high_delayed_duration,
    the bottleneck is in the Legion mapper layer, not in Realm. Unlike the
    scheduler-spin pattern, each mapper call IS doing work — the duration per call
    is high. Unlike a serialized-mapper-bottleneck, this occurs even in concurrent
    mapper mode because the individual call itself is slow. Check the mapper call
    breakdown on utility processors to distinguish.

root_cause: |
  Mapper calls (map_task, select_task_to_map, select_task_options, etc.) are
  callbacks into user-provided or default mapper code that run on utility processors.
  They can be slow due to: (1) complex mapper logic or expensive operations
  (optimization solvers, network I/O, complex heuristics), (2) OS scheduling
  interference descheduling the utility thread mid-call, (3) mapper bugs causing
  excessive computation, (4) lock contention in the mapper, or (5) mappers calling
  back into the runtime during these calls, creating compound latency. The runtime
  releases internal locks before calling the mapper specifically because of this
  risk, but the mapper call itself still occupies the scheduling thread.

gotchas:
  - "The runtime elaborately swaps queue copies and manages queue guards specifically because select_task_to_map is expensive — this overhead is the system TRYING to work around your slow mapper."
  - "Mapper calls can call back into the runtime, meaning a single mapper call might trigger cascading runtime work that extends its duration further."
  - "In serialized mapper mode, a long mapper call blocks ALL other mapper calls for that mapper, compounding the problem."
  - "OS scheduling interference can inflate mapper call times even when the mapper code itself is efficient. A complementary sampling profiler is needed to distinguish mapper-code slowness from OS descheduling."
  - "Legion Prof only measures start and end times — if the utility thread is descheduled mid-mapper-call, the measured duration includes the descheduled time."
  - "Automatic tracing does not help with mapper overhead directly — tracing skips re-analysis but mapper calls may still occur on the trace-replay path depending on the mapper implementation."

fix:
  primary: |
    Optimize the mapper's select_task_to_map and other mapper callbacks to be as
    fast as possible:
    1. Pre-compute mapping decisions or use cached results. Avoid expensive searches,
       solver calls, or I/O inside mapper callbacks.
    2. Use a complementary sampling profiler (Nsight/VTune) simultaneously to
       determine if the overhead is in the mapper code itself or due to OS scheduling.
    3. If mapper code is slow: optimize the mapper logic, cache decisions, or switch
       to a more efficient mapper.
    4. If OS descheduling: fix CPU affinity/binding (see slurm_misconfiguration).

  alternatives: |
    - Use DefaultMapper memoization (-dm:memoize) to cache mapper decisions.
    - Enable tracing to reduce the number of mapper calls in steady state.
    - Simplify region requirements to reduce mapper decision complexity.
    - Switch to concurrent mapper mode (if your mapper is thread-safe) so other
      mapper calls can proceed while one is slow.
    - Use DefaultMapper or another well-optimized mapper implementation as a base.

  what_not_to_do: |
    - Do NOT ignore mapper call durations assuming they are always fast.
    - Do NOT assume all utility processor saturation is dependence analysis —
      always check whether mapper calls are the dominant contributor.
    - Do NOT add sleep or yield calls inside mapper functions — the runtime has
      its own scheduling and yield mechanisms.
    - Do NOT hold mapper-internal locks for extended periods as this defeats the
      runtime's lock-release-around-mapper-call design.

verification: |
  After fixing, individual mapper call durations should drop and be short relative
  to task execution times. Utility processor utilization attributable to mapper calls
  should decrease. Application processor idle gaps between tasks should shrink.
  Downstream GPU/CPU gaps correlated with mapper calls should shrink. Overall task
  throughput should increase.

real_cases:
  - case: "Talk 017 live demo"
    app: "[unnamed application]"
    scale: "[not specified]"
    result: "Identified as a pattern during live debugging demonstration"
    key_detail: "Bauer noted 'this application is spending a very long time with some of these mapper calls'"
  - case: "[No specific case cited]"
    app: "[not specified]"
    scale: "[not specified]"
    result: "[not specified]"
    key_detail: "select_task_to_map is explicitly called out as the single most expensive mapper call"

related_patterns:
  - "runtime_limited_no_tracing"
  - "thread_descheduling_invisible"
  - "slurm_misconfiguration"
  - "scheduler_spin_no_deferral"
  - "serialized_mapper_bottleneck"

  ```yaml

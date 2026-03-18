id: insufficient_parallelism
title: Insufficient task parallelism starves processors between task waves
source: low_processor_utilization_diagnosis.md, Category 3; GPU differential diagnosis guide, Cause 7; Legion profiling documentation; Jax-on-Realm presentation (December 2024 Legion retreat)
confidence: medium
user_type: all

symptoms:
  what_you_see: |
    GPU is idle, utility processors are idle or nearly idle, and no channel
    activity is present during [T1, T2]. Gaps appear simultaneously across ALL
    application processors of a kind (e.g., all GPUs idle at the same time).
    Utility processors are ALSO idle during these same gaps — they have nothing
    to analyze. The gap occurs between "waves" of tasks — for example, between
    index launches where one parallel wave must complete before the next begins.
    The pattern is irregular, unlike regular gaps from explicit synchronization.
    Looking at the task dependency graph (via Legion Spy overlay, `a` key in the
    viewer), the critical path runs through a serial chain of tasks with no
    parallel alternatives. The ready queue is empty during the gap.

  key_metrics: |
    - Q3.2: Many time slices with <20% application processor utilization
    - Q3.3: Utility processors also idle during application gaps (rows returned = periods where everything is idle)
    - Q3.1: Concurrent task count consistently below number of available processors
    - Critical path overlay (press 'a' in viewer, requires -lg:spy) passes through nearly every time step
    - Channel activity during gap: zero
    - CPU activity during gap: idle or actively submitting next wave (not blocked)
    - Gap is irregular, occurring between waves of tasks
    - Scheduling window (`-lg:window`, default 1024) may be too small

  distinguishing_features: |
    Unlike Category 1 (runtime overhead), utility processors are IDLE during
    application processor gaps. In Category 1, utility processors are saturated
    during gaps. This is the key discriminator: both categories show app gaps,
    but utility behavior differs completely. Unlike Cause 4 (blocking Python),
    the Python processor is either idle (all tasks for this wave already
    submitted) or actively submitting next-wave tasks — NOT in a waiting/blocked
    state. Unlike Cause 3 (missing tracing), utility processors are idle during
    the gap (no mapper calls — there's nothing to map because no new tasks have
    been created yet). Unlike Cause 6 (explicit sync), gaps are irregular and
    between waves, not uniform between every single task. The critical path
    visualization (`a` key) confirms a serial dependency chain.

root_cause: |
  The application issues fewer independent tasks than available processors.
  The Jax-on-Realm presentation states: "It is a pipeline! No task parallelism
  to hide task startup latency." When the runtime's scheduling window
  (`-lg:window`, default 1024) is insufficient, or when task dependencies create
  long serial chains, the GPU drains its ready queue faster than new tasks arrive.
  The SOOP pipeline stages (dependence analysis, mapping, scheduling, execution)
  cannot overlap if there aren't enough independent tasks in flight. Common
  causes: individual task launches instead of index space launches, or Legate
  minimum chunk sizes preventing parallelization of small arrays
  (LEGATE_MIN_GPU_CHUNK = 1,048,576 elements by default).

gotchas:
  - "Q3.3 is the critical check: if utility processors are busy during app gaps, redirect to Category 1 (runtime overhead), not parallelism."
  - "The critical path visualization (a key in the viewer) is essential for confirming this diagnosis — do not skip it."
  - "The -lg:window default of 1024 may be too small for applications that need deep lookahead to find parallelism."
  - "Index launches that require all tasks in one wave to complete before the next wave starts create artificial serialization."
  - "For Legate, the default LEGATE_MIN_GPU_CHUNK of 1,048,576 elements may prevent parallelization even when the array is large enough for multiple GPUs."
  - "May co-occur with Cause 3 (missing tracing) or Cause 4 (blocking Python) — if the runtime is also slow at analysis or Python is also blocking, the starvation is worse."
  - "This pattern can also result from mapper bugs (Category 3c) — sufficient independent tasks exist but the mapper serializes them on the same processor."

fix:
  primary: |
    Increase task parallelism by restructuring the algorithm to overlap
    independent computations. Replace individual task launches with index space
    launches over partitions. Index launches have O(1) runtime overhead regardless
    of task count, versus O(N) for N individual launches. Use at least one task
    per GPU. Increase the scheduling window with `-lg:window` if the runtime's
    lookahead is the bottleneck. Consider double-buffering or multi-phase
    pipelining where independent stages overlap. Increase `-lg:sched` to schedule
    more tasks per scheduler invocation. In Legate, index launches happen
    automatically for parallel arrays.

  alternatives: |
    - Increase partition granularity so more independent tasks exist.
    - For Legate, reduce LEGATE_MIN_GPU_CHUNK (default 1,048,576) to allow
      finer-grained parallelism on smaller arrays.
    - Verify the mapper is not serializing independent tasks on the same processor.
    - Coarsen tasks (fewer, larger tasks) so each GPU task runs longer relative
      to the inter-wave gap.
    - Use futures and must-epoch launches to express fine-grained dependencies
      that allow overlap between waves.

  what_not_to_do: |
    Do NOT attempt runtime overhead fixes (-dm:memoize, -ll:util) for a
    parallelism problem — the runtime has nothing to do. Do NOT add more
    processors when there aren't enough tasks to fill the existing ones. Do NOT
    assume the problem is tracing (Cause 3) just because there are gaps with low
    activity — Cause 7 has low activity because there is nothing TO do, whereas
    Cause 3 has high utility activity doing redundant work. Check utility
    processors before concluding starvation.

verification: |
  After increasing task count / parallelism:
  1. Q3.1 should show concurrent task counts matching or exceeding the number
     of available application processors.
  2. Q3.2 time slices with <20% utilization should decrease.
  3. Q3.3 should return fewer rows (less time where everything is idle).
  4. Gaps between task waves should shrink or disappear.
  5. The critical path should show more parallel alternatives.
  6. The ready queue should have tasks available during previously idle periods.
  7. Increasing `-lg:window` should show the runtime mapping tasks further
     ahead of execution.

real_cases:
  - case: "[No specific issue number cited]"
    app: "Jax-on-Realm"
    scale: "December 2024 Legion retreat analysis"
    result: "[Not quantified — identified as pipeline starvation pattern]"
    key_detail: "Jax-on-Realm presentation explicitly identified 'no task parallelism to hide task startup latency' as the problem"
  - case: "[INCOMPLETE — needs review]"
    app: "[INCOMPLETE — needs review]"
    scale: "[INCOMPLETE — needs review]"
    result: "[INCOMPLETE — needs review]"
    key_detail: "Document describes pattern but provides no specific case study for this sub-cause"

related_patterns:
  - "insufficient_parallelism_dependency_serialization"
  - "insufficient_parallelism_mapper_serialization"
  - "runtime_overhead_no_tracing"
  - "blocking_python_operations"
  - "missing_tracing"
```

# Unique Records — No Merge Needed
# These records have no overlaps and pass through as-is.


## Source: Diagnosed Cases

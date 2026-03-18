id: tasks_below_metg
title: Tasks below METG threshold (~100 µs CPU, ~39 µs GPU optimized) cannot achieve 50% efficiency
source: low_processor_utilization_diagnosis.md, Category 1 + Co-occurrence section; Task Bench (SC 2020); Task granularity and launch overhead section; Anti-pattern reference table
confidence: high
user_type: all

symptoms:
  what_you_see: |
    Narrow task bars in Legion Prof with large gaps between them. Utility
    processors show continuous activity but execution processors are
    frequently idle. Zooming in reveals many tiny tasks separated by idle
    periods — individual task execution times are often visually
    indistinguishable from gaps at normal zoom. High ratio of runtime
    meta-task time to application task time.

  key_metrics: |
    - AVG(stop - start) for application processor items <100μs (with tracing)
      or <1ms (without tracing) — this is the FIRST screening check in the
      prioritization algorithm
    - METG(50%) thresholds: ~100 µs (CPU), ~39 µs (GPU optimized),
      ~173 µs (GPU standard)
    - ~20 µs aggregate pointer-query overhead per task even at best
    - Runtime overhead exceeds useful work per task

  distinguishing_features: |
    This is a screening condition that supersedes the four-category
    decision tree. If average task duration is below METG, runtime
    overhead is mathematically guaranteed to be the bottleneck regardless
    of other factors. Unlike missing tracing or low -ll:util (where the
    runtime is the bottleneck for adequate-sized tasks), here the tasks
    themselves are fundamentally too small. Even with perfect runtime
    configuration, tasks below METG cannot achieve 50% efficiency. The
    narrow task bars with proportionally large gaps are the visual
    signature.

root_cause: |
  The Minimum Effective Task Granularity (METG), defined by Task Bench
  (SC 2020), is the shortest task duration at which a system maintains
  ≥50% efficiency. Legion's runtime overhead floor includes dependence
  analysis, mapping, physical instance management, and Realm event
  processing, adding ~20 µs of aggregate pointer-query overhead per task
  even when Realm achieves sub-10-µs event processing. For untraced
  Legion on CPU, METG ≈ 1ms; for traced Legion on CPU, METG ≈ 100μs;
  for GPU optimized mode, METG ≈ 39μs; for GPU standard mode,
  METG ≈ 173μs. When tasks are below the applicable threshold, per-task
  runtime overhead exceeds useful work, making efficiency mathematically
  impossible.

gotchas:
  - "METG is a hard floor — no amount of runtime tuning can fix tasks below this threshold."
  - "The METG threshold depends on whether tracing is active: ~100μs traced vs ~1ms untraced on CPU. Check tracing status first."
  - "The METG for GPU standard mode (~173 µs) is much higher than optimized mode (~39 µs) — ensure GPU optimizations are enabled."
  - "In Legate, LEGATE_MIN_GPU_CHUNK (default 1,048,576 elements) prevents parallelization of small arrays, but the resulting tasks may still be below METG if per-element work is trivial."
  - "Do NOT attempt to fix other categories (communication, parallelism, memory) when tasks are below METG — the primary recommendation is always to increase task granularity or enable tracing."
  - "For comparison: MPI ~5 µs METG, Charm++ ~10 µs — Legion's overhead is inherently higher due to richer programming model."
  - "Actor-model optimizations improved METG by 3.3–7.1× for Legion and 1.77–5.3× for Realm."

fix:
  primary: |
    Coarsen tasks to keep each above ~200 µs of useful work (2× METG for
    safety margin). Methods: increase problem size per task (coarser
    partitions), fuse multiple operations into single tasks, increase
    per-element work (algorithmic change). Use IndexTaskLauncher for all
    collections of similar tasks. If tracing is not active, enable it
    with -dm:memoize to lower the CPU threshold from ~1ms to ~100μs.

  alternatives: |
    - Enable tracing (-dm:memoize) to reduce METG from ~1ms to ~100μs on CPU
    - Increase LEGATE_MIN_GPU_CHUNK to force coarser decomposition
    - Merge multiple fine-grained operations into fewer coarser tasks
    - Use tiling or blocking strategies to increase per-task work
    - Restructure the algorithm to perform more work per task

  what_not_to_do: |
    Do NOT spend time diagnosing communication, parallelism, or memory
    pressure when tasks are below METG — these are secondary effects
    at best. Do NOT add more processors — they will only increase the
    number of idle resources waiting for the runtime pipeline. Do NOT
    attempt finer granularity than METG — it yields diminishing and
    eventually negative returns regardless of other optimizations. Do NOT
    expect runtime tuning alone (utility processors, etc.) to fix
    fundamentally too-small tasks.

verification: |
  After increasing task granularity:
  1. AVG(stop - start) for application tasks should exceed the applicable
     METG threshold (100μs traced CPU, 1ms untraced CPU, 39μs GPU
     optimized, 173μs GPU standard) — ideally 2× METG (~200 µs+).
  2. Task bars in Legion Prof should be wider relative to gaps.
  3. Application processor utilization should increase above 50%.
  4. Runtime-to-application time ratio should decrease.
  5. The ratio of useful work time to total time should improve measurably.

real_cases:
  - case: "Task Bench (SC 2020)"
    app: "Task Bench synthetic benchmarks"
    scale: "Various"
    result: "Defined METG metric; Legion untraced ≈ 1ms CPU, traced ≈ 100μs CPU"
    key_detail: "METG is defined as the shortest task duration achieving ≥50% efficiency"
  - case: "Task Bench benchmarking paper"
    app: "Task Bench"
    scale: "[single-node benchmarks]"
    result: "METG(50%): Legion ~100 µs CPU, ~39 µs GPU optimized, ~173 µs GPU standard"
    key_detail: "Establishes hard floor for task granularity across runtimes."

related_patterns:
  - "runtime_overhead_no_tracing"
  - "runtime_overhead_with_tracing"
  - "individual_task_launches"
  - "generic_accessors"
  - "unmarked_leaf_inner"

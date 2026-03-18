id: legate_partition_cache_bug
title: Broken partition cache caused 15-minute stalls in Legate
source: GitHub nv-legate/cunumeric#29; Case 6
confidence: high
user_type: legate

symptoms:
  what_you_see: |
    A simple Laplace equation solver takes >15 minutes under legate.numpy
    vs. seconds under vanilla NumPy on a 401×401 grid with 150K
    iterations. Profiler traces show excessive time in partitioning and
    runtime analysis rather than computation. The application processor is
    continuously busy but not doing useful computational work.

  key_metrics: |
    Wall-clock time: >15 minutes (legate.numpy) vs. seconds (NumPy) on
    401×401 grid. Application processor utilization = 100% but on
    runtime overhead, not computation. Partition creation count: expected
    cached after first call, actual re-created every operation.
    Time-per-iteration: constant overhead regardless of computation size.

  distinguishing_features: |
    Unlike runtime dependence analysis overhead (Case 4), the problem is
    in Legate's partition caching layer, not in Legion's core dependence
    analysis. The application processor is busy (not idle), but with
    Legate-level overhead. Distinguished from element-wise loop anti-
    pattern (Case 18b) by the fact that bulk operations are being used
    correctly — the overhead is internal to Legate.

root_cause: |
  A caching data structure for array-view creation was "not firing" — a
  code bug caused unnecessary partitioning calls to be dumped onto the
  Legion runtime for every operation. This was not inherent overhead but
  a broken cache lookup. Additionally, checking convergence every loop
  iteration prevented the Legion runtime from "running ahead" to schedule
  GPU work, killing pipelining.

gotchas:
  - "The cache bug and the convergence-check frequency are two separate issues — fixing one without the other still leaves poor performance."
  - "On small grids (401×401), Legate overhead is expected to dominate — use properly-sized problems (5001×5001+) to evaluate real performance."
  - "Convergence checks every iteration kill pipelining. Batch to every 100 iterations for GPU performance."

fix:
  primary: |
    Apply commit `e24dbdd` to fix the caching mechanism so partitions are
    properly reused. Reduce convergence-check frequency to every 100
    iterations (critical for GPU pipelining).

  alternatives: |
    If the cache fix is not available in your version, manually reuse
    partition objects. For the convergence-check issue, any reduction in
    check frequency helps — even every 10 iterations is better than every
    iteration.

  what_not_to_do: |
    Do NOT benchmark Legate on tiny problems (401×401) and conclude it
    is fundamentally slow. The overhead is amortized on larger problems.
    Do NOT check convergence every iteration on GPUs — this forces
    synchronization and kills the deferred execution pipeline.

verification: |
  After the fix, 150K iterations on 401×401 grid: ~742 seconds (3.3×
  slower than NumPy — expected for this small problem). On properly-sized
  5001×5001 grid with batched convergence checks: single-CPU 14.76
  iter/sec, single-GPU 233.61 iter/sec — a ~16× GPU speedup
  demonstrating proper pipelining.

real_cases:
  - case: "GitHub cunumeric#29"
    app: "Laplace equation solver (Legate)"
    scale: "Single node, single CPU and single GPU"
    result: "From >15 min stall to working; 16× GPU speedup on proper problem size"
    key_detail: "Partition cache 'not firing' was a code bug, not inherent overhead"

related_patterns:
  - cupynumeric_pipeline_stalls
  - legate_gc_oom

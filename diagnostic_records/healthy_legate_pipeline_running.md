id: healthy_legate_pipeline_running
title: Healthy Legate/cuPyNumeric profile — deferred pipeline running ahead
source: gpu-differentialdiagnosis.md (Cause 4 — inverse); cuPyNumeric Best Practices documentation
confidence: medium
user_type: legate

symptoms:
  what_you_see: |
    GPU rows show dense continuous task execution. Python processor row shows
    continuous task submission activity (NOT waiting/blocked). Utility rows
    show the runtime is well ahead of execution (brief trace replay activity,
    mostly idle). No sawtooth/comb pattern of GPU activity. The visual
    impression is "everything running smoothly" — no periodic drain-and-refill
    cycles.

  key_metrics: |
    - GPU utilization >80%
    - Python processor NOT in waiting state (continuously submitting)
    - Deferred duration avg >10ms (pipeline running well ahead)
    - Task IDs being mapped far ahead of task IDs being executed
    - No periodic GPU utilization drops to 0%
    - No sawtooth/comb pattern

  distinguishing_features: |
    The ABSENCE of the sawtooth pattern is the key health indicator for
    Legate profiles. Healthy: continuous GPU activity. Unhealthy: periodic
    GPU activity bursts separated by gaps (pipeline draining because Python
    blocked on a value materialization). Check that the Python processor is
    continuously active (submitting work) rather than alternating between
    active and waiting.

root_cause: |
  This is not a problem. The Legate deferred execution pipeline is working
  correctly: Python submits tasks asynchronously, the runtime analyzes them
  ahead of execution, and GPUs are continuously fed with work. No blocking
  operations are forcing pipeline synchronization.

gotchas:
  - "Even in a healthy Legate profile, convergence checks every N iterations will cause BRIEF pipeline pauses. If N is large enough (e.g., every 100 iterations), these pauses are negligible and the profile still looks healthy."
  - "cuPyNumeric operations like .shape, .ndim, .dtype, and .size are NOT blocking and do NOT affect pipeline health. Only computed-value materializations block."
  - "Set CUPYNUMERIC_DOCTOR=1 proactively to verify no anti-patterns are present, even if the profile looks healthy."

fix:
  primary: |
    No fix needed. If the user wants to verify health, run with
    CUPYNUMERIC_DOCTOR=1 to confirm no anti-patterns are present.

  alternatives: |
    N/A — healthy behavior.

  what_not_to_do: |
    Do NOT diagnose CPU/Python processor being continuously busy as a
    problem — it means the pipeline is running. A busy Python processor
    is healthy. A WAITING Python processor is unhealthy.

verification: |
  Continuous GPU utilization >80% across steady-state execution with no
  periodic drops.

real_cases:
  - case: "cuPyNumeric Best Practices documentation"
    app: "Properly optimized cuPyNumeric applications"
    scale: "Any"
    result: "16× GPU speedup on 5001×5001 grid (from Case 6 after fix)"
    key_detail: "Healthy pipeline produces continuous GPU utilization without sawtooth pattern"

related_patterns:
  - "cupynumeric_blocking_materialization_sync"
  - "blocking_python_operations"
  - "legate_partition_cache_bug"

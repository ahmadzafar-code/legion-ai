id: healthy_cpu_traced_profile
title: Healthy CPU-only profile with active tracing — processors well-utilized
source: legionconcepts.md (scaling behavior table); low_processor_utilization_diagnosis.md
confidence: medium
user_type: legion_cpp

symptoms:
  what_you_see: |
    CPU processor rows show dense task execution with small gaps. Utility rows
    show brief Replay Physical Trace bursts but are mostly quiet in steady
    state. No GPU rows present (or GPU rows empty — correct if no GPU tasks
    exist). Channel rows may show inter-node copies if multi-node, but copy
    activity should not dominate. OMP processor rows, if present, show parallel
    task execution.

  key_metrics: |
    - CPU utilization >70% in steady state (CPU tasks have more scheduling
      overhead than GPU tasks, so 70% is healthy vs 80% for GPU)
    - Utility utilization <30% in steady state
    - trace_replay_count > 0
    - Deferred duration avg >5ms
    - No GPU rows OR GPU utilization = 0% (correct if CPU-only workload)

  distinguishing_features: |
    CPU-only profiles have slightly lower utilization ceilings than GPU
    profiles because CPU tasks are typically finer-grained and scheduling
    overhead per task is higher relative to execution time. 70% CPU
    utilization is healthy where 70% GPU utilization might indicate room
    for improvement.

root_cause: |
  This is not a problem. Healthy CPU-only execution with tracing active.

gotchas:
  - "Do NOT apply GPU utilization thresholds (>80%) to CPU profiles. CPU tasks have higher relative scheduling overhead. 70%+ CPU utilization is healthy."
  - "Empty GPU rows on a CPU-only profile are correct behavior, not an error."
  - "OMP processors may show lower individual utilization than LOC processors because OpenMP thread management adds overhead — this is normal."

fix:
  primary: |
    No fix needed. If further optimization is desired, look at: task
    granularity (are tasks above METG?), critical path (which tasks are
    on the critical path?), and inside-task efficiency (VTune/perf for
    cache misses, branch mispredictions, etc.).

  alternatives: |
    N/A — profile is healthy.

  what_not_to_do: |
    Do NOT flag GPU rows being empty as a problem.
    Do NOT apply GPU utilization thresholds to CPU workloads.

verification: |
  Consistent CPU utilization >70% across steady-state iterations.

real_cases: []

related_patterns:
  - "healthy_gpu_traced_profile"

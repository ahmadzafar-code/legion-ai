id: healthy_gpu_traced_profile
title: Healthy GPU profile with active tracing — high utilization, low runtime overhead
source: legionconcepts.md (scaling behavior table); PROFILER_EXTRACTION.md (Bauer debugging methodology); low_processor_utilization_diagnosis.md (Category 1 thresholds)
confidence: high
user_type: all

symptoms:
  what_you_see: |
    GPU processor rows show dense, nearly continuous task execution with small
    gaps between tasks. Utility processor rows show periodic brief bursts of
    "Replay Physical Trace" tasks but are mostly idle in steady state. After
    the first iteration (which is visibly busier on utility rows due to initial
    trace recording), subsequent iterations show dramatically less utility
    activity. CPU rows may be mostly idle if all compute is GPU-mapped — THIS
    IS CORRECT BEHAVIOR, not a problem. Channel rows show moderate copy
    activity that does not temporally correlate with GPU gaps (copies overlap
    with GPU execution = healthy overlap). Memory rows show stable utilization
    without monotonic growth.

  key_metrics: |
    - GPU utilization >80% in steady state
    - Utility utilization <20% in steady state (after first iteration)
    - trace_replay_count > 0 (Replay Physical Trace tasks present)
    - Deferred duration avg >10ms (runtime running well ahead of execution)
    - Run-ahead distance: mapped task IDs 10s-100s ahead of executing task IDs
    - CPU utilization low IF all compute is GPU-mapped (correct behavior)
    - No red deferred annotations on copies
    - No long-latency message warnings from profiler

  distinguishing_features: |
    The key signatures of health are: (1) utility rows go quiet after iteration 1
    (tracing working), (2) deferred durations are large (pipeline not stalled),
    (3) GPU rows are dense (processors fed), (4) no synchronized gaps across
    all GPUs (no thread oversubscription). A healthy profile may still have
    minor gaps — task scheduling inherently has some overhead. Gaps <0.5% of
    profile duration are noise, not issues.

root_cause: |
  This is not a problem. The profile is showing correct, well-optimized
  execution where: tracing eliminates repeated dependence analysis, the SOOP
  pipeline runs far ahead of execution, GPU processors are well-fed with
  work, and data movement overlaps with computation.

gotchas:
  - "CPU idle on GPU workloads is CORRECT BEHAVIOR. Do NOT flag it as a problem. If all application tasks are GPU-mapped, the CPU has nothing to do."
  - "First-iteration utility activity is EXPECTED. Tracing must record on the first iteration before it can replay. Do NOT diagnose first-iteration overhead as a tracing problem."
  - "Periodic brief utility spikes during trace replay are NORMAL and HEALTHY. They indicate the runtime is replaying memoized analysis, not recomputing it."
  - "Small gaps (<0.5% of profile duration) between GPU tasks are scheduling noise, not performance issues. Focus on gaps >2% of total duration."
  - "Memory that appears 'full' may not indicate pressure — Legion's GC is lazy and keeps invalid instances until memory is needed. Check for GC meta-tasks and allocation failures, not just occupancy."

fix:
  primary: |
    No fix needed. This profile is healthy. If the user is still experiencing
    slower-than-expected wall time, the bottleneck is likely INSIDE task
    execution (algorithmic efficiency, memory access patterns, kernel
    optimization) rather than in Legion's runtime overhead. Recommend Nsight
    Systems or VTune for inside-task profiling.

  alternatives: |
    If GPU utilization is >80% but the user wants more:
    - Check if critical path tasks can be shortened (algorithmic improvement)
    - Check for copy operations on the critical path that could be eliminated
      with better data placement
    - Look for load imbalance across GPUs (one GPU consistently finishing
      earlier than others)

  what_not_to_do: |
    Do NOT diagnose CPU idle time as a problem on GPU workloads.
    Do NOT diagnose first-iteration tracing overhead as a problem.
    Do NOT diagnose small scheduling gaps as performance issues.
    Do NOT suggest runtime flags (-dm:memoize, -ll:util, etc.) when tracing
    is already active and utilization is already high.
    Do NOT suggest -cuda:legacysync when there are no synchronized GPU gaps.

verification: |
  A healthy profile should maintain these properties across iterations:
  - Steady-state GPU utilization consistently >80%
  - Utility processors quiet after iteration 1
  - Deferred durations stable at tens of ms or more
  - No profiler warnings (long-latency messages, debug mode banner)

real_cases:
  - case: "SC 2018 tracing paper — post-optimization benchmarks"
    app: "Multiple Legion benchmarks with tracing enabled"
    scale: "Various"
    result: "4.9-7.0× improvement over untraced = the healthy baseline"
    key_detail: "After enabling tracing, the profiles showed the pattern described here"

related_patterns:
  - "runtime_limited_no_tracing"
  - "low_deferred_duration"

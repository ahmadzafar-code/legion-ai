id: healthy_utility_trace_replay_spikes
title: Periodic utility processor spikes during trace replay are normal
source: Legion Runtime anti-patterns reference (tracing section); PROFILER_EXTRACTION.md
confidence: high
user_type: all

symptoms:
  what_you_see: |
    In steady-state execution (iteration 2+), utility processor rows show
    periodic brief spikes of activity. These spikes consist of "Replay
    Physical Trace" meta-tasks. Between spikes, utility processors are quiet.
    The spikes are regular and predictable, occurring once per iteration or
    once per traced phase. Application processor rows show continuous execution
    with no corresponding gaps during the utility spikes.

  key_metrics: |
    - Utility spikes contain "Replay Physical Trace" tasks (NOT "Logical
      Dependence Analysis" or "map_task")
    - Spike duration is short relative to iteration time (typically <5%)
    - Application processors continue executing during utility spikes
    - No deferred duration degradation during spikes

  distinguishing_features: |
    Unlike runtime overhead from missing tracing (where utility is CONTINUOUSLY
    saturated with analysis work), trace replay spikes are BRIEF and PERIODIC
    with quiet periods between them. Unlike thread oversubscription (where
    utility spikes CORRELATE with GPU gaps), healthy trace replay spikes do
    NOT cause application processor gaps. The content of the spikes matters:
    Replay Physical Trace = healthy. Logical Dependence Analysis = unhealthy.

root_cause: |
  This is not a problem. Trace replay requires some utility processor time
  to replay the memoized task graph for each iteration. This is orders of
  magnitude cheaper than fresh analysis (~100μs vs ~1ms per task) and is
  the expected behavior of a well-configured traced application.

gotchas:
  - "Check WHAT is in the utility spike: Replay Physical Trace = healthy, Logical Dependence Analysis = unhealthy. They can look similar at zoom-out."
  - "If utility spikes are growing over time (each iteration's spike is longer than the last), this may indicate trace template growth — a different issue."
  - "If utility spikes DO cause corresponding application processor gaps, this may indicate thread oversubscription (Cause 2) where trace replay is contending with application threads for CPU cores."

fix:
  primary: |
    No fix needed. This is expected behavior for traced execution.

  alternatives: |
    N/A — healthy behavior.

  what_not_to_do: |
    Do NOT disable tracing to eliminate the utility spikes.
    Do NOT increase -ll:util specifically to handle trace replay spikes
    (unless spikes are actually causing application processor stalls).
    Do NOT confuse trace replay spikes with runtime overhead.

verification: |
  Healthy trace replay spikes should: (1) contain Replay Physical Trace
  tasks, (2) be brief relative to iteration time, (3) NOT cause gaps on
  application processors, (4) remain constant size across iterations.

real_cases: []

related_patterns:
  - "thread_oversubscription_stream_interference"
  - "runtime_overhead_with_tracing"

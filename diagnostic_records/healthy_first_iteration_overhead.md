id: healthy_first_iteration_overhead
title: First iteration is slower than subsequent iterations — expected tracing behavior
source: PROFILER_EXTRACTION.md (Bauer methodology); gpu-differentialdiagnosis.md (Cause 3 gotchas); Legion Runtime anti-patterns reference (tracing section)
confidence: high
user_type: all

symptoms:
  what_you_see: |
    The first iteration of the application shows: heavy utility processor
    activity (dependence analysis, mapping, scheduling), longer task execution
    times on application processors, more copy operations, and potentially
    visible gaps between tasks. Starting from iteration 2, utility processors
    become mostly quiet, task gaps shrink dramatically, and overall throughput
    improves. The profile has a visually obvious "warmup" phase followed by
    steady-state execution.

  key_metrics: |
    - Iteration 1: utility utilization may be 80-100%, deferred durations small
    - Iteration 2+: utility utilization drops to <20%, deferred durations increase to tens of ms
    - Replay Physical Trace tasks appear on utility processors starting iteration 2
    - Total time for iteration 1 may be 2-10× longer than subsequent iterations

  distinguishing_features: |
    The KEY distinguishing feature is that subsequent iterations are dramatically
    better. If ALL iterations show the same overhead as iteration 1, tracing
    is NOT working (see pattern: runtime_limited_no_tracing). If only iteration
    1 is slow and all subsequent iterations are fast, tracing IS working and
    the first iteration overhead is expected.

root_cause: |
  This is not a problem. Dynamic tracing must record the task graph on the
  first iteration before it can replay it. The first iteration performs full
  dependence analysis, mapping, and scheduling — exactly like untraced
  execution. Starting from iteration 2, the recorded trace is replayed,
  skipping most of this work. The first-iteration overhead is the cost of
  building the trace template.

gotchas:
  - "Do NOT diagnose first-iteration overhead as 'missing tracing' — tracing IS working, it just needs one iteration to record."
  - "Multiple distinct phases in the application may each have their own 'first iteration' — the trace is per-phase, not global."
  - "If using automatic tracing (Apophenia, v25.03.0+), the system may take 1-3 iterations to detect the repeated pattern before trace recording begins."
  - "When reporting profile statistics, EXCLUDE the first iteration from steady-state analysis. Report both 'including warmup' and 'steady-state only' numbers."

fix:
  primary: |
    No fix needed. This is expected behavior. When reporting performance,
    exclude the first iteration from steady-state metrics.

  alternatives: |
    If the first iteration is unacceptably slow for the use case (e.g.,
    inference serving where latency matters on every request), consider:
    - Pre-warming with a dummy iteration before the real workload
    - Saving and restoring trace templates (not currently supported as
      a user-facing feature but on the roadmap)

  what_not_to_do: |
    Do NOT disable tracing to make the first iteration "consistent" with
    subsequent iterations — you'd make ALL iterations as slow as the first.
    Do NOT report first-iteration performance as representative of
    steady-state behavior.

verification: |
  Iteration 2 and beyond should show: dramatically lower utility processor
  activity, presence of Replay Physical Trace tasks, and consistent per-
  iteration performance within ±5%.

real_cases:
  - case: "Every tracing benchmark in SC 2018 paper"
    app: "All"
    scale: "All"
    result: "First iteration 2-10× slower is expected and documented"
    key_detail: "The 4.9-7.0× improvement numbers compare steady-state traced vs all-iterations untraced"

related_patterns:
  - "runtime_limited_no_tracing"
  - "runtime_overhead_with_tracing"

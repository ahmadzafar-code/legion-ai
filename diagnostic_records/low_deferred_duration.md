id: low_deferred_duration
title: Low Deferred Duration Indicates Runtime Falling Behind Execution
source: "017 - Michael Bauer (live demo), Section 4: Deferred vs. Delayed Metrics"
confidence: high
user_type: all

symptoms:
  what_you_see: |
    Copies (and potentially tasks) annotated with red coloring in Legion
    Prof, indicating low deferred duration. Gaps appear in GPU/CPU
    timelines as processors wait for the runtime to produce work. The
    visual pattern is periodic or growing "bubbles" in execution as the
    pipeline drains.

  key_metrics: |
    - Deferred duration < 1 ms (red annotation threshold).
    - Specifically: "less than a millisecond down to a few hundreds of
      microseconds or less" triggers the red warning.
    - Healthy deferred values are "tens to hundreds of milliseconds" —
      indicating the runtime is running well ahead.

  distinguishing_features: |
    Unlike high delayed duration (which indicates Realm scheduling
    contention), low deferred duration means the problem is UPSTREAM:
    Legion's dependence analysis hasn't produced work far enough ahead.
    Deferred measures the gap between when Legion launches the copy
    and when its precondition fires. Delayed measures the gap between
    precondition firing and Realm actually executing it.

root_cause: |
  The Legion dependence analysis pipeline is not running far enough ahead
  of actual execution. The "deferred" metric measures how long a copy sat
  in the pipeline between being launched by dependence analysis and having
  its event precondition triggered. When this shrinks, execution is
  "catching up" with analysis, and bubbles form because processors must
  idle while waiting for the next piece of work to be analyzed and ready.

gotchas:
  - "Low deferred is a SYMPTOM, not a root cause. The root cause is typically missing tracing, slow mapper calls, or insufficient task granularity. Always investigate WHY the runtime is slow."
  - "A single low-deferred copy in isolation may just be a scheduling artifact. Look for a PATTERN of low-deferred items across multiple copies/tasks."
  - "Deferred applies to copies specifically (measured from Legion launch to event precondition trigger). For tasks, the equivalent diagnostic is critical-path analysis showing 'not created yet' as the blocking reason."

fix:
  primary: |
    Identify and fix the upstream bottleneck that's slowing the runtime
    pipeline. Most commonly: enable tracing (see runtime_limited_no_tracing).

  alternatives: |
    - Reduce mapper call latency (see long_mapper_calls).
    - Increase task granularity (fewer, larger tasks) to reduce per-task
      analysis overhead.
    - Check for OS scheduling interference on utility processor threads.

  what_not_to_do: |
    Do NOT try to fix low deferred by adjusting Realm configuration or
    copy scheduling — the bottleneck is upstream in the Legion analysis
    pipeline, not downstream in Realm execution.

verification: |
  After fixing the upstream bottleneck, deferred durations should return
  to tens-to-hundreds of milliseconds range. Red annotations on copies
  should disappear. GPU/CPU bubbles caused by pipeline starvation should
  shrink or vanish.

real_cases: []

related_patterns:
  - "runtime_limited_no_tracing"
  - "high_delayed_duration"
  - "long_mapper_calls"

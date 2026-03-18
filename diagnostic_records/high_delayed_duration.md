id: high_delayed_duration
title: High Delayed Duration Indicates Realm Worker Overload
source: "017 - Michael Bauer (live demo), Section 4: Deferred vs. Delayed Metrics"
confidence: medium
user_type: all

symptoms:
  what_you_see: |
    Copies or tasks that appear to start late despite their preconditions
    having fired. In the timeline view, there is a visible gap between
    when a copy's precondition triggered and when Realm actually began
    executing it. Triggering latency (from task finish to downstream
    event visibility) is also elevated.

  key_metrics: |
    - Delayed duration significantly above microseconds (exact red
      threshold not stated; healthy is "microseconds").
    - Triggering latency elevated (time from task completion to downstream
      event trigger).
    - Deferred duration may still be healthy (ruling out upstream pipeline
      starvation).

  distinguishing_features: |
    Unlike low deferred duration (upstream Legion analysis bottleneck),
    high delayed duration means the precondition HAS fired but Realm
    hasn't picked up the work yet. This points to Realm-level contention:
    background workers overloaded, thread scheduling issues, or too many
    concurrent operations competing for Realm's attention.

root_cause: |
  Once a copy's event precondition triggers, Realm must schedule and
  execute it. If Realm's worker threads are overloaded (too many
  concurrent operations, background work, or OS-level thread contention),
  there is a delay between "ready to run" and "actually running." Large
  triggering latency similarly indicates Realm is struggling to propagate
  event completions promptly.

gotchas:
  - "Legion Prof does not have visibility into Realm background worker threads or CUDA driver threads — these can cause interference that isn't directly visible in the profile."
  - "Legion Prof is not a sampling-based profiler. If a Realm thread is descheduled by the OS, Legion Prof won't know. Use a complementary sampling profiler (Nsight, VTune, magic trace) to diagnose OS-level scheduling issues."
  - "Copy profiling in Realm has a known imprecision: it makes concurrent copies appear to run simultaneously, but they likely execute sequentially on the DMA engine. Don't mistake apparent copy concurrency for actual concurrency."

fix:
  primary: |
    Use a complementary sampling-based profiler (NVIDIA Nsight Systems,
    Intel VTune, or magic trace) to identify what Realm worker threads
    are actually doing during the delay. Align the sampling profiler
    timeline with the Legion Prof timeline.

  alternatives: |
    - Check for CPU oversubscription (more threads than cores).
    - Verify SLURM/job scheduler CPU binding (misconfigured binding can
      pin all threads to one core).
    - Reduce the number of concurrent in-flight operations to ease Realm
      worker contention.

  what_not_to_do: |
    Do NOT assume the solution is to add more Realm workers without first
    profiling what existing workers are doing. The problem may be
    contention (more workers = worse) rather than insufficient parallelism.

verification: |
  After fixing, delayed durations should drop to microsecond range.
  Triggering latency should decrease. Complementary profiler should show
  Realm workers spending less time idle or contending on locks.

real_cases: []

related_patterns:
  - "low_deferred_duration"
  - "copy_profiling_imprecision"
  - "thread_descheduling_invisible"

id: thread_descheduling_invisible
title: OS Thread Descheduling Invisible to Legion Prof
source: "017 - Michael Bauer, Section 1: Limitations; Section 4: Complementary Profiling"
confidence: medium
user_type: all

symptoms:
  what_you_see: |
    Tasks or mapper calls appear to take longer than expected in Legion
    Prof, but there is no clear explanation within the profile. The
    duration of a task or mapper call seems inflated. No other visible
    activity explains the extra time. The profile "just looks slow" for
    specific items without an obvious cause.

  key_metrics: |
    - Task or mapper call durations longer than expected based on
      computational complexity.
    - No corresponding busy activity on other processors that would
      explain the gap.
    - [INCOMPLETE — needs review] No specific threshold given; detected
      by comparison to expected runtime.

  distinguishing_features: |
    This is distinguished by the ABSENCE of a visible cause in Legion
    Prof. Unlike runtime_limited_no_tracing (where utility is visibly
    saturated), or long_mapper_calls (where mapper overhead is visible),
    here the item simply looks too slow with no explanation. This is
    the clue to use a complementary profiler.

root_cause: |
  Legion Prof measures start and end times at specific instrumentation
  points. It is NOT a sampling-based profiler. If the OS descheduled
  the thread running a task or mapper call (due to oversubscription,
  CPU migration, interrupt handling, etc.), Legion Prof includes that
  descheduled time in the measured duration but cannot identify it.
  The thread may have been idle for milliseconds without Legion Prof's
  knowledge.

gotchas:
  - "This is the fundamental limitation of trace-based vs. sampling-based profiling. If you see unexplained long durations, ALWAYS consider thread descheduling."
  - "Realm background workers and CUDA driver threads are also invisible to Legion Prof and can cause interference that inflates measured durations."
  - "Machine misconfiguration (SLURM binding, NUMA issues) is a common cause of unexpected descheduling."

fix:
  primary: |
    Use a complementary sampling-based profiler simultaneously:
    - NVIDIA Nsight Systems
    - Intel VTune
    - magic trace
    Align the sampling profiler timeline with Legion Prof to identify
    descheduling events during unexpectedly long items.

  alternatives: |
    - Check machine configuration: verify CPU binding, NUMA affinity,
      and process-to-core assignment.
    - Future roadmap: Legion Prof plans to capture machine configuration
      and flag misconfiguration (e.g., process bound to one core) with
      a red warning banner.

  what_not_to_do: |
    Do NOT assume Legion Prof durations are pure computation time.
    Do NOT diagnose slow tasks as algorithmic problems without first
    ruling out OS scheduling interference via a sampling profiler.

verification: |
  If a complementary profiler shows descheduling events during the
  inflated items, fix the scheduling issue and verify durations drop
  to expected levels in both profilers.

real_cases: []

related_patterns:
  - "long_mapper_calls"
  - "slurm_misconfiguration"

id: slurm_misconfiguration
title: Process Bound to Wrong Cores Due to SLURM/Job Scheduler Misconfiguration
source: "016 - Elliott Slaughter (Planned Features); 017 - Michael Bauer (implicit)"
confidence: low
user_type: all

symptoms:
  what_you_see: |
    [INCOMPLETE — needs review] Not directly demonstrated in profiles.
    Would manifest as all threads running slower than expected (inflated
    task durations, inflated mapper calls, poor overall throughput)
    without any specific Legion-level bottleneck being visible. The
    profile looks "uniformly slow."

  key_metrics: |
    - [INCOMPLETE — needs review] No specific profiler metrics.
    - All processor types slower than expected.
    - Machine configuration check (planned feature) would flag this.

  distinguishing_features: |
    Unlike specific bottlenecks (mapper calls, tracing, copies), this
    affects EVERYTHING uniformly. All tasks are slow, all mapper calls
    are slow, all copies are slow. No single bottleneck dominates.
    A complementary profiler would show context switches and core
    contention.

root_cause: |
  Job schedulers like SLURM control CPU affinity and process binding.
  A misconfiguration can bind all threads of a process to a single
  core, causing massive contention. As Slaughter described: "if you
  have misconfigured your slurm, so it binds your process to one core,
  that would be bad."

gotchas:
  - "This is a MACHINE/DEPLOYMENT issue, not a Legion issue. It won't show up as a specific pattern in Legion Prof — everything just looks slow."
  - "Legion Prof plans to capture machine configuration in future versions and flag this with a red warning banner, but this is not yet implemented."
  - "This can be confused with thread_descheduling_invisible since both cause unexplained slowness, but misconfiguration affects ALL threads while descheduling may affect individual operations."

fix:
  primary: |
    Check CPU binding outside of Legion Prof:
    1. Use `taskset`, `numactl`, or `hwloc-bind` to inspect CPU affinity.
    2. Verify SLURM job script: check --cpu-bind, --ntasks-per-node,
       --cpus-per-task settings.
    3. Ensure each process has access to the appropriate number of cores.

  alternatives: |
    - Wait for Legion Prof's planned machine configuration capture
      feature, which would automatically flag this.
    - Use `lscpu`, `lstopo`, or `/proc/self/status` (Cpus_allowed) to
      verify actual affinity at runtime.

  what_not_to_do: |
    Do NOT try to fix this within Legion (e.g., by adjusting thread
    counts or processor configuration). Fix the job scheduler
    configuration instead.

verification: |
  After fixing SLURM binding, all task and mapper call durations
  should improve uniformly. Overall throughput should increase
  proportionally to the number of cores made available.

real_cases: []

related_patterns:
  - "thread_descheduling_invisible"
  - "long_mapper_calls"


## Source: Critical Path

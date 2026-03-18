id: cpu_uses_all_cores
title: Setting -ll:cpu to all physical cores starves utility and background threads
source: Processor kinds and task-to-processor mapping section; Anti-pattern reference table
confidence: medium
user_type: all

symptoms:
  what_you_see: |
    Runtime overhead dominates execution. Utility processors and background workers compete with application threads for CPU time. Pipeline bubbles despite adequate -ll:util count. System-level context switching visible in OS profiling.

  key_metrics: |
    -ll:cpu set equal to physical core count. Runtime overhead >10% of total execution. Utility processors show intermittent stalls despite low logical utilization.

  distinguishing_features: |
    Unlike low -ll:util (utility processors at >80% utilization), here the utility processors may show moderate logical utilization but are physically competing for cores. OS-level tools (top, htop) will show oversubscription. The key indicator is -ll:cpu = physical_cores in the command line.

root_cause: |
  When -ll:cpu equals the physical core count, there are no remaining cores for utility processors (-ll:util) and background workers (-ll:bgwork). All threads compete for the same cores via OS scheduling, introducing context-switching overhead and unpredictable latency in runtime operations.

gotchas:
  - "This is often the default configuration users try first — 'use all cores' seems intuitive but is wrong for Legion."
  - "The performance impact is non-obvious because utility processors don't show 100% utilization — they're just slow."

fix:
  primary: |
    Set -ll:cpu = physical_cores - 2 to reserve cores for runtime threads. Typical production configuration: -ll:cpu = physical_cores - 2, -ll:util 2, -ll:bgwork 3-4.

  alternatives: |
    On systems with hardware threads (hyperthreading/SMT), utility and background threads can share HT siblings with application threads, partially mitigating the issue.

  what_not_to_do: |
    Do NOT set -ll:cpu equal to or greater than physical cores. Do NOT assume that more CPU processors always means better performance.

verification: |
  After reserving cores, runtime overhead should decrease. Pipeline throughput should improve. Utility processor responsiveness should increase (visible as reduced gap between runtime IDs and executing IDs).

real_cases: []

related_patterns:
  - "low_utility_processors"

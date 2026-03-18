id: low_utility_processors
title: Insufficient utility processors (-ll:util too low) bottleneck dependence analysis
source: Processor kinds and task-to-processor mapping section; Anti-pattern reference table
confidence: medium
user_type: all

symptoms:
  what_you_see: |
    Utility processors show >80% utilization while application processors have execution bubbles (gaps). The runtime falls behind execution — meta-task IDs on utility processors are close to executing task IDs rather than 10s–100s ahead. Mapper calls (map_task, select_task_options) dominate utility processor time.

  key_metrics: |
    Utility processor utilization >80%. Execution bubbles on application processors. Runtime meta-task IDs close to executing task IDs. Runtime overhead >10% of total execution time.

  distinguishing_features: |
    Unlike missing tracing (which shows repeated analysis every iteration), this pattern shows the utility processors are genuinely saturated — even with tracing, complex applications may need more utility processors. Unlike individual task launches (O(N) analysis cost), the issue is throughput, not per-task cost. The fix is adding resources, not changing the algorithm.

root_cause: |
  The default -ll:util 1 allocates only one utility processor for runtime meta-tasks including dependence analysis, mapping, and instance management. Complex applications overwhelm a single utility processor, creating a pipeline bottleneck that starves application processors of work.

gotchas:
  - "The default -ll:util is 1, which is insufficient for most non-trivial applications."
  - "Setting -ll:cpu too high (equal to physical cores) can starve utility processors of CPU resources even if -ll:util is adequate."
  - "Utility processors run on the same physical cores as CPU tasks — they compete for resources."

fix:
  primary: |
    Increase -ll:util to 2–4 for complex applications. The typical production configuration: -ll:cpu = physical_cores - 2, -ll:util 2, -ll:bgwork 3-4.

  alternatives: |
    Combine with tracing to reduce per-iteration analysis load. Use index launches to reduce per-task analysis cost. Increase -lg:sched and -lg:width to improve pipeline throughput.

  what_not_to_do: |
    Do NOT set -ll:cpu equal to physical cores — this starves utility and background threads. Do NOT increase -ll:util without also ensuring sufficient physical cores are available.

verification: |
  After increasing -ll:util, utility processor utilization should drop below 80%. Runtime meta-task IDs should run 10s–100s ahead of executing task IDs. Execution bubbles on application processors should shrink. Runtime overhead should drop below 10%.

real_cases: []

related_patterns:
  - "cpu_uses_all_cores"
  - "missing_tracing"
  - "individual_task_launches"

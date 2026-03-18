id: individual_task_launches
title: Individual task launches instead of index launches cause O(tasks) analysis overhead
source: Dependency analysis and the task dataflow graph section; Anti-pattern reference table
confidence: high
user_type: all

symptoms:
  what_you_see: |
    Per-task analysis overhead visible as heavy utility processor activity. Pipeline bubbles in execution timelines. Utility processor meta-task IDs are close to executing task IDs (runtime is not running ahead). Large gaps between task executions on application processors.

  key_metrics: |
    Analysis time ≈ N × single-launch cost vs. ≤3 ms for an index launch even at extreme partition sizes. Runtime IDs close to executing IDs rather than 10s–100s ahead. Utility processors heavily loaded with per-task dependence analysis.

  distinguishing_features: |
    Unlike missing tracing (which repeats analysis across iterations), this pattern shows excessive per-launch analysis within a single iteration. Unlike low -ll:util (insufficient utility processors), the root cause is O(N) analysis cost that could be O(partition) with index launches. The SC '21 paper showed index launch analysis takes ≤3 ms even for extreme partition sizes, roughly equal to one single-task launch.

root_cause: |
  Individual task launches force per-task dependence analysis. Index launches amortize analysis to O(partition) rather than O(tasks). Launching thousands of individual tasks instead of one index launch incurs per-task analysis cost, each roughly equal to one index launch. This is a fundamental algorithmic difference in the SOOP pipeline's dependence analysis stage.

gotchas:
  - "Even if tasks are logically identical except for their subregion, individual launches prevent amortized analysis."
  - "The SOOP window size (-lg:window, default 1024) caps outstanding operations — individual launches fill this window much faster."
  - "This anti-pattern can degrade performance by orders of magnitude for large task collections."

fix:
  primary: |
    Use IndexTaskLauncher for all collections of similar tasks operating on subregions of a partition. This reduces analysis from O(tasks) to O(partition).

  alternatives: |
    If tasks are heterogeneous and cannot use index launches, increase -lg:window to allow more outstanding operations. Increase -ll:util to add utility processors for parallel analysis.

  what_not_to_do: |
    Do NOT launch thousands of individual tasks when an index launch would work. Do NOT assume adding utility processors will compensate for O(N) vs. O(partition) analysis cost differences.

verification: |
  After switching to index launches, utility processor load should decrease substantially. The gap between runtime meta-task IDs and executing task IDs should widen to 10s–100s ahead. Pipeline bubbles should shrink or disappear. Analysis time for the launch should be ≤3 ms regardless of partition size.

real_cases:
  - case: "SC '21 paper"
    app: "[general benchmark]"
    scale: "[extreme partition sizes]"
    result: "Index launch analysis ≤3 ms vs. N × single-launch cost"
    key_detail: "The amortization is the key insight — analysis cost becomes independent of task count."

related_patterns:
  - "missing_tracing"
  - "low_utility_processors"
  - "tasks_too_small"

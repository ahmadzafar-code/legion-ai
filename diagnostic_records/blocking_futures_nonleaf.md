id: blocking_futures_nonleaf
title: Blocking on futures in non-leaf tasks prevents SOOP from running ahead
source: Dependency analysis and the task dataflow graph section; Anti-pattern reference table
confidence: high
user_type: all

symptoms:
  what_you_see: |
    Pipeline bubbles in processor execution timelines. The runtime cannot run ahead — meta-task IDs on utility processors are close to executing task IDs. Runtime warning 1047 fires. Execution stalls while waiting for future resolution.

  key_metrics: |
    Runtime IDs close to executing IDs (healthy: 10s–100s ahead). Warning 1047 present in output. Pipeline depth near zero during blocking periods.

  distinguishing_features: |
    Unlike individual task launches (where analysis is simply slow), here the pipeline is explicitly stalled by a blocking operation. Warning 1047 is the definitive distinguishing feature. Unlike missing tracing (repeated analysis across iterations), this is a within-iteration pipeline stall. The is_empty test variant produces warning 1001 with "severe performance degradation."

root_cause: |
  When a non-leaf task blocks on a future (e.g., calling get_result() on a future before it's ready), the SOOP pipeline cannot continue launching subsequent operations. The pipeline stalls, exposing the full latency of the blocked future. This prevents the runtime from building up a sufficient window of outstanding operations to hide latency.

gotchas:
  - "Warning 1001 (blocking is_empty test in non-leaf tasks) is a sub-variant that causes 'severe performance degradation.'"
  - "The fix is conceptual, not configurational — it requires restructuring task logic to pass futures as arguments rather than blocking on them."
  - "Tracing cannot help here because the pipeline stall prevents the runtime from recording future operations."

fix:
  primary: |
    Never block on futures in non-leaf tasks. Pass futures as arguments to subsequent tasks instead, allowing the runtime to establish data dependencies without blocking the pipeline.

  alternatives: |
    If the future value is needed for a control-flow decision, restructure the algorithm to move the decision into a leaf task or use predication.

  what_not_to_do: |
    Do NOT call get_result() on futures in non-leaf tasks. Do NOT use blocking is_empty() tests in non-leaf tasks (warning 1001). Do NOT assume increasing -lg:window will fix this — the pipeline is blocked, not just slow.

verification: |
  After restructuring, warning 1047 (and 1001 if applicable) should no longer appear. Runtime meta-task IDs should run 10s–100s ahead of executing task IDs. Pipeline bubbles should shrink dramatically.

real_cases: []

related_patterns:
  - "individual_task_launches"
  - "missing_tracing"

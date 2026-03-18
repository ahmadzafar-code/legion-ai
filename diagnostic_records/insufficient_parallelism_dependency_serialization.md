id: insufficient_parallelism_dependency_serialization
title: Tasks serialized by dependencies — scalar materialization fences
source: low_processor_utilization_diagnosis.md, Category 3; GitHub issue #1203
confidence: high
user_type: regent legate

symptoms:
  what_you_see: |
    Gaps appear simultaneously across all application processors (same
    as other Category 3 patterns). Utility processors show alternating
    phases: saturation (catching up after a fence) and idleness (waiting
    for the fence result). Periodic, repeating gap-burst patterns in the
    timeline correspond to iterative convergence checks. The critical path
    overlay shows a long chain passing through nearly every time step.

  key_metrics: |
    - Q3.2: Periodic time slices with <20% utilization
    - Q3.3: Utility processors also idle during some gaps but busy
      catching up in others (hybrid pattern with Category 1)
    - Task graph forms a long chain rather than a wide DAG
    - CUPYNUMERIC_DOCTOR=1 detects scalar materialization anti-pattern
      (Legate/cuPyNumeric)

  distinguishing_features: |
    Unlike "too few tasks" (Category 3a), sufficient tasks exist but
    they form a long chain. The distinguishing visual is the hybrid
    utility processor pattern: alternating saturation and idleness
    rather than continuous idleness (Category 3a) or continuous
    saturation (Category 1). Scalar materializations create implicit
    execution fences that both serialize the pipeline (Category 3) AND
    prevent run-ahead distance from building (Category 1).

root_cause: |
  Operations like np.linalg.norm(), print(array_value), or any
  reduction to a scalar force a scalar materialization — an implicit
  execution fence that drains the entire pipeline before the scalar
  can be returned to Python. Each convergence check in an iterative
  solver creates such a fence. The runtime cannot build run-ahead
  distance past these fences. Blocking on a future in the top-level
  task (flagged by -lg:warn) similarly prevents run-ahead. In GitHub
  issue #1203, blocking in a non-leaf task was confirmed as the root
  cause of scaling failure.

gotchas:
  - "This is a hybrid Category 1 + Category 3 pattern — utility processors alternate between saturated and idle. Diagnosing as purely Category 1 or purely Category 3 leads to incomplete fixes."
  - "The co-occurrence section explicitly states: 'Fix the blocking operations first — this resolves both symptoms.'"
  - "In Legate, CUPYNUMERIC_DOCTOR=1 automatically detects this anti-pattern."
  - "-lg:warn flags blocking in non-leaf tasks — check runtime warnings."

fix:
  primary: |
    Reduce scalar materializations: check convergence every N iterations
    instead of every iteration. In cuPyNumeric, set CUPYNUMERIC_DOCTOR=1
    to automatically detect this anti-pattern.

  alternatives: |
    - Remove unnecessary explicit execution fences
      (runtime->issue_execution_fence()) that are not required for
      correctness.
    - Remove must_epoch launches that serialize the pipeline.
    - Avoid blocking on futures in non-leaf tasks (flagged by -lg:warn).
    - Restructure iterative loops to allow speculative execution past
      convergence checks.

  what_not_to_do: |
    Do NOT fix this by only adding -dm:memoize or -ll:util — while
    those help with the Category 1 component, the fundamental
    serialization from fences remains. Fix the blocking operations
    first, then address any remaining runtime overhead.

verification: |
  After reducing fence frequency:
  1. The periodic gap-burst pattern should become less frequent or
     disappear.
  2. Utility processor alternating saturation/idleness should smooth out.
  3. Run-ahead distance should increase past former fence points.
  4. Overall execution time should decrease proportionally to fence
     reduction.

real_cases:
  - case: "GitHub issue #1203"
    app: "Not specified"
    scale: "Not specified"
    result: "Application scaled correctly after removing blocking call"
    key_detail: "Blocking on a future in the top-level task was confirmed as root cause of scaling failure"

related_patterns:
  - "insufficient_parallelism_too_few_tasks"
  - "runtime_overhead_no_tracing"

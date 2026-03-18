id: cupynumeric_elementwise_loops
title: Python for-loops over array elements instead of vectorized operations
source: NVIDIA cuPyNumeric Best Practices docs; Case 18b
confidence: medium
user_type: legate

symptoms:
  what_you_see: |
    Profiler shows an enormous number of tiny tasks — one per array
    element — instead of a few bulk tasks. Execution is dominated by
    per-task overhead. Wall-clock time is far worse than expected for
    the data size.

  key_metrics: |
    Task count: proportional to array size (one per element). Per-task
    execution time: tiny (dominated by launch overhead). Task-to-overhead
    ratio: extremely unfavorable.

  distinguishing_features: |
    Unlike task fusion needs (Case 9) where bulk operations produce
    moderate numbers of tasks, here the task count is proportional to
    ARRAY SIZE because of Python-level element-wise access. The fix is
    at the Python level, not the runtime level.

root_cause: |
  Python for-loops over array elements (e.g., `for i in range: x[0,j,i]
  = y[3,j,i]`) launch one tiny task per element instead of one bulk
  task. This is a user-level anti-pattern, not a runtime bug.

gotchas:
  - "This is the same performance anti-pattern as in regular NumPy, but the penalty is far worse in Legate because each element access becomes a distributed task."
  - "The fix is always at the Python application level — no runtime optimization can help."

fix:
  primary: |
    Replace element-wise loops with vectorized NumPy operations:
    `x[0] = y[3]` instead of `for i in range: x[0,j,i] = y[3,j,i]`.

  alternatives: |
    If element-wise access is truly needed, accumulate into a local
    NumPy array and then assign in bulk to the cuPyNumeric array.

  what_not_to_do: |
    Do NOT try to fix this with task fusion or runtime tuning — the
    problem is in the Python code structure.

verification: |
  Task count should drop from proportional-to-array-size to a small
  constant number of bulk operations. Wall-clock time should improve
  dramatically.

real_cases:
  - case: "cuPyNumeric Best Practices documentation"
    app: "General cuPyNumeric applications"
    scale: "Any"
    result: "[No specific quantitative result documented]"
    key_detail: "Same anti-pattern as NumPy but with much worse penalty due to distributed task overhead"

related_patterns:
  - legate_task_fusion

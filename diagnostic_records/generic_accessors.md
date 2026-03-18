id: generic_accessors
title: Generic accessors cause 2–3× per-element access slowdown
source: Task granularity and launch overhead section; Anti-pattern reference table
confidence: medium
user_type: legion_cpp

symptoms:
  what_you_see: |
    High per-element access time within tasks. Tasks take longer than expected based on computational complexity. Warning 1091 fires: "Generic accessors are very slow." Task execution time dominated by data access overhead rather than computation.

  key_metrics: |
    2–3× accessor slowdown (warning 1091). Per-element access time significantly higher than expected. Warning 1091 present in runtime output.

  distinguishing_features: |
    Unlike tasks-too-small (narrow task bars), the task bars may be appropriately sized but individual tasks are slower than they should be. The warning 1091 is the definitive diagnostic. The overhead is inside the task, not in the runtime pipeline.

root_cause: |
  Generic accessors perform runtime type checking and indirection on every element access. Typed FieldAccessor<PRIVILEGE, TYPE, DIM> compiles to direct memory access with compile-time type safety, eliminating per-access overhead.

gotchas:
  - "The 2–3× slowdown is per-element, so it multiplies with the number of elements accessed per task."
  - "For ReductionAccessor, setting the exclusive template parameter to true when no concurrent access exists eliminates atomic instructions for ~7× speedup."
  - "Warning 1091 may be suppressed or missed in noisy output — look for it specifically."

fix:
  primary: |
    Use typed FieldAccessor<PRIVILEGE, TYPE, DIM> instead of generic accessors. For ReductionAccessor, set the exclusive template parameter to true when the task has exclusive access (no concurrent reductions).

  alternatives: |
    [INCOMPLETE — needs review] No alternative to switching accessor types if performance is the goal.

  what_not_to_do: |
    Do NOT use generic accessors in performance-sensitive code. Do NOT set ReductionAccessor exclusive=true if concurrent access actually exists — this introduces data races.

verification: |
  After switching to typed accessors, warning 1091 should disappear. Per-task execution time should decrease by 2–3×. For ReductionAccessor with exclusive=true, an additional ~7× speedup in reduction operations should be visible.

real_cases: []

related_patterns:
  - "tasks_too_small"
  - "unmarked_leaf_inner"

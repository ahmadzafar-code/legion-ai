id: blocking_python_operations
title: Blocking Python operations drain the Legate deferred execution pipeline
source: GPU differential diagnosis guide, Cause 4; cuPyNumeric best practices documentation; Legion v25.03.0 backtrace feature; NVIDIA cuPyNumeric Best Practices docs; Case 18a
confidence: medium
user_type: legate

symptoms:
  what_you_see: |
    Python/CPU processor row shows the top-level task in a waiting/blocked state
    (lighter colored box or absence of activity). Utility processors are idle or
    caught up — the task IDs being mapped are very close to those being executed,
    instead of being 10s–100s ahead. No channel or copy activity is present. The
    pattern is periodic if the blocking operation occurs inside a loop — a
    "sawtooth" or "comb" pattern of work bursts separated by gaps, with each
    burst corresponding to the pipeline refilling after Python resumes. GPUs
    alternate between busy and idle in a regular pattern corresponding to
    operations that force synchronization: `bool(array)`, `float(array)`,
    `print(array)`, or per-iteration convergence checks.

  key_metrics: |
    - Python/CPU processor `waiting` time is high during [T1, T2]
    - Utility processor meta-task count during gap: low or zero (runtime has
      caught up to execution)
    - No channel activity during the gap
    - Periodic gap pattern with regular spacing (if blocking op is in a loop)
    - Gap duration correlates with the time to materialize a concrete value
    - GPU utilization: alternating busy/idle pattern
    - Pipeline depth: effectively 1 (no overlap between task submission and
      prior results)
    - Synchronization operations identifiable in the task timeline

  distinguishing_features: |
    Unlike Cause 1 (scalar reduction), the CPU/Python processor is
    BLOCKED/WAITING during the gap, not actively computing. Unlike Cause 3
    (missing tracing), utility processors are IDLE during the gap, not busy
    with mapper calls. Unlike Cause 5 (network congestion), no channel activity
    is present. The "sawtooth"/"comb" pattern of work bursts separated by
    pipeline-drain gaps is distinctive. The pattern is regular and corresponds
    to specific Python operations that materialize values. The backtrace feature
    (v25.03.0+) directly identifies the blocking call site.

root_cause: |
  cuPyNumeric operates on a deferred execution model: `z = x + y` submits a
  task without waiting for completion. Python continues submitting tasks while
  the GPU executes earlier ones. Any operation that forces Python to wait for a
  concrete value — `__bool__()` in an `if norm < tolerance:` check, `__int__()`,
  `__float__()`, `print(array)`, `.item()`, `.tolist()`, or `__array__()` —
  blocks the Python thread, halting task submission. The GPU drains its ready
  queue and goes idle. Each synchronization point drains the pipeline and waits
  for all pending work to complete before proceeding.

  IMPORTANT: `.shape`, `.ndim`, `.dtype`, and `.size` are NOT blocking — these
  are metadata stored locally. Only computed values trigger blocking.

gotchas:
  - "A simple `if norm < tolerance:` in a convergence check calls __bool__() which blocks — this is the most common source"
  - "Convergence checks every iteration are the most common real-world source — batch to every N iterations"
  - "Even a single synchronization point per iteration can destroy GPU pipelining"
  - "print(array) forces materialization — remove print statements from performance-critical loops"
  - "print() during debugging is a common accidental source of pipeline stalls"
  - ".shape, .ndim, .dtype, and .size are NOT blocking — only operations that materialize a computed value block"
  - "The sawtooth pattern can be confused with Cause 7 (insufficient parallelism) if you don't check Python processor state"
  - "The backtrace feature (v25.03.0+) directly identifies the blocking Python source line — use it instead of guessing"

fix:
  primary: |
    Restructure Python code to minimize blocking:
    - Check convergence every N iterations instead of every iteration
    - Use `np.where()` instead of Python `if` on array-derived scalars
    - Use array-based logical operations (`np.logical_and`) instead of
      element-wise conditionals
    - Avoid `print(array)` in performance-critical loops
    - Remove `bool(array)`, `float(array)` from inner loops
    - Batch small operations into larger tasks that keep the GPU busy for
      ≥1ms each

  alternatives: |
    Use the backtrace feature (Legion v25.03.0+) to identify the exact Python
    source line causing the stall, then refactor that specific line. For
    convergence checks that must happen every iteration, consider using a
    Legate future callback mechanism if available. Use asynchronous convergence
    checking if supported. Structure the algorithm to reduce the frequency of
    global synchronization.

  what_not_to_do: |
    Do NOT assume .shape or .ndim access is the problem — these are metadata
    and never block. Do NOT add -cuda:legacysync to fix this pattern — the
    problem is Python blocking, not CUDA stream interference. Do NOT check
    convergence every iteration on GPUs. Do NOT use print(array) for debugging
    in performance-critical code paths.

verification: |
  After restructuring the blocking calls, the "sawtooth"/"comb" pattern should
  disappear or its period should increase (if convergence checks were moved to
  every N iterations). The Python processor should show continuous task
  submission activity. Utility processors should show the runtime staying far
  ahead of execution (task IDs being mapped should be 10s–100s ahead of those
  being executed). GPU timeline should show continuous utilization without
  regular idle gaps. Pipeline depth should increase (tasks overlapping with
  prior computation).

real_cases:
  - case: "[No specific issue number cited]"
    app: "cuPyNumeric applications (general pattern)"
    scale: "Any scale"
    result: "[Not quantified — documented as best practice]"
    key_detail: "cuPyNumeric best practices documentation is the primary source; backtrace feature added in v25.03.0 enables direct identification"
  - case: "cuPyNumeric Best Practices documentation"
    app: "General cuPyNumeric applications"
    scale: "Any"
    result: "[No specific quantitative result documented]"
    key_detail: "Convergence checks every iteration are the most common real-world source of this anti-pattern"

related_patterns:
  - "scalar_reduction_blocking"
  - "insufficient_parallelism"
  - "legate_partition_cache_bug"
```

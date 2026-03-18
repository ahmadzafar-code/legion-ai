id: legate_gc_oom
title: Distributed garbage collection causing OOM in iterative Legate loops
source: GitHub nv-legate/cunumeric#33; Case 8
confidence: high
user_type: legate

symptoms:
  what_you_see: |
    During iterative loops (e.g., stencil.py), Legate profiler shows
    proliferating memory allocations with unbounded memory growth until
    out-of-memory crash. The stencil benchmark crashes at iteration 14.
    Memory timeline shows monotonic growth for an algorithm that should
    have constant memory requirements.

  key_metrics: |
    Memory allocation: monotonic growth, unbounded. Crash point:
    iteration 14 for stencil benchmark. Per-iteration memory delta:
    positive and constant (should be zero). NUMPY_FIELD_REUSE_FREQ
    setting determines crash timing.

  distinguishing_features: |
    Unlike eager pool exhaustion (Case 18d), this is about field reuse
    across iterations, not about scratch space within a single operation.
    The memory growth is monotonic across iterations, not a single-
    operation spike. The crash occurs at a consistent iteration count
    determined by available memory and per-iteration leak rate.

root_cause: |
  Legate uses a different "field" to back each live ndarray. After GC,
  fields enter a local queue but aren't redistributed until all shards
  synchronize. The default `NUMPY_FIELD_REUSE_FREQ` was too infrequent,
  causing memory allocation to outpace recycling. Additionally, Python
  reference cycles in user code prevented deterministic collection. A
  fundamental tension: deferring field reuse expands task-parallelism
  (more independent fields for Legion) but increases memory pressure.

gotchas:
  - "FREQ=1 overhead is near-zero for single-node runs but may have synchronization cost at scale — test on your target configuration."
  - "Python reference cycles are a separate contributor — breaking cycles in legate.core (PR #84) was needed alongside the frequency fix."
  - "There is a fundamental parallelism–memory tradeoff: more aggressive reuse limits the parallelism Legion can discover."

fix:
  primary: |
    Set `NUMPY_FIELD_REUSE_FREQ=1` to force garbage collection on every
    allocation. For single-node runs, overhead is near-zero.

  alternatives: |
    Break Python reference cycles in application code. Apply PR #84
    in legate.core which breaks cycles in core data structures.
    Tune `NUMPY_FIELD_REUSE_FREQ` to balance parallelism vs. memory
    pressure for your specific workload.

  what_not_to_do: |
    Do NOT increase available memory as the only fix — the leak is
    unbounded and will eventually exhaust any amount of memory. The
    root cause is a recycling rate mismatch, not insufficient memory.

verification: |
  With FREQ=1, stencil benchmark goes from crashing at iteration 14 to
  running past iteration 57. Memory growth is significantly slowed.
  Per-iteration memory delta should approach zero.

real_cases:
  - case: "GitHub cunumeric#33"
    app: "Stencil benchmark (Legate/cuPyNumeric)"
    scale: "Single node"
    result: "From crash at iteration 14 to running past iteration 57"
    key_detail: "NUMPY_FIELD_REUSE_FREQ=1 fixed the recycling rate; Python reference cycles were a separate contributor"

related_patterns:
  - cupynumeric_eager_pool_exhaustion
  - legate_partition_cache_bug

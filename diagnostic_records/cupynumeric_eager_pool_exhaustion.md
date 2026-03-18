id: cupynumeric_eager_pool_exhaustion
title: Eager memory pool exhaustion from high-scratch-space operations
source: NVIDIA cuPyNumeric Best Practices docs; Case 18d
confidence: medium
user_type: legate

symptoms:
  what_you_see: |
    Runtime error: `LEGION ERROR: Failed to allocate DeferredBuffer`.
    Operations with high scratch space requirements (einsum, convolve)
    fail. The error occurs during execution, not at startup.

  key_metrics: |
    Error message: "LEGION ERROR: Failed to allocate DeferredBuffer".
    Operations triggering: einsum, convolve, and other high-scratch-space
    operations.

  distinguishing_features: |
    Unlike GC-related OOM (Case 8) which shows gradual memory growth over
    iterations, this is a single-operation spike from scratch space
    requirements. The error message specifically mentions DeferredBuffer,
    not general OOM. It's triggered by specific high-scratch operations,
    not by iteration count.

root_cause: |
  Operations with high scratch space requirements (einsum, convolve)
  exhaust the eager memory pool. The artificial deferred/eager memory
  pool split in pre-25.01 Legate creates an artificial boundary that
  prevents one pool from using the other's free memory.

gotchas:
  - "This was architecturally addressed in Legate 25.01 with a unified memory pool — upgrade if possible."
  - "The deferred/eager split is artificial — there's no fundamental reason scratch space can't use deferred memory."

fix:
  primary: |
    Upgrade to Legate 25.01 or later, which replaces the artificial
    deferred/eager split with a unified memory pool.

  alternatives: |
    For pre-25.01: reduce the eager pool size to free memory for deferred
    allocations, or reduce the problem size for high-scratch operations.

  what_not_to_do: |
    Do NOT confuse this with GC-related OOM (Case 8). The fix for Case 8
    (NUMPY_FIELD_REUSE_FREQ) won't help here because the issue is
    scratch space allocation, not field recycling.

verification: |
  The DeferredBuffer allocation error should not occur after upgrading
  to Legate 25.01+. High-scratch operations (einsum, convolve) should
  execute successfully.

real_cases:
  - case: "cuPyNumeric Best Practices documentation"
    app: "cuPyNumeric applications using einsum, convolve"
    scale: "Any"
    result: "Architecturally resolved in Legate 25.01 with unified memory pool"
    key_detail: "Artificial deferred/eager pool split was the root cause; unified pool in 25.01 was the architectural fix"

related_patterns:
  - legate_gc_oom

---

## Summary
- Total records extracted: 20
- High confidence: 16 (real diagnosed cases with verification — Cases 1–17)
- Medium confidence: 4 (documented patterns from cuPyNumeric best practices — Cases 18a–d)
- Low confidence: 0
- Gaps identified:
  - **Case 16 network congestion**: The communication scaling issue at 32+ nodes was left as an open investigation — no fix or verification was documented.
  - **Case 17 specific fix**: The exact code change that resolved the HTR regression is not documented — only that it was resolved in subsequent commits.
  - **Case 12 quantitative results**: The visibility algorithms paper documents asymptotic improvements but specific speedup numbers on real applications are not provided in this catalog.
  - **Profiling methodology section**: The document references a general diagnostic workflow (task-ID gap heuristic, critical-path visualization via `press 'a'`, VTune/Nsight Systems complementing Legion Prof) but does not provide enough detail for a standalone diagnostic record.
  - **Task Bench cross-system comparison**: The SC 2020 paper tested 15 systems with >5 orders of magnitude variation in METG, but individual system results beyond Realm and Chapel are not detailed here.
  - **Legate 25.01 unified memory pool**: Referenced as the fix for Case 18d, but no performance verification data is provided.


## Source: GPU Diagnosis

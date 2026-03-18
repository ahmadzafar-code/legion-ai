id: cupynumeric_api_fallback
title: Unimplemented cuPyNumeric API silently falling back to single-threaded NumPy
source: NVIDIA cuPyNumeric Best Practices docs; Case 18c
confidence: medium
user_type: legate

symptoms:
  what_you_see: |
    Sudden, dramatic performance cliff when using specific API calls.
    The profiler shows computation happening on a single CPU instead of
    distributed across GPUs. GPU utilization drops to zero for the
    affected operations. No error or warning is printed by default.

  key_metrics: |
    GPU utilization: 0% for affected operations. CPU utilization: single
    thread active. Performance cliff: orders of magnitude slower than
    expected for distributed execution.

  distinguishing_features: |
    Unlike other patterns where the runtime is the bottleneck, here the
    computation is happening correctly — but on the WRONG processor
    (single-threaded CPU instead of distributed GPUs). The silent
    fallback with no warning is the distinguishing feature.

root_cause: |
  Unimplemented cuPyNumeric APIs silently fall back to single-threaded
  NumPy on CPU, causing sudden performance cliffs. There is no error or
  warning by default — the operation produces correct results but at
  dramatically lower performance.

gotchas:
  - "The fallback produces CORRECT results — you won't catch this via correctness testing, only via performance testing."
  - "New cuPyNumeric versions may implement previously-missing APIs — check release notes when upgrading."

fix:
  primary: |
    Set `CUPYNUMERIC_DOCTOR=1` environment variable to diagnose anti-
    patterns automatically. This will report when operations fall back
    to NumPy.

  alternatives: |
    Check the cuPyNumeric documentation for implemented vs. unimplemented
    APIs before using them in performance-critical code. Restructure code
    to avoid unimplemented operations.

  what_not_to_do: |
    Do NOT assume all NumPy APIs are accelerated in cuPyNumeric. Do NOT
    rely on correctness testing to catch performance regressions from
    fallback — the results are correct, just slow.

verification: |
  With `CUPYNUMERIC_DOCTOR=1`, fallback operations are reported. GPU
  utilization should increase for operations that were previously falling
  back. No more single-CPU computation for distributed operations.

real_cases:
  - case: "cuPyNumeric Best Practices documentation"
    app: "General cuPyNumeric applications"
    scale: "Any"
    result: "[No specific quantitative result documented]"
    key_detail: "Silent fallback — correct results but orders of magnitude slower; CUPYNUMERIC_DOCTOR=1 is the diagnostic"

related_patterns:
  - cupynumeric_pipeline_stalls

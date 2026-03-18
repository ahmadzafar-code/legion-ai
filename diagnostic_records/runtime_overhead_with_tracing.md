id: runtime_overhead_with_tracing
title: Runtime overhead persists despite tracing being active
source: low_processor_utilization_diagnosis.md, Category 1; ASPLOS 2025 (Apophenia)
confidence: medium
user_type: all

symptoms:
  what_you_see: |
    Utility processors show periodic bursts of "Replay Physical Trace"
    (enum ID 46) meta-tasks — tracing IS active — but utility processors
    remain at >80% utilization. Application processors still show gaps.
    "Logical Dependence Analysis" may also be present alongside replay
    tasks, indicating partial tracing coverage (some operations traced,
    others not).

  key_metrics: |
    - Utility processor saturation >0.8 (Q1.1)
    - "Replay Physical Trace" count >0 (Q1.3) — tracing is active
    - "Logical Dependence Analysis" still significant (Q1.4) — partial coverage
    - Average task duration near or below ~100μs (traced METG threshold)
    - Run-ahead distance still compressed (<50 op IDs)

  distinguishing_features: |
    Unlike the no-tracing variant, "Replay Physical Trace" tasks ARE
    present. But utility saturation remains high because either: (a) trace
    replay itself is the bottleneck (too many traces, or very large traces),
    (b) some operations fall outside traces and still require fresh analysis,
    or (c) task granularity is below the traced METG of ~100μs.

root_cause: |
  Even with tracing, per-task overhead is ~100μs. If task durations are
  near or below this threshold, the runtime pipeline still cannot keep up.
  Additionally, if the application has dynamic control flow or
  non-repeating phases, not all operations can be captured in traces,
  leaving some to require fresh analysis. Insufficient utility processors
  (-ll:util) can also bottleneck trace replay throughput.

gotchas:
  - "Traced METG is ~100μs. Tasks shorter than this will bottleneck on runtime overhead even with perfect tracing."
  - "Partial tracing coverage (some ops traced, some not) can look confusing — both Replay and Analysis tasks appear on utility processors."
  - "Dynamic control flow (data-dependent branches, variable iteration counts) can prevent effective trace construction."

fix:
  primary: |
    Increase task granularity above 100μs (the traced METG threshold).
    This may require increasing problem size per task or coarsening
    partitions.

  alternatives: |
    - -ll:util N: Increase utility processors to 2–4 to increase trace
      replay throughput.
    - -lg:window N: Increase to 2048–8192 to give the pipeline more
      buffering capacity.
    - Ensure all iterative loops are covered by traces — restructure
      dynamic control flow to enable full trace coverage.
    - -dm:replicate: If not already enabled, add control replication to
      distribute replay work across nodes.

  what_not_to_do: |
    Do NOT add -dm:memoize again if it is already active — tracing is
    already enabled, the problem is either task granularity or trace
    coverage. Do NOT assume tracing "isn't working" just because utility
    processors are busy — check whether the busy time is replay vs.
    fresh analysis.

verification: |
  After increasing task granularity or adding utility processors:
  1. Utility processor utilization should drop below 50%.
  2. Run-ahead distance should increase substantially.
  3. "Replay Physical Trace" should remain dominant over
     "Logical Dependence Analysis" on utility processors.
  4. Application processor gaps should shrink.

real_cases:
  - case: "[INCOMPLETE — needs review]"
    app: "[INCOMPLETE — needs review]"
    scale: "[INCOMPLETE — needs review]"
    result: "METG drops from ~1ms (untraced) to ~100μs (traced) per Task Bench (SC 2020)"
    key_detail: "Task Bench defined METG as the shortest task duration at which a system maintains ≥50% efficiency"

related_patterns:
  - "runtime_overhead_no_tracing"
  - "small_tasks_below_metg"

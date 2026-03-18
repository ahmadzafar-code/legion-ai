id: htr_runtime_regression
title: HTR solver 10% regression from runtime commit — super-linear initialization growth
source: GitHub StanfordLegion/legion#1652; Case 17
confidence: high
user_type: legion_cpp

symptoms:
  what_you_see: |
    Comparing two Legion runtime commits (cba415a vs. 91b55ce) with
    identical HTR application code shows ~10% increase in time per step
    with minor GPU utilization degradation. More critically, a very large
    idle section at the beginning of the run, growing super-linearly with
    node count. On 8 nodes: old commit ~46s total vs. new commit ~107s
    total (2.3× slower overall).

  key_metrics: |
    Time per step: ~10% increase. Total runtime on 8 nodes: 46s → 107s
    (2.3× slower). Initialization idle time: growing super-linearly with
    node count. GPU utilization in steady state: minor degradation.

  distinguishing_features: |
    This is a REGRESSION between specific runtime commits, not a
    persistent pattern. Distinguished by the side-by-side commit
    comparison methodology. The super-linear initialization growth with
    node count is the key symptom — steady-state per-step degradation
    is minor (~10%) but the initialization penalty dominates.

root_cause: |
  A performance regression introduced between two specific Legion runtime
  commits. The idle initialization time grew with some power of the
  number of nodes, suggesting a scalability bug in the startup or initial
  analysis path.

gotchas:
  - "The 10% per-step regression is a distraction — the real problem is the initialization phase growing super-linearly with node count."
  - "This requires bisecting runtime commits to identify the regression, not application-level debugging."
  - "Side-by-side Legion Prof comparison with identical application code is the diagnostic methodology."

fix:
  primary: |
    Resolved in subsequent Legion commits (issue closed, tracked as part
    of release milestone #1032). The specific fix addressed the scalability
    bug in the startup/initial analysis path.

  alternatives: |
    Pin to the known-good commit (cba415a) until the regression is fixed.
    This is the standard approach for runtime regressions.

  what_not_to_do: |
    Do NOT try to fix this at the application level — it's a runtime
    regression. Do NOT focus on the 10% per-step degradation when the
    initialization super-linear growth is the dominant problem.

verification: |
  Issue closure confirmed the regression was fixed. 8-node runtime
  should return to the ~46s baseline. Initialization phase should not
  grow super-linearly with node count.

real_cases:
  - case: "GitHub legion#1652"
    app: "HTR solver"
    scale: "Up to 8 nodes"
    result: "2.3× overall slowdown fixed by runtime commit fix"
    key_detail: "Initialization time grew super-linearly with node count — the per-step regression was secondary"

related_patterns:
  - control_replication_bottleneck

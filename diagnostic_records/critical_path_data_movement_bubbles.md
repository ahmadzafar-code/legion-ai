id: critical_path_data_movement_bubbles
title: Timeline bubbles from untracked data movement masquerading as critical path gaps
source: criticalpath-research.md, section "Architecture determines which operations enter the DAG"
confidence: low — mentioned in passing via a documentation quote, no profiler signature or specific case detailed
user_type: all

symptoms:
  what_you_see: |
    In Legion Prof's timeline view, gaps ("bubbles") appear between tasks
    on application processors. The critical path analysis does not explain
    these gaps — the critical path line either skips over them or the gaps
    appear on processors/operations that are not on the critical path at
    all. The bubbles could be caused by pending data movement operations
    that the profiler does not fully surface, or by genuine critical path
    dependencies. Without application-specific context, the two causes are
    visually indistinguishable.

  key_metrics: |
    - Idle gaps on application processors between task executions
    - Critical path analysis does not account for / explain the gaps
    - Data movement (copy/fill) operations may or may not be visible
      in the timeline during the gap period
    - [INCOMPLETE — needs review] No specific threshold or metric
      provided to quantify this pattern

  distinguishing_features: |
    Unlike the structural coverage gap (critical_path_structural_coverage_gap),
    this pattern involves gaps WITHIN the portion of the profile that
    critical path analysis should cover (application tasks and data
    movement). The documentation explicitly states that "Legion Prof does
    not show data movement operations and therefore bubbles can be caused
    either by critical path dependencies, or by pending data movement
    operations. We currently rely on application specific information to
    discern the cause." This is a gap in causal modeling even within the
    DAG-covered operation types.

root_cause: |
  Even though copy and fill operations participate in the critical path
  DAG, the profiler's visualization may not fully surface all data
  movement operations. When a task is waiting for data to arrive (a
  pending copy or fill), the resulting idle time appears as a bubble
  in the timeline. The critical path DAG may not capture the full
  causal chain through these data movements, leaving the gap unexplained.
  The document states this requires "application specific information to
  discern the cause," indicating the profiler alone cannot distinguish
  data-movement-induced bubbles from other dependency-induced gaps.

gotchas:
  - "Users may assume all timeline gaps are explained by the critical path analysis — but even within the DAG's coverage area, data movement causality is incompletely modeled."
  - "This is distinct from the ~50% structural gap: those are operations that are architecturally excluded. This pattern concerns operations that SHOULD be in the DAG but whose causal relationships are not fully captured."
  - "The document suggests this is a known limitation requiring application-specific reasoning, not a profiler bug to be fixed with flags."

fix:
  primary: |
    [INCOMPLETE — needs review] The document does not provide a specific
    fix. It states: "We currently rely on application specific information
    to discern the cause." Users must use their knowledge of the
    application's data flow to determine whether bubbles are caused by
    pending data movement or other dependencies.

  alternatives: |
    - Cross-reference timeline gaps with copy/fill operations visible on
      channel processors or DMA processors to see if data movement
      overlaps with the gap period.
    - Use Legion Spy (-lg:spy) data if available, which may provide
      additional dependency information beyond what the profiler's
      critical path DAG captures.
    - Examine physical instance placement and data movement patterns
      in the application to reason about whether remote copies could
      explain observed bubbles.

  what_not_to_do: |
    Do NOT assume all bubbles are critical path issues — some may be
    data movement latency. Do NOT assume the critical path analysis
    fully explains all gaps even within the application task layer.

verification: |
  [INCOMPLETE — needs review] No specific verification method provided
  in the document. Verification requires application-specific reasoning
  about expected data movement patterns.

real_cases:
  - case: "[No specific case cited]"
    app: "[N/A]"
    scale: "[N/A]"
    result: "[N/A]"
    key_detail: "Pattern described only via documentation quote. No real diagnosed case provided."

related_patterns:
  - "critical_path_structural_coverage_gap"

---

## Summary
- Total records extracted: 4
- High confidence: 0 (no real diagnosed cases with before/after verification)
- Medium confidence: 3 (documented patterns with profiler signatures or specific error codes: critical_path_structural_coverage_gap, prof_flag_verbosity_misconception, critical_path_dynamic_collectives_missing_flag)
- Low confidence: 1 (mentioned in passing with insufficient detail for full diagnosis: critical_path_data_movement_bubbles)
- Gaps identified:
  - **No real diagnosed cases**: The document is a research/explainer piece, not a case study. No specific applications, node counts, or quantitative before/after results are cited for any pattern.
  - **Dynamic collectives silent degradation**: It is unclear whether missing -lg:prof_all_critical_arrivals always triggers fatal error 2020 or can sometimes silently degrade coverage without an error. The document hedges: "If your application uses dynamic collectives and you're seeing ~50% coverage, this missing flag could be a significant contributing factor."
  - **Data movement bubble mechanism**: The document quotes a known limitation but provides no profiler signature, metric, or fix for distinguishing data-movement-induced bubbles from other dependency-induced gaps.
  - **Exact DAG edge construction**: The document repeatedly notes that examining state.rs source code is necessary to understand exactly which operation types enter the petgraph DAG and how edges are constructed — this information is not available from public documentation.
  - **Version-specific behavior**: Critical path was added between December 2022 and December 2024 with no specific release tag. Behavior may differ across versions in this window but the document cannot pinpoint which release introduced what.
  - **Rendering vs. computation gaps**: The "Improve rendering of critical paths" near-term priority suggests some coverage issues may be rendering bugs (data exists but isn't displayed) rather than computation gaps (data doesn't exist). The document cannot distinguish these.


## Source: Healthy Baselines

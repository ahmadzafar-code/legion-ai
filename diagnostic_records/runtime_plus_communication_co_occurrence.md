id: runtime_plus_communication_co_occurrence
title: Runtime overhead and communication co-occurring — fix order matters
source: low_processor_utilization_diagnosis.md, Co-occurrence section
confidence: medium
user_type: all

symptoms:
  what_you_see: |
    Utility processors show high saturation with analysis tasks
    (Category 1 signature) AND channels show high utilization with
    copy-gap correlation (Category 2 signature) in the same profile.
    Both sets of symptoms are present simultaneously.

  key_metrics: |
    - Utility saturation >0.8 with dominant "Logical Dependence Analysis"
    - Channel utilization >0.7 with temporal correlation to app gaps
    - Both Category 1 and Category 2 scoring >0.3 in the prioritization algorithm

  distinguishing_features: |
    This is not a single root cause but a co-occurrence. The key insight
    is that runtime overhead (no tracing/replication) causes suboptimal
    mapping decisions which in turn cause unnecessary communication.
    Fixing runtime overhead often resolves both simultaneously.

root_cause: |
  Without memoization (-dm:memoize) and control replication
  (-dm:replicate), the runtime spends excessive time on fresh analysis
  AND the resulting mapping decisions may be suboptimal, causing
  unnecessary communication. This is the most common co-occurrence
  pattern. Tracing reuses validated mapping decisions, eliminating
  both the analysis overhead and the suboptimal mapping.

gotchas:
  - "Fix runtime overhead FIRST — enabling -dm:memoize and -dm:replicate often resolves both simultaneously because traced execution reuses validated mapping decisions."
  - "If you fix communication first (e.g., restructure partitions) without fixing runtime overhead, you waste effort because the suboptimal mapping will be re-analyzed from scratch each iteration anyway."

fix:
  primary: |
    Enable -dm:memoize and -dm:replicate first. Re-profile to see if
    communication issues resolve as a side effect of traced execution
    reusing validated mapping decisions.

  alternatives: |
    - If communication issues persist after enabling tracing, proceed
      with Category 2 fixes (sharding, instance reuse, partition
      restructuring).

  what_not_to_do: |
    Do NOT fix communication patterns first without addressing runtime
    overhead — the suboptimal mapping decisions that cause the
    communication will be recomputed each iteration anyway without
    tracing.

verification: |
  After enabling -dm:memoize and -dm:replicate:
  1. Utility processor saturation should drop (Category 1 resolved).
  2. Check if channel utilization also dropped (communication may
     self-resolve with better mapping from traces).
  3. If channels remain congested, proceed with Category 2 diagnosis.

real_cases:
  - case: "[INCOMPLETE — needs review]"
    app: "[INCOMPLETE — needs review]"
    scale: "[INCOMPLETE — needs review]"
    result: "[INCOMPLETE — needs review]"
    key_detail: "Document identifies this as the most common co-occurrence pairing"

related_patterns:
  - "runtime_overhead_no_tracing"
  - "communication_blocking_localized"
  - "communication_blocking_systemic"

---

## Summary
- Total records extracted: 11
- High confidence: 5 (real diagnosed cases with verification)
  - runtime_overhead_no_tracing (SC 2018, SC 2017/PPoPP 2021)
  - small_tasks_below_metg (Task Bench SC 2020)
  - communication_blocking_localized (GitHub issue #1640)
  - insufficient_parallelism_dependency_serialization (GitHub issue #1203)
  - memory_pressure_instance_churn (GitHub issue #1739)
- Medium confidence: 6 (documented patterns with profiler signatures)
  - runtime_overhead_with_tracing
  - communication_blocking_systemic
  - insufficient_parallelism_too_few_tasks
  - insufficient_parallelism_mapper_serialization
  - memory_pressure_with_communication
  - runtime_plus_communication_co_occurrence
- Low confidence: 0
- Gaps identified:
  - The DuckDB schema is described as "inferred" and column names should be verified against actual `legion_prof duckdb` export (added v25.06.0, July 2025)
  - The "truly-in-use" memory line feature for distinguishing valid from invalid instances is referenced as requested but not implemented
  - No specific case studies provided for: systemic network saturation, too-few-tasks parallelism, mapper serialization, or memory+communication co-occurrence
  - Critical path analysis (requires `-lg:spy` data) is mentioned as useful but no DuckDB queries are provided for it
  - The prioritization scoring algorithm (Cat1–Cat4 formulas) references a `gap_correlation_score` for Category 2 that is not defined or queryable
  - Warning 1122 ("Detected unbounded pool in trace") is mentioned but no profiler signature or diagnosis query is provided
  - The document references 91 entries in the `LgTaskID` enum but only discusses ~15 specific meta-task titles


## Source: Pitfalls

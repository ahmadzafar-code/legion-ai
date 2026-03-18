id: healthy_channel_activity_overlapping_execution
title: Channel copy activity during task execution is healthy overlap, not congestion
source: gpu-differentialdiagnosis.md (Causes 5 vs 8 distinguishing); low_processor_utilization_diagnosis.md (Category 2 temporal correlation)
confidence: medium
user_type: all

symptoms:
  what_you_see: |
    Channel rows show copy operations running concurrently with task execution
    on GPU/CPU rows. The copies and tasks overlap temporally — copies are
    happening WHILE processors are busy, not INSTEAD of processors being busy.
    No GPU/CPU gaps correspond to the copy activity.

  key_metrics: |
    - Channel utilization moderate (10-50%)
    - Temporal OVERLAP between copies and application task execution
    - NO temporal correlation between channel activity and application processor gaps
    - Application processor utilization remains high during copy activity

  distinguishing_features: |
    The critical distinction from network congestion (Cause 5) is TEMPORAL
    CORRELATION. Congestion: channels busy DURING application processor gaps
    (copies blocking execution). Healthy overlap: channels busy WHILE
    application processors are ALSO busy (copies running concurrently with
    computation). The low utilization diagnosis framework specifically states:
    "Temporal correlation is critical: channels busy DURING app gaps =
    communication on critical path. Channels busy WHILE apps are also busy =
    healthy overlap."

root_cause: |
  This is not a problem. Legion pipelines data movement with computation.
  The runtime pre-stages data for upcoming tasks while current tasks execute.
  Seeing copies and computation running simultaneously means the pipeline is
  working as designed — data arrives before it's needed, so processors never
  stall waiting for it.

gotchas:
  - "Copy concurrency in the profiler is imprecise — Realm reports copies as concurrent but they may run sequentially on the DMA engine. Don't estimate aggregate bandwidth from apparent concurrency."
  - "If channel utilization is moderate but growing with node count, it may become congestion at higher scale even if it's healthy overlap now."
  - "Multi-hop copies (hop count > 1) may appear as healthy overlap at the aggregate level but be individually suboptimal."

fix:
  primary: |
    No fix needed. Overlapping copies with computation is the desired behavior.

  alternatives: |
    N/A — healthy behavior.

  what_not_to_do: |
    Do NOT diagnose all channel activity as network congestion.
    Do NOT recommend reducing copy volume when copies are overlapping
    with computation rather than blocking it.
    Do NOT use copy concurrency numbers for bandwidth analysis (profiler
    imprecision).

verification: |
  Healthy: copies during task execution with no corresponding gaps.
  Unhealthy: copies during processor idle gaps (copies on critical path).

real_cases: []

related_patterns:
  - "network_congestion"
  - "communication_blocking_localized"
  - "copy_profiling_imprecision"

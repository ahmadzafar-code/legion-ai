id: memory_pressure_with_communication
title: Memory pressure stalls inter-node copy buffer allocation
source: low_processor_utilization_diagnosis.md, Co-occurrence section (Communication + Memory pressure)
confidence: medium
user_type: all

symptoms:
  what_you_see: |
    Channels show high utilization (appears similar to Category 2
    communication blocking), BUT copies exhibit unusual start delays —
    large ready→start gaps on copy operations that are not explained
    by channel contention alone. Memory timelines show near-capacity
    occupancy. Utility processors show a mix of allocation meta-tasks
    and are not dominated by analysis tasks.

  key_metrics: |
    - High channel utilization (Q2.1)
    - Large ready→start gaps on copies (visible in copy timing details)
    - Memory above 85% peak occupancy (Q4.1)
    - "Malloc Instance" significant on utility processors (Q4.3)

  distinguishing_features: |
    Unlike pure Category 2 (communication), copies have large ready→start
    gaps — they are waiting for buffer allocation, not bandwidth. Unlike
    pure Category 4 (memory pressure), channels ARE busy — this is a
    feedback loop where memory pressure stalls buffer allocation which
    stalls copies. The combination of high channel utilization + high
    memory occupancy + copy start delays is the signature.

root_cause: |
  At scale, inter-node copies require intermediate buffers. When memory
  is tight, buffer allocation stalls copies, creating a feedback loop:
  memory pressure → slow buffer allocation → slow copies → more data
  in flight → more memory pressure. The intermediate buffers are
  controlled by -ll:ib_rsize.

gotchas:
  - "This looks like a communication problem but the root cause is memory — fixing communication patterns alone won't help."
  - "The feedback loop means both symptoms appear simultaneously, making diagnosis ambiguous."

fix:
  primary: |
    Increase -ll:ib_rsize (intermediate buffer memory for multi-hop
    copies; default 0; typical values 512m–4096m) AND -ll:fsize
    (GPU framebuffer) BEFORE tuning communication patterns.

  alternatives: |
    - After resolving memory pressure, re-evaluate whether channel
      congestion remains as a separate Category 2 issue.

  what_not_to_do: |
    Do NOT attempt to fix this as purely a communication problem by
    restructuring partitions or fixing sharding — the underlying cause
    is memory pressure on intermediate buffers. Fix memory first.

verification: |
  After increasing -ll:ib_rsize and -ll:fsize:
  1. Copy ready→start gaps should shrink significantly.
  2. If channel utilization drops, the problem was purely memory.
  3. If channel utilization remains high but copy start delays are gone,
     a separate Category 2 communication issue may remain.

real_cases:
  - case: "[INCOMPLETE — needs review]"
    app: "[INCOMPLETE — needs review]"
    scale: "[INCOMPLETE — needs review]"
    result: "[INCOMPLETE — needs review]"
    key_detail: "Document describes pattern in co-occurrence section but provides no specific case"

related_patterns:
  - "communication_blocking_systemic"
  - "memory_pressure_instance_churn"

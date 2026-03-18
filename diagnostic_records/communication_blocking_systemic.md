id: communication_blocking_systemic
title: Systemic network saturation blocking execution across all channels
source: low_processor_utilization_diagnosis.md, Category 2
confidence: medium
user_type: all

symptoms:
  what_you_see: |
    All inter-node channels show high utilization simultaneously.
    Application processor gaps appear system-wide and correlate
    temporally with channel activity across the board. Unlike localized
    congestion, no single channel stands out — all are uniformly busy.

  key_metrics: |
    - Most/all inter-node channels above 0.7 utilization (Q2.1)
    - Q2.2 confirms blocking copies across many channels
    - Q2.4 shows relatively symmetric but uniformly high inter-node volumes
    - Legion Prof long-latency message warning may fire

  distinguishing_features: |
    Unlike localized congestion, ALL inter-node channels are saturated
    simultaneously, not just a few. This indicates the aggregate
    bandwidth demand exceeds the interconnect capacity — a fundamentally
    different problem from a mapping bug.

root_cause: |
  The application's communication pattern inherently requires more
  aggregate bandwidth than the interconnect provides. This can occur
  with all-to-all communication patterns, dense ghost region exchanges
  in stencil codes, or applications that simply move too much data
  relative to compute.

gotchas:
  - "Systemic saturation may be inherent to the algorithm and not fully fixable without algorithmic restructuring."
  - "Check Q2.4 carefully: if volumes are highly asymmetric even with all channels busy, the root cause may still be a mapping bug (localized pattern) that cascades across the network."
  - "Communication + Memory pressure co-occurrence: if memory is tight, buffer allocation stalls copies, creating a feedback loop visible as high channel utilization with unusual copy start delays (large ready→start gaps on copies). Fix -ll:ib_rsize and -ll:fsize first."

fix:
  primary: |
    Restructure partitions to minimize ghost region surface area.
    Use image partitions for stencil patterns to reduce communication
    volume. Algorithmic changes to reduce the communication-to-compute
    ratio.

  alternatives: |
    - -ll:bgwork N: Increase background work threads (2–4) to maximize
      copy throughput.
    - -ll:ib_rsize N: Increase intermediate buffer memory (512m–4096m)
      to prevent buffer allocation from stalling copies.
    - Overlap communication with computation by ensuring the task graph
      has independent work to execute while copies are in flight.

  what_not_to_do: |
    Do NOT assume a mapper bug if Q2.4 shows symmetric, uniformly high
    volumes — this is likely algorithmic. Do NOT expect sharding fixes
    to help when the aggregate demand genuinely exceeds capacity.

verification: |
  After restructuring partitions or increasing buffer sizes:
  1. Q2.1 channel utilization should decrease.
  2. Application processor gaps correlated with copies should shrink.
  3. Q2.4 inter-node volumes should decrease if partitioning was changed.

real_cases:
  - case: "[INCOMPLETE — needs review]"
    app: "[INCOMPLETE — needs review]"
    scale: "[INCOMPLETE — needs review]"
    result: "[INCOMPLETE — needs review]"
    key_detail: "Document mentions systemic saturation as a pattern but provides no specific case study"

related_patterns:
  - "communication_blocking_localized"
  - "memory_pressure_with_communication"

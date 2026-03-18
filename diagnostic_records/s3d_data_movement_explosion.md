id: s3d_data_movement_explosion
title: S3D over-transferring all fields when only a few needed
source: SC 2014 paper (Bauer, Treichler, Slaughter, Aiken) — Structure Slicing; Case 11
confidence: high
user_type: legion_cpp

symptoms:
  what_you_see: |
    S3D combustion simulation profiler timelines show communication time
    dominating compute time. Excessive data movement volume far exceeds
    what tasks actually need. Tasks that only need a handful of chemical
    species fields trigger transfer of entire region instances containing
    all species data.

  key_metrics: |
    Data movement volume per task vs. actual data requirements: orders of
    magnitude higher than needed. Bandwidth utilization vs. theoretical
    peak. Communication-to-computation time ratio in profiler timeline:
    communication dominates.

  distinguishing_features: |
    Unlike network congestion (Case 16) where the data volume is correct
    but the network is saturated, here the data volume itself is wrong —
    far more data is being moved than needed. The excess is proportional
    to the number of unused fields (thousands of chemical species in S3D).

root_cause: |
  The original Legion data model did not distinguish between individual
  fields within regions. When a task required even one field from a
  region, all fields in that region's physical instance were moved. For
  S3D with thousands of chemical species fields, this caused orders-of-
  magnitude data over-transfer.

gotchas:
  - "This is a data-model limitation in pre-2014 Legion, not a user error. If you're on a modern Legion version, structure slicing is built in."
  - "The problem scales with the number of fields in the region — applications with few fields per region won't see this."

fix:
  primary: |
    Structure slicing — incorporating fields as first-class elements in
    the logical region data model. This enables Legion to automatically
    infer task parallelism from field non-interference, decouple data
    usage specification from physical layout, and transfer only the
    fields actually needed by each task. (Built into Legion since the
    SC 2014 paper.)

  alternatives: |
    Manually split regions into per-field or per-field-group regions to
    avoid over-transfer. This is the workaround for pre-structure-slicing
    Legion versions.

  what_not_to_do: |
    Do NOT attempt to fix this by optimizing the network or adding
    bandwidth. The problem is that the wrong amount of data is being
    sent, not that the network is too slow.

verification: |
  S3D scaled to 8,192 nodes on Titan (then #2 supercomputer). Achieved
  3.68× speedup over vectorized CPU-only Fortran and 1.88× over hand-
  tuned OpenACC code. First truly large-scale Legion deployment (up from
  16 nodes max in SC 2012).

real_cases:
  - case: "SC 2014 paper"
    app: "S3D (combustion simulation with thousands of chemical species)"
    scale: "8,192 nodes on Titan"
    result: "3.68× speedup over Fortran; 1.88× over OpenACC; scaled from 16 to 8,192 nodes"
    key_detail: "Thousands of chemical species fields made the over-transfer orders of magnitude worse than typical"

related_patterns:
  - circuit_missing_tracing_network

id: healthy_multi_node_balanced
title: Healthy multi-node profile — distributed execution with control replication active
source: legionconcepts.md (scaling behavior table); Legion Runtime anti-patterns reference (control replication section)
confidence: medium
user_type: all

symptoms:
  what_you_see: |
    Across all nodes: utility processor activity is distributed evenly (not
    concentrated on node 0). All nodes show similar GPU/CPU utilization levels.
    Channel rows show inter-node copy activity but at moderate levels that
    don't dominate execution. The profiler does NOT show one node saturated
    while others are idle. Shard tasks are visible (control replication
    active).

  key_metrics: |
    - Utility utilization SIMILAR across all nodes (±10%)
    - Application processor utilization SIMILAR across all nodes (±10%)
    - No single node at 100% utility while others are idle
    - Shard tasks visible in profiler (control replication active)
    - Channel utilization moderate and not scaling super-linearly with node count
    - No long-latency message warnings

  distinguishing_features: |
    The key signature of healthy multi-node execution is SYMMETRY: all nodes
    look roughly the same. Unhealthy multi-node execution shows asymmetry —
    one node saturated (missing control replication) or one channel path
    congested (mapper placement bug) or all channels saturated (network
    congestion). Note: timing skew across nodes can make visual alignment
    misleading — focus on per-node utilization metrics, not cross-node
    visual alignment.

root_cause: |
  This is not a problem. Healthy distributed execution with control
  replication distributing analysis work and a good sharding function
  distributing computation.

gotchas:
  - "Timing skew across nodes can make healthy profiles look misaligned — check skew warnings before diagnosing cross-node timing issues."
  - "Some load imbalance (±10% across nodes) is normal and acceptable. Only flag imbalance when one node is consistently 2×+ busier than others."
  - "Channel activity that's moderate now may become congestion at higher node counts. If the user is scaling up, note the current channel utilization as a baseline."

fix:
  primary: |
    No fix needed. If further scaling is desired, note the current channel
    utilization and utility utilization as baselines to watch for degradation
    at higher node counts.

  alternatives: |
    N/A — healthy behavior.

  what_not_to_do: |
    Do NOT diagnose timing skew artifacts as performance problems.
    Do NOT diagnose normal ±10% load imbalance as a sharding issue.
    Do NOT diagnose moderate channel activity as network congestion when
    it's not blocking execution.

verification: |
  Consistent behavior as node count increases: utilization remains high,
  no single node becomes a bottleneck, channel activity scales sub-linearly.

real_cases:
  - case: "PPoPP 2021 DCR paper"
    app: "PENNANT, HTR, Soleil-X"
    scale: "256-1024 GPUs"
    result: "99% parallel efficiency at 1,024 nodes"
    key_detail: "This is what a well-scaled multi-node profile looks like"

related_patterns:
  - "control_replication_scalability_wall"
  - "no_control_replication"
  - "bad_sharding_function"
  - "network_congestion"

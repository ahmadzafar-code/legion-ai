id: timing_skew_cross_node
title: Clock Timing Skew Across Nodes Distorts Cross-Node Comparisons
source: "017 - Michael Bauer, Section 4: Timing Skew Guidance; 016 - Elliott Slaughter"
confidence: high
user_type: all

symptoms:
  what_you_see: |
    Tasks or copies on different nodes appear temporally misaligned —
    a task that should causally precede another appears to start after
    it, or gaps between cross-node operations look larger or smaller
    than expected. Legion Prof's skew detection may flag warnings.

  key_metrics: |
    - Timing skew ~100 μs: "you can probably just mostly ignore them."
    - Timing skew ~1 ms: "you should really start to pay attention."
    - Timing skew 10–100 ms: observed in some machines; severely
      distorts cross-node comparison.

  distinguishing_features: |
    Unlike real scheduling delays, timing skew affects the VISUAL
    POSITION of items on different nodes but not their actual execution.
    Causal anomalies (effect appears before cause across nodes) are the
    telltale sign. Legion Prof now has built-in skew detection. This
    should not be confused with actual performance problems.

root_cause: |
  Different nodes have different clock offsets. Legion Prof logs
  timestamps from each node's local clock. Without perfect
  synchronization, the positions of items on different nodes are
  shifted relative to each other. The skew varies by machine and
  can range from microseconds to hundreds of milliseconds.

gotchas:
  - "Skew means 'you can't necessarily compare the position of boxes on different nodes, because effectively they might be shifted in the order that they actually ran in.'"
  - "Do not diagnose cross-node 'gaps' or 'overlaps' as performance problems until timing skew has been accounted for."
  - "Skew detection is built into the profiler now — check for skew warnings before doing any cross-node timing analysis."

fix:
  primary: |
    Check Legion Prof's skew detection warnings. If skew is >1 ms:
    - Do not rely on cross-node visual alignment for diagnosis.
    - Focus on single-node analysis or use critical path (which accounts
      for causal ordering) instead of visual timeline comparison.
    - Investigate the machine's clock synchronization (NTP, PTP).

  alternatives: |
    - For skew <100 μs, it can generally be ignored.
    - Use critical path analysis (which follows causal chains, not
      timestamps) for cross-node diagnosis.

  what_not_to_do: |
    Do NOT diagnose cross-node timing issues (gaps, overlaps, ordering
    anomalies) without first checking skew magnitude. Do NOT attempt
    manual timestamp correction — use Legion Prof's built-in handling.

verification: |
  After improving clock synchronization, skew warnings should show
  smaller values. Critical path analysis should give consistent
  results regardless of skew.

real_cases:
  - case: "Talk 017 - Bauer general guidance"
    app: "[general — multiple machines]"
    scale: "Multi-node"
    result: "Observed skews of 10–100 ms on some machines"
    key_detail: "Severity ranges from ignorable (100 μs) to seriously misleading (100 ms)"

related_patterns: []

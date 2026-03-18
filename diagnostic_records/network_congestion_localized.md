id: network_congestion_localized
title: Localized channel congestion blocking execution on specific paths
source: low_processor_utilization_diagnosis.md, Category 2; GitHub issue #1640; GPU differential diagnosis guide, Cause 5; Legion issue #1640 (Modified Circuit on Perlmutter); GitHub StanfordLegion/legion#1640; Case 16
confidence: high
user_type: all

symptoms:
  what_you_see: |
    Expanding channel rows in Legion Prof reveals one or a few specific
    channels (memory pairs, e.g., node 0 GPU FB → node 1 GPU FB) at
    near-100% utilization with dense copy operation bars, while other
    channels are idle or lightly used. Gaps on application processor
    timelines align temporally with active copy operations on the
    congested channels. Other application processors may be running
    tasks concurrently on unaffected data. Utility processors may be
    idle — they have already dispatched the work; the bottleneck is
    data movement, not analysis. The gap duration may increase with
    node count.

  key_metrics: |
    - One or few channels above 0.7 utilization (Q2.1) while most are low
    - Q2.2 confirms copies temporally overlap with application processor gaps
    - Q2.4 shows highly asymmetric inter-node communication volumes
    - Blocking copy count (Q2.2) is significant
    - Copy operations between computation tasks grew from ~2ms at 8 nodes to
      ~16ms at 32 nodes (in issue #1640)
    - Automatic profiler warning: "A significant number of long latency messages
      were detected... 3865 messages >1000μs (7.77% of total), longest 35524μs"
      (issue #1640)
    - Channel utilization doubling from 16 to 32 nodes
    - Per-channel vs. aggregate utilization comparison needed to distinguish
      localized hotspot from systemic bandwidth saturation

  distinguishing_features: |
    Unlike systemic network saturation (communication_blocking_systemic), only a
    small number of channels are congested — per-channel utilization is high on
    specific paths while most channels are idle. Unlike Category 1 (runtime
    overhead), utility processors are NOT saturated — they have dispatched the
    work but data movement is the bottleneck. Unlike Category 4 (memory
    pressure), channels ARE busy — the bottleneck is transfer, not allocation.
    Unlike mapper wrong placement (Cause 8 / unnecessary copies), the copies in
    localized congestion may appear necessary but are caused by poor sharding
    that routes too much traffic through one interconnect path. To distinguish
    from systemic congestion: check whether a small number of channels carry
    disproportionate load (localized) vs. all channels are saturated (systemic).
    The `-C` flag (copy matrix analysis) identifies dominant memory-pair traffic
    patterns.

root_cause: |
  A mapper sharding or partitioning problem is placing too much data
  movement on one interconnect path. The mapper's sharding function
  is not correctly mapping tasks to the node that already owns their
  data, causing unnecessary cross-node copies. In GitHub issue #1640,
  a custom mapper was "spraying tasks randomly across the machine"
  because it couldn't handle multi-level partitioning hierarchies.
  At scale, this localized overload worsens: in issue #1640, channel
  utilization doubled from 16 to 32 nodes and copy operations grew
  from ~2ms at 8 nodes to ~16ms at 32 nodes. The GPU is blocked
  waiting for remote input data before it can execute the next task.

gotchas:
  - "Localized congestion is almost always a mapper/sharding bug, not a hardware limitation. Do NOT recommend hardware changes."
  - "Temporal correlation is critical: channels busy DURING app gaps = communication on critical path. Channels busy WHILE apps are also busy = healthy overlap."
  - "The long-latency message warning in Legion Prof ('A significant number of long latency messages were detected...') can confirm network congestion but does not distinguish localized from systemic."
  - "Individual per-channel utilization may appear low (~10%) — you must check AGGREGATE utilization across all channels to see whether the bottleneck is localized or systemic."
  - "Issue #1640 had three co-occurring causes — network congestion was only one; fixing it alone was not sufficient."
  - "The automatic --message-threshold/--message-percentage warning is a strong signal but may not trigger at moderate scale."
  - "Localized congestion (Cause 5) and bad mapper placement (Cause 8) can co-occur: bad mapper placement can create unnecessary copies that worsen genuine network congestion."
  - "Per-channel vs. aggregate utilization comparison is needed to distinguish localized hotspot congestion from systemic bandwidth saturation."
  - "The -dm:memoize flag alone is NOT sufficient with the DefaultMapper — a mapper configuration flag is also required. This is a common trap that can mask co-occurring issues."
  - "The network congestion in issue #1640 was identified as a separate issue requiring deeper investigation into network topology-aware mapping."

fix:
  primary: |
    Fix the mapper's sharding function to map tasks to the node that
    already owns their data, eliminating unnecessary cross-node copies.
    Ensure the sharding function correctly handles multi-level
    partitioning hierarchies. Run the profiler's copy matrix analysis
    (`-C` flag) to identify dominant memory-pair traffic patterns and
    target those specific communication paths.

  alternatives: |
    - Configure mapper instance reuse: the map_task callback should
      prefer existing physical instances with valid data over creating
      fresh instances that require copies.
    - Restructure partitions to minimize ghost region surface area;
      use image partitions for stencil patterns.
    - -ll:bgwork N: Increase background work threads from default 1
      to 2–4 to parallelize copy operations (e.g., -ll:bgwork 3
      -ll:bgworkpin 1).
    - -ll:ib_rsize N: Increase intermediate buffer memory for multi-hop
      copies (default 0; typical 512m–4096m).
    - Tune -ll:amsg for active message handling threads.
    - Use overlap of computation and communication (double-buffering)
      to hide remaining transfer latency.
    - Reduce the ghost zone width or overlap region if applicable.
    - Consider algorithmic changes that reduce communication (e.g.,
      local subiterations between global exchanges).

  what_not_to_do: |
    Do NOT increase network bandwidth or assume a hardware problem
    when Q2.4 shows asymmetric volumes — the issue is software mapping.
    Do NOT confuse temporal correlation (channels busy during gaps)
    with temporal coincidence (channels busy at the same time as tasks).
    Do NOT assume all channel activity is congestion — check whether
    the copies are actually necessary before trying to reduce them.
    Unnecessary copies indicate bad mapper placement (Cause 8), which
    has a different fix. Do NOT conflate tracing fixes with network
    fixes if both problems are present — they are independent.

verification: |
  After fixing the sharding function:
  1. Q2.4 should show roughly symmetric inter-node volumes.
  2. The previously congested channel should drop well below 0.7 utilization.
  3. Application processor gaps that correlated with copies should disappear.
  4. Overall execution time should decrease.
  5. Copy durations should shrink at the same node count.
  6. The automatic `--message-threshold` warning should disappear or report
     fewer long-latency messages.

real_cases:
  - case: "GitHub issue #1640"
    app: "Application with multi-level partitioning hierarchies"
    scale: "Multi-node (exact count not specified)"
    result: "Identified custom mapper spraying tasks randomly as root cause"
    key_detail: "3,865 messages exceeding 1,000μs (7.77% of total), max latency 35,524μs — confirmed by long-latency message warning"
  - case: "Legion issue #1640"
    app: "Modified Circuit benchmark"
    scale: "32 nodes on Perlmutter"
    result: "Part of multi-cause fix (one of three co-occurring causes)"
    key_detail: "Profiler auto-warning: 3865 messages >1000μs (7.77% of total), longest 35524μs; copy times grew from ~2ms at 8 nodes to ~16ms at 32 nodes"
  - case: "GitHub legion#1640"
    app: "Modified Circuit benchmark"
    scale: "16–32 nodes"
    result: "Tracing enabled after config fix; network issue remained open"
    key_detail: "DefaultMapper requires both -dm:memoize AND mapper config flag — just one is insufficient"

related_patterns:
  - "communication_blocking_systemic"
  - "mapper_wrong_placement"
  - "missing_tracing"
  - "dynamic_tracing_missing"
  - "automatic_tracing_apophenia"


  ```

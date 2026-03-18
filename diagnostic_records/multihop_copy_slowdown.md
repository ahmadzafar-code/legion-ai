id: multihop_copy_slowdown
title: Multi-Hop Copies Causing Unexpected Data Movement Latency
source: "017 - Michael Bauer (live demo), Section 4; 008 - Jonathan Graham (FleCSI)"
confidence: high
user_type: all

symptoms:
  what_you_see: |
    Copy operations with unusually long durations visible in the timeline.
    When clicking on a slow copy, the "number of hops" parameter shows
    a value greater than 1. In the FleCSI case, gather copies appeared
    as green squares in the profile with the highlighted copy showing
    "three hops" — data going through host memory.

  key_metrics: |
    - Copy hop count > 1 (Legion Prof now tracks this).
    - Copy duration disproportionately long relative to data size.
    - In the FleCSI case: 10x slowdown going from 1 GPU to 2 GPUs.

  distinguishing_features: |
    Multi-hop copies are identifiable by the hop count annotation in
    Legion Prof. This is different from slow copies caused by bandwidth
    saturation (which would have hop count = 1 but high channel
    utilization) or from Realm scheduling delays (which would show high
    delayed duration). The key indicator is "data has to move more than
    once between two memories."

root_cause: |
  When a direct copy path between two memories doesn't exist (e.g.,
  GPU-to-GPU without NVLink, or sparse gather copies that must go
  through host memory), Realm routes the data through intermediate
  memories. Each hop adds latency and consumes bandwidth on intermediate
  interconnects. In the FleCSI case, sparse gather copies between GPUs
  were forced through host memory as 3-hop copies.

gotchas:
  - "The FleCSI team reported that 'all their instances are where they're supposed to be' — correct instance placement does NOT guarantee direct copy paths. The copy routing depends on memory affinity and available DMA channels, not just instance location."
  - "Debugging this was 'quite tricky' according to the FleCSI team, and they also saw anomalous profiler output that may have been a profiler bug."
  - "Copy profiling has a known imprecision: Realm makes concurrent copies APPEAR simultaneous, but they likely execute sequentially on the DMA engine. Apparent high copy parallelism may mask sequential multi-hop overhead."

fix:
  primary: |
    Examine the copy paths and ensure direct memory channels exist:
    1. Check instance placement — ensure source and destination are in
       memories with direct copy paths.
    2. For GPU-to-GPU: verify NVLink/NVSwitch connectivity.
    3. For sparse/gather copies: these inherently may require host staging;
       consider restructuring data layouts to enable dense copies.

  alternatives: |
    - Use Legion Prof's hop-count annotation to identify the worst
      offenders and prioritize fixing those copy paths.
    - Future Legion Prof versions may annotate the actual hop path
      ("we've been trying to explore ways of annotating what the path
      is, where the hops are").
    - Request bandwidth annotations when available (feature requested
      in Q&A: effective bandwidth + red warning when below expected).

  what_not_to_do: |
    Do NOT assume that because instances are correctly placed, copies
    will be efficient. Instance placement and copy path are related but
    not identical concerns. Do NOT assume copy concurrency shown in the
    profiler is real — Realm's copy profiling has known imprecision
    about concurrency.

verification: |
  After fixing copy paths, the hop count for affected copies should
  drop to 1. Copy durations should decrease correspondingly. The 
  scaling anomaly (e.g., 10x slowdown at 2 GPUs) should be resolved.

real_cases:
  - case: "Talk 008 - FleCSI GPU debugging"
    app: "FleCSI (Los Alamos)"
    scale: "1 to 2 GPUs"
    result: "Identified 10x slowdown caused by sparse gather copies taking 3 hops through host memory"
    key_detail: "All instances were reportedly in correct locations — the issue was the copy PATH, not the instance placement. Also encountered possible profiler bugs."

related_patterns:
  - "copy_profiling_imprecision"

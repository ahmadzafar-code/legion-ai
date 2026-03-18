id: copy_profiling_imprecision
title: Realm Copy Profiling Shows False Concurrency
source: "017 - Michael Bauer, Section 1: Limitations Called Out"
confidence: medium
user_type: all

symptoms:
  what_you_see: |
    Multiple copies in the channel/copy rows appear to be running
    simultaneously — overlapping time extents in the timeline. This
    creates an impression of high copy parallelism that may not reflect
    reality.

  key_metrics: |
    - Multiple concurrent copies visible on the same channel.
    - Apparent channel utilization may look higher than actual.
    - Individual copy durations may appear shorter than wall-clock
      sequential execution would suggest.

  distinguishing_features: |
    This is a PROFILER ARTIFACT, not a performance problem per se.
    It can cause misdiagnosis: you might think copies are efficiently
    parallelized when they're actually sequential on the DMA engine.
    It differs from a real performance issue because the application
    behavior is correct — only the profiler's representation is
    misleading.

root_cause: |
  Realm records the copy start time as when the event precondition
  triggers, not when the DMA engine actually begins the transfer.
  Since multiple copies can have their preconditions triggered
  nearly simultaneously, they all appear to "start" at the same time.
  In reality, copies likely execute sequentially on the DMA engine.

gotchas:
  - "Do not use apparent copy parallelism in Legion Prof to estimate aggregate bandwidth — the copies are likely serialized."
  - "This can mask multi-hop copy overhead: if three sequential hops appear concurrent, the wall-clock time looks shorter than reality."
  - "A complementary profiler (Nsight Systems) will show actual DMA engine activity and reveal the true serialization."

fix:
  primary: |
    This is a known limitation, not a bug to fix. Be aware of it when
    interpreting copy behavior:
    - Use Nsight Systems for accurate copy timing.
    - Future Realm profiling improvements (targeted early 2025 per
      Talk 004 - Artem Priakhin) aim to improve copy profiling fidelity.

  alternatives: |
    - Use the Nsight integration with Legate (Talk 005) to see actual
      kernel and copy activity alongside Legion Prof's semantic view.
    - Compute effective bandwidth manually and compare to expected
      peak to detect serialization.

  what_not_to_do: |
    Do NOT rely on Legion Prof's copy timeline for bandwidth analysis.
    Do NOT assume high apparent copy concurrency means copies are
    actually running in parallel.

verification: |
  Run Nsight Systems simultaneously and compare copy timelines. If
  copies that appear concurrent in Legion Prof are sequential in
  Nsight, the imprecision is confirmed.

real_cases: []

related_patterns:
  - "multihop_copy_slowdown"

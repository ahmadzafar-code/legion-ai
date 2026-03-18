id: explicit_sync_calls
title: Explicit cudaDeviceSynchronize calls create regular inter-task gaps
source: GPU differential diagnosis guide, Cause 6; Legion issue #440; Jax-on-Realm presentation (December 2024 Legion retreat)
confidence: medium
user_type: all

symptoms:
  what_you_see: |
    Small, regular gaps between every consecutive GPU task — a "picket fence"
    pattern. During [T1, T2], no other processor is particularly busy: no utility
    spikes, no channel activity, no CPU computation. The gap is the host-side
    latency from detecting task completion to launching the next kernel. If an
    application explicitly calls `cudaDeviceSynchronize()` within a task, the gap
    will be larger and appear INSIDE a task's execution bar (the task bar shows a
    waiting period within its running time).

  key_metrics: |
    - Gap size typically sub-millisecond (between tasks)
    - Gaps are regular and uniform between every consecutive GPU task
    - No concurrent activity on utility, channel, or CPU processors during gap
    - Inter-task gap > 100 microseconds (threshold used in DuckDB detection query)

  distinguishing_features: |
    Unlike Cause 2 (thread oversubscription), gaps are small and regular, NOT
    node-correlated with utility spikes. Unlike Cause 7 (insufficient parallelism),
    gaps occur between EVERY single task, not irregularly between waves. The
    "picket fence" uniformity is the key visual signature. When sync occurs WITHIN
    a task, the task bar itself shows an internal waiting period — this is unique
    to Cause 6.

root_cause: |
  With CUDA hijack active, `cudaDeviceSynchronize()` maps to
  `cudaStreamSynchronize()` on the task's stream. Even this scoped
  synchronization creates a gap because the GPU must complete all pending work
  on that stream before the host can proceed. The Jax-on-Realm analysis notes
  that "tasks are not marked done until all data effects on the GPU are visible,"
  meaning each task boundary inherently involves a synchronization point that
  creates a small bubble. Explicit `cudaDeviceSynchronize()` calls within task
  code amplify this overhead.

gotchas:
  - "CUDA hijack maps cudaDeviceSynchronize to stream-scoped cudaStreamSynchronize — this is better than device-wide sync but still creates gaps"
  - "-cuda:legacysync 1 is the fix for Cause 2 (stream interference) but can INCREASE synchronization overhead for Cause 6 — do not confuse these"
  - "In newer Legion versions (≥ v24.06.0), asynchronous CUDA task launch via cuCtxRecordEvent reduces this overhead"
  - "Every task boundary inherently involves a small sync point — only fix if gaps are significantly above the baseline"

fix:
  primary: |
    Ensure CUDA hijack is active (so `cudaDeviceSynchronize` maps to stream-
    scoped sync rather than device-wide sync). Remove any explicit
    `cudaDeviceSynchronize()` calls from task code. For Realm-level
    optimization, use the newer asynchronous task launch mechanism available
    in Legion ≥ v24.06.0 (uses `cuCtxRecordEvent`).

  alternatives: |
    Increase task granularity (longer-running tasks) so that the relative
    overhead of inter-task sync is smaller. Fuse small consecutive tasks into
    larger ones where possible.

  what_not_to_do: |
    Do NOT use `-cuda:legacysync 1` to fix this pattern — it forces single-stream
    behavior which can increase synchronization overhead. `-cuda:legacysync` is
    for Cause 2 (stream interference), not for Cause 6.

verification: |
  After removing explicit `cudaDeviceSynchronize` calls and/or upgrading to
  asynchronous task launch: the "picket fence" gaps should narrow significantly.
  Inter-task gaps should approach the minimum achievable baseline for task launch
  overhead. Measure the mean inter-task gap before and after.

real_cases:
  - case: "Legion issue #440"
    app: "Soleil-X (Regent CUDA codegen)"
    scale: "[Same case as Cause 1 — scalar reductions]"
    result: "Linked to the 2–3× improvement in scalar reduction performance"
    key_detail: "cudaDeviceSynchronize was called as part of the reduction postamble; the explicit sync is one component of the Cause 1 pattern"
  - case: "Jax-on-Realm presentation"
    app: "Jax-on-Realm"
    scale: "December 2024 Legion retreat analysis"
    result: "[No specific quantitative improvement cited]"
    key_detail: "Noted that task completion inherently involves a sync point — 'tasks are not marked done until all data effects on the GPU are visible'"

related_patterns:
  - "scalar_reduction_blocking"
  - "thread_oversubscription_stream_interference"

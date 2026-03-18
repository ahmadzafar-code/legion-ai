id: thread_oversubscription_stream_interference
title: Thread oversubscription and CUDA stream interference cause synchronized GPU gaps
source: GPU differential diagnosis guide, Cause 2; Legion issue #1203 (DG-Legion on Summit); GitHub StanfordLegion/legion#1203; Case 5
confidence: high
user_type: legion_cpp

symptoms:
  what_you_see: |
    Legion Prof shows regularly-spaced gaps in GPU timelines across ALL ranks on
    the same node simultaneously. During each gap, ALL utility processors show
    simultaneous utilization spikes. One rank consistently has an unusually long
    task box on its GPU row, and the resulting delay propagates to other ranks
    through synchronization dependencies. `Post-Task Execution` entries take
    O(10ms). On utility processor rows, `Replay Physical Trace` meta-tasks take
    several milliseconds. Light green boxes appear in the utility processor
    timeline with no hover metadata, indicating delayed/queued meta-tasks.

  key_metrics: |
    - All GPUs on the same node gap simultaneously (node-correlated gaps)
    - Utility processor utilization spikes during [T1, T2] across all ranks
    - `Post-Task Execution` duration: O(10ms)
    - `Replay Physical Trace` duration: several milliseconds
    - Thread count exceeds hardware thread count (e.g., 14 threads on 7 physical cores)
    - Task IDs being dispatched thousands ahead of executing IDs (healthy — confirming this is NOT dependence analysis overhead)
    - No blocking-future warnings from `-lg:warn`

  distinguishing_features: |
    Node-correlated synchronized gaps are UNIQUE to this cause — no other pattern
    produces simultaneous gaps across all GPUs on the same node. Unlike missing
    tracing, `Replay Physical Trace` tasks DO exist but take too long (due to
    oversubscription), and the runtime IS dispatching tasks far ahead (healthy
    task-ID gap). Unlike insufficient parallelism, utility processors are spiking
    rather than idle. The gaps are PERIODIC and SIMULTANEOUS across all GPUs — not
    task-specific like scalar reduction gaps. `-lg:warn` shows no blocking-future
    warnings, ruling out accidental future blocking. The diagnostic flag
    `-ll:show_rsrv` reveals thread-to-core mapping and confirms oversubscription.

root_cause: |
  Two compounding problems: (1) Thread oversubscription — more threads than
  physical cores. In the issue #1203 case (Power9 SMT1, 7 cores per rank),
  the system created 14 threads but had only 7 physical cores. Three pinned
  background workers, three pinned OMP threads, and utility processors competed
  for cores, leaving only 1 core for 2 utility processors. (2) CUDA stream
  interference — Realm submits GPU operations (device-to-device copies) on a
  different CUDA stream from application kernels (unlike MPI+Kokkos which uses
  only the default stream), causing stream interference that disrupts GPU
  execution. Both problems must be resolved together.

gotchas:
  - "Neither fix alone suffices — issue #1203 showed both -cuda:legacysync AND thread count adjustment are required together"
  - "On Power9, the SMT mode matters: SMT1 gives 7 cores, SMT4 gives 28 — the same -ll:cpu/-ll:util values can oversubscribe in SMT1 but work fine in SMT4"
  - "Do NOT apply -cuda:legacysync if gaps are NOT node-correlated — it forces single-stream behavior and hurts performance when stream interference is not the cause"
  - "The unusually long task box on one rank propagates delays to other ranks via synchronization dependencies — the root cause may appear to be on a different rank than where the longest box appears"
  - "The initial hypothesis (blocking futures) was ruled out by `-lg:warn` producing no warnings. Always check this before assuming thread contention."
  - "The light green boxes with no hover metadata on utility processors are a clue — they indicate meta-tasks that are delayed/queued."

fix:
  primary: |
    Both changes are required (neither alone suffices, per issue #1203):
    1. `-cuda:legacysync 1` — forces all CUDA operations onto a single stream,
       eliminating stream interference between Realm's internal copies and
       application kernels
    2. Increase hardware thread availability — on Power9, switch from SMT1 to
       SMT4 to provide enough hardware threads for all software threads; on x86,
       ensure `-ll:cpu` + `-ll:util` + `-ll:bgwork` + OMP threads ≤ available
       hardware threads

  alternatives: |
    Check thread-to-core mapping with `-ll:show_rsrv` first to quantify the
    oversubscription. Reduce `-ll:util` or `-ll:bgwork` counts if increasing
    hardware threads isn't possible. Reduce the number of pinned
    background/OpenMP threads to free cores for utility processors. Alternatively,
    use a node configuration with more physical cores per GPU.

  what_not_to_do: |
    Do NOT apply `-cuda:legacysync 1` unless ALL GPUs on the same node gap
    simultaneously — it forces single-stream behavior and hurts performance if
    multi-stream interference is not the actual cause. Do NOT just fix one of
    the two problems and assume the other doesn't matter. Do NOT assume blocking
    futures without checking `-lg:warn` first.

verification: |
  After applying both fixes: synchronized node-correlated GPU gaps should
  disappear. `Post-Task Execution` entries should drop from O(10ms) to
  sub-millisecond. `Replay Physical Trace` tasks should execute in microseconds
  rather than milliseconds. Run `-ll:show_rsrv` to confirm thread count ≤
  hardware thread count. Utility processor spikes should be eliminated or
  greatly reduced. Expected result: strong scaling close to optimal.

real_cases:
  - case: "Legion issue #1203"
    app: "DG-Legion (discontinuous Galerkin solver)"
    scale: "ORNL Summit, Power9 SMT1, 7 cores per rank"
    result: "Eliminated synchronized GPU gaps (quantitative improvement not specified)"
    key_detail: "14 threads on 7 physical cores; 3 pinned background workers + 3 pinned OMP threads + utility processors all competing for cores; both thread count fix AND -cuda:legacysync were required"
  - case: "GitHub legion#1203"
    app: "DG-Legion (Discontinuous Galerkin solver)"
    scale: "2 nodes, 6 GPUs/node (12 total) on Summit"
    result: "Strong scaling close to optimal up to 8 nodes"
    key_detail: "Both smt4 AND -cuda:legacysync required — either alone insufficient"

related_patterns:
  - "missing_tracing"
  - "explicit_sync_calls"
  - "dynamic_tracing_missing"
  - "soleil_x_scalar_reduction_gpu_gaps"


  ```

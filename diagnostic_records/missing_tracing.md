id: missing_tracing
title: Missing tracing causes per-iteration overhead that grows with scale (~1 ms per-task reduced to ~100 μs with tracing)
source: GPU differential diagnosis guide, Cause 3; Legion issue #1640 (Modified Circuit benchmark); Apophenia (ASPLOS 2025); SC 2018 paper (Lee, Slaughter, Bauer et al.); Legion profiling docs; Case 4; Tracing and memoization section; Dependency analysis section; Anti-pattern reference table; low_processor_utilization_diagnosis.md, Category 1; "017 - Michael Bauer (live demo), Section 4: Performance Debugging Methodology"
confidence: high
user_type: all

symptoms:
  what_you_see: |
    Gaps/bubbles appear on GPU or CPU application processor timelines.
    Expanding utility processor rows reveals dense, back-to-back meta-task
    boxes dominated by "Logical Dependence Analysis" (enum ID 8),
    "Trigger Task Mapping", "Trigger Operation Mapping", "Prepipeline Stage",
    and "Scheduler" — rather than trace-replay meta-tasks. The critical
    visual diagnostic is the ABSENCE of "Replay Physical Trace" (enum ID 46)
    light-green boxes on utility processors. Every iteration looks like the
    first iteration — full mapper call sequences and dependence analysis
    repeat. The overall visual impression is "utility rows solid, everything
    else empty." At high node counts, gaps grow because the runtime cannot
    stay far ahead of execution — the task IDs being mapped are close to the
    IDs being executed (instead of being hundreds ahead). At 32 nodes in
    issue #1640, gaps appeared where "nothing seems to be happening."

  key_metrics: |
    - trace_replay_count = 0 (no "Replay Physical Trace" entries anywhere in
      the profile)
    - Utility processor saturation >0.8 (often near 100% across all nodes)
    - "Logical Dependence Analysis" total time >2× "Replay Physical Trace"
      total time (Q1.3 vs Q1.4)
    - High volume of mapper calls (map_task, select_task_options, slice_task)
      on utility processors during [T1, T2]
    - Run-ahead distance below 50 operation IDs consistently (Q1.5); gap
      between mapped task IDs and executing task IDs ≈ 0 (should be 10s–100s)
    - Per-task overhead ~1 ms (untraced METG threshold)
    - Average task duration near or below ~1 ms
    - Overhead scales with iteration count and node count
    - 4.9× average slower strong scaling without tracing (up to 7.0×)
    - Strong scaling wall: performance stops improving as problem size per
      node decreases; METG degrading with scale
    - Channel utilization doubling from 16 to 32 nodes (in issue #1640)
    - Utility processor utilization remains high in steady state rather than
      dropping after iteration 1

  distinguishing_features: |
    Unlike thread oversubscription / stream interference (Case 5), "Replay
    Physical Trace" tasks do NOT exist at all — they are completely absent
    from the profile, not just slow. The presence vs. absence of Replay
    Physical Trace is the single most important differentiator: utility
    saturation WITH Replay Physical Trace tasks is thread oversubscription;
    utility saturation WITHOUT them is missing tracing. Unlike control
    replication bottleneck (Case 3), ALL utility processors across all nodes
    are saturated, not just node 0. Unlike blocking Python / idle utility
    (Case 4), utility processors are BUSY with dependence analysis and
    mapper calls during the gap, not idle. Unlike insufficient parallelism
    (Category 3 / Case 7), the runtime is actively working (analyzing/
    mapping), it's just too slow to keep ahead. Unlike memory pressure
    (Category 4), the dominant meta-tasks are dependence analysis and
    mapping, NOT "Malloc Instance" / "Free Instance". Unlike mapper-
    dominated utility saturation (long_mapper_calls), the busy work is
    dependence analysis, not map_task / select_task_options calls. The
    task-ID gap metric (mapped vs. executing) is the key quantitative
    differentiator. The key iteration-level diagnostic: compare utility
    processor activity in iteration 1 vs. iteration 2+. With tracing,
    iteration 2+ should show minimal utility activity and Replay Physical
    Trace tasks. Without tracing, every iteration looks like iteration 1.

root_cause: |
  Legion operates as an out-of-order task processor with a pipeline:
  application phase (task launch) → analysis phase (dependence analysis +
  mapping on utility processors) → execution phase (application processors).
  Without tracing/memoization enabled, every task in every iteration
  requires fresh dynamic dependence analysis costing ~1 ms per task. For
  iterative applications (the vast majority of scientific computing), the
  same dependence pattern repeats every iteration — re-analyzing identical
  dependencies is pure waste. The run-ahead distance (gap between op_ids
  being analyzed vs. executed) collapses to near zero, meaning every task
  execution must wait for its analysis to complete. The per-task analysis
  latency is directly exposed as idle time on application processors.
  Tracing (begin_trace/end_trace) records the task graph once and replays
  it on subsequent iterations, eliminating per-iteration overhead and
  reducing per-task cost from ~1 ms to ~100 μs (10×). In issue #1640, the
  Modified Circuit benchmark lacked the -dm:memoize flag required for the
  DefaultMapper to enable tracing — the per-iteration overhead grew with
  problem size and node count, causing scaling failure beyond 16 nodes. As
  of Legion v25.09.0, the Apophenia automatic tracing system (ASPLOS 2025)
  discovers traces dynamically without manual annotations at ~5 μs per task
  launch overhead. The -lg:window flag (default 1024) acts as the reorder
  buffer; -lg:sched (default 1) controls scheduling pause threshold;
  -lg:width (default 4) sets operations per scheduling pass.

gotchas:
  - "The task-ID gap metric (mapped vs. executing) is the single most important diagnostic: if they're close, the runtime is the bottleneck."
  - "Do NOT confuse utility saturation from dependence analysis (no Replay Physical Trace tasks) with utility saturation from trace replay (Replay Physical Trace tasks present). The latter is a different problem (thread oversubscription, Case 5)."
  - "The presence or absence of tracing 'significantly changes what you see in profiles' — always check for Replay Physical Trace tasks before concluding the runtime is inherently too slow."
  - "METG threshold shifts with tracing: ~1 ms untraced, ~100 μs traced. A profile showing 500 μs tasks looks fine if tracing is enabled but terrible if it is not."
  - "As of ASPLOS 2025 (Apophenia), automatic tracing activates by default — but -dm:memoize must still be passed for the DefaultMapper to cooperate. Users on 25.03.0+ may assume tracing is automatic and skip -dm:memoize, leaving the DefaultMapper uncooperative."
  - "Before Legion 25.03.0, the DefaultMapper requires BOTH -dm:memoize AND a mapper configuration flag — just one is not enough (see Case 16)."
  - "The -dm:memoize flag is only needed for the DefaultMapper (pre-v25.03.0 behavior) — custom mappers need begin_trace/end_trace support and memoization enabled in select_task_options instead."
  - "For custom mappers: memoization must be enabled in select_task_options — the DefaultMapper flag does not apply."
  - "Traces cannot be nested. Error 474 fires if behavior changes between trace record and replay."
  - "Replication inside physical traces is not supported (warning 1117)."
  - "Issue #1640 had THREE co-occurring causes (missing tracing + bad mapper placement + network congestion) — fixing tracing alone only partially resolved the problem."
  - "For applications with non-repeating task graphs, tracing won't help — focus on reducing per-task overhead instead."
  - "Automatic tracing can be disabled with -lg:no_auto_tracing — check this flag isn't set if you expect auto-tracing."
  - "Utility saturation of 50–80% indicates runtime overhead is a contributing factor alongside another category — not sole cause. Do not dismiss it if below 80%."
  - "Do not confuse this with mapper overhead on utility processors. Check whether the busy utility work is dependence analysis vs. mapper calls (Legion Prof profiles all mapper calls separately)."

fix:
  primary: |
    Add -dm:memoize to the command line when using the DefaultMapper. This
    enables dynamic tracing, reducing per-task overhead from ~1 ms to ~100 μs
    by replaying memoized dependence analysis. Improves strong scaling
    4.9–7.0× on average (SC 2018). For the DefaultMapper on pre-25.03.0,
    ensure both -dm:memoize AND the mapper configuration flag are set. For
    custom mappers, enable memoization in select_task_options and implement
    begin_trace/end_trace support. For Legion >= 25.03.0: verify Automatic
    Traces (Apophenia) is enabled (on by default); -dm:memoize must still be
    passed for the DefaultMapper to cooperate. Manual trace annotations
    (runtime->begin_trace() / runtime->end_trace()) are no longer required
    but take priority when present. Automatic tracing achieves 0.92×–1.03×
    of manually traced performance and 0.91×–2.82× speedups over untraced
    versions at ~5 μs per task launch overhead.

  alternatives: |
    - -dm:replicate: Enables dynamic control replication, distributing
      dependence analysis across all nodes instead of serializing on one
      controller. Achieves 99% parallel efficiency at 1,024 nodes
      (SC 2017, PPoPP 2021). Essential complement to -dm:memoize at scale.
    - -ll:util N: Increase utility processors from default 1 to 2–4. Each
      additional utility processor dedicates a hardware core to runtime
      analysis. Low-cost for GPU-centric workloads with spare CPU cores.
    - -lg:window N: Increase run-ahead window beyond default 1024 (try
      2048–8192) for applications with very long task chains.
    - -lg:filter N: Trim physical instance user lists (default 0/disabled,
      useful range 128–4096) to reduce dependence analysis cost for programs
      with many concurrent instances.
    - Wrap iterative loops with begin_trace(ctx, trace_id)/end_trace(ctx,
      trace_id) using consistent trace IDs for explicit manual trace
      annotation.
    - FlexFlow approach: aggressive task fusion (merge many small operations
      into a single Legion task) combined with explicit tracing of the fused
      pattern.
    - If tracing is not applicable (non-repeating task graphs or complex
      dynamic control flow), reduce per-task overhead by coarsening tasks,
      reducing the total number of tasks per iteration, or using task fusion
      (Case 9).
    - Disable automatic tracing with -lg:no_auto_tracing only if it causes
      correctness issues with non-idempotent operations.

  what_not_to_do: |
    Do NOT coarsen task granularity as a first resort — tracing reduces
    per-task overhead from ~1 ms to ~100 μs (10×), which is far more
    effective than reducing task count. Do NOT increase -ll:util as the
    sole fix without first enabling -dm:memoize — more utility processors
    performing fresh analysis helps marginally, but tracing provides a 10×
    reduction in per-task overhead. Do NOT assume adding more utility
    processors will fix this — the bottleneck is serial dependence analysis
    throughput, not parallelism of utility work. Do NOT confuse this with a
    parallelism issue (Category 3) and attempt to restructure the task graph
    when the bottleneck is purely runtime analysis overhead. Do NOT assume
    the runtime is idle just because gaps appear — check utility processors
    for mapper call activity before concluding "nothing is happening." Do
    NOT add -lg:no_auto_tracing without a specific reason — it disables the
    Apophenia system that may be needed. Do NOT ignore the mapper
    configuration flag when using -dm:memoize on the DefaultMapper
    (pre-25.03.0). Do NOT disable automatic tracing (-lg:no_auto_tracing)
    except for debugging. Do NOT modify behavior between trace record and
    replay iterations (triggers error 474). Do NOT nest traces. Do NOT
    ignore this pattern assuming "the runtime will catch up" — it indicates
    a fundamental analysis-bound execution.

verification: |
  After enabling tracing (-dm:memoize or automatic), re-profile and check:
  1. "Replay Physical Trace" tasks should appear on utility processors in
     iterations 2+ and dominate utility processor time.
  2. "Logical Dependence Analysis" should drop to near-zero in steady state.
  3. Utility processor utilization should drop to <20% in steady state
     (mapper calls replaced by trace replay).
  4. Run-ahead distance (Q1.5) should increase to 50+ operation IDs; the
     mapped-to-executing task ID gap should widen to 10s–100s.
  5. Gaps/bubbles on application processor timelines should shrink or
     disappear; CPU/GPU utilization should rise correspondingly.
  6. The overhead gap between iterations should shrink — iteration 2+ should
     look dramatically different from iteration 1.
  7. Per-task overhead drops from ~1 ms to ~100 μs.
  8. Strong scaling improved by average 4.9× and up to 7.0× across
     optimized benchmarks (SC 2018).
  9. Automatic tracing should achieve 0.92–1.03× of manual tracing
     performance (ASPLOS 2025).

real_cases:
  - case: "Legion issue #1640"
    app: "Modified Circuit benchmark"
    scale: "Failed to weak-scale beyond 16 nodes (tested up to 32 nodes on Perlmutter)"
    result: "Part of multi-cause fix — tracing alone was not sufficient due to co-occurring Causes 5 and 8"
    key_detail: "Code lacked -dm:memoize; three co-occurring causes required sequential diagnosis"
  - case: "SC 2018 (tracing paper)"
    app: "Suite of Legion benchmarks (HTR, Soleil-X, etc.)"
    scale: "Strong scaling across multiple node counts"
    result: "Average 4.9× and up to 7.0× improvement across already-optimized benchmarks"
    key_detail: "Per-task overhead reduced from ~1 ms to ~100 μs (10× reduction); single flag (-dm:memoize) produced the largest per-flag performance gain in Legion tuning"
  - case: "ASPLOS 2025 Apophenia (automatic tracing)"
    app: "[multiple benchmarks]"
    scale: "[not specified]"
    result: "0.92–1.03× of manual tracing performance; 0.91–2.82× speedups over untraced"
    key_detail: "Automatic detection of repeated sequences without programmer annotations at ~5 μs per task launch overhead"
  - case: "SC 2017, PPoPP 2021 (control replication)"
    app: "Distributed Legion applications"
    scale: "1,024 nodes"
    result: "99% parallel efficiency"
    key_detail: "-dm:replicate distributes dependence analysis; essential complement to -dm:memoize at scale"
  - case: "Talk 017 live demo (Michael Bauer)"
    app: "[unnamed application in live demo]"
    scale: "[not specified]"
    result: "Demonstrated as the canonical example of a runtime-limited profile"
    key_detail: "Bauer explicitly pointed out the absence of tracing as the explanation"
  - case: "Talk 014 - FlexFlow/LLM"
    app: "FlexFlow (LLM serving)"
    scale: "[not specified]"
    result: "Applied tracing to entire repeated inference pattern to minimize Legion overhead"
    key_detail: "Combined task fusion with tracing — entire small-model computation merged into single task"

related_patterns:
  - "thread_oversubscription_stream_interference"
  - "network_congestion"
  - "mapper_wrong_placement"
  - "control_replication_bottleneck"
  - "circuit_missing_tracing_network"
  - "automatic_tracing_apophenia"
  - "graph_compilation_metg"
  - "individual_task_launches"
  - "low_utility_processors"
  - "trace_behavior_change"
  - "no_control_replication"
  - "runtime_overhead_with_tracing"
  - "insufficient_parallelism_dependency_serialization"
  - "small_tasks_below_metg"
  - "long_mapper_calls"
  - "low_deferred_duration"
  - "tracing_memory_tension"

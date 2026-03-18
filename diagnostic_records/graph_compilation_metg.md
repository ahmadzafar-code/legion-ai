id: graph_compilation_metg
title: Graph compilation reduced METG by 3.3–7.1× and rescued PENNANT GPU scaling
source: Preprint (Yadav, Guman, Treichler et al., 2025) — arXiv:2508.16522; Case 14
confidence: high
user_type: all

symptoms:
  what_you_see: |
    Standard task-based system has METG(50%) values between 83–173 μs.
    PENNANT with standard Legion "falls over at 8 GPUs" — performance
    degrades instead of improving beyond that point. Multiple compounding
    overhead sources visible: task launch latency, dependence analysis
    cost, copy scheduling, runtime meta-task scheduling.

  key_metrics: |
    METG(50%): 83–173 μs for standard Legion/Realm/StarPU. Pointer
    query overhead: ~20 μs per task. PENNANT scaling: performance
    degrades beyond 8 GPUs. Standard Legion stencil scaling: stops at
    16 GPUs.

  distinguishing_features: |
    Unlike missing tracing (Case 4) which addresses dependence analysis
    alone, graph compilation targets the AGGREGATE of all per-task
    overheads: pointer queries, copy scheduling, launch latency, and
    meta-task scheduling. Each is individually modest (~20 μs) but they
    compound multiplicatively. This is the "last mile" optimization after
    tracing and CR are already in place.

root_cause: |
  The aggregate per-task overhead of ~20 μs for pointer queries alone,
  plus copy scheduling and launch latency, creates a floor that prevents
  scaling for fine-grained tasks. Each overhead source is individually
  modest but they compound multiplicatively. Tracing alone reduces
  dependence analysis but doesn't address the other overhead components.

gotchas:
  - "Graph compilation builds on top of dynamic tracing — you need tracing working first before graph compilation can help."
  - "The H100 GPU METG(50%) of 39 μs is approaching MPI's 22 μs but hasn't closed the gap entirely."
  - "This addresses the 'fall over' behavior where adding more GPUs hurts — if you see performance degrade with more GPUs beyond a threshold, graph compilation may help."

fix:
  primary: |
    Enable graph compilation, which combines dynamic tracing with
    pre-planned copy optimization and reduced task launch overhead.
    Specific optimizations target each overhead component identified
    through METG decomposition.

  alternatives: |
    Coarsen task granularity to reduce the number of tasks (but this
    sacrifices parallelism). Use task fusion (Case 9) for Legate workloads.

  what_not_to_do: |
    Do NOT expect graph compilation to help if tracing is not already
    active — it builds on top of tracing. Do NOT apply this to workloads
    where task granularity is already coarse (>1 ms tasks) — the
    overhead floor only matters for fine-grained tasks.

verification: |
  Legion METG improved by 3.3–7.1× (H100 GPU METG(50%) = 39 μs,
  approaching MPI's 22 μs). Realm improved by 1.7–5.3×. PENNANT with
  "Legion Opt" delivers continuous scaling up to 32 GPUs, 5.0×
  improvement over standard Legion. Stencil: 3.4× improvement at 32
  GPUs where standard Legion stopped scaling at 16 GPUs.

real_cases:
  - case: "arXiv:2508.16522 (2025 preprint)"
    app: "PENNANT, stencil benchmarks"
    scale: "Up to 32 H100 GPUs"
    result: "3.3–7.1× METG improvement; PENNANT 5.0× over standard Legion at 32 GPUs"
    key_detail: "PENNANT 'fell over at 8 GPUs' with standard Legion but scaled continuously to 32 with graph compilation"

related_patterns:
  - dynamic_tracing_missing
  - realm_small_copy_overhead
  - legate_task_fusion

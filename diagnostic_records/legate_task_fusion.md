id: legate_task_fusion
title: Per-task meta-overhead reduced by task fusion in Legate
source: PR nv-legate/legate#113; PR nv-legate/cupynumeric#150; Case 9
confidence: high
user_type: legate

symptoms:
  what_you_see: |
    Profiling shows that task-level overhead (mapper calls, scheduling,
    dependence analysis) dominates wall-clock time for workloads with
    many small fuseable operations. Inter-task gaps are large relative to
    task execution time, especially for element-wise operations like
    Black-Scholes and stencil codes.

  key_metrics: |
    Ratio of meta-overhead time to task execution time: high (overhead
    dominates). Number of tasks launched per second: below theoretical
    throughput. Gap analysis between consecutive tasks on the same
    processor: gaps exceed task durations for small element-wise ops.

  distinguishing_features: |
    Unlike missing tracing (Case 4), the overhead here is per-operation
    in Legate's translation layer, not in Legion's dependence analysis.
    The tasks ARE being traced, but there are simply too many of them.
    The workloads are element-wise operations where multiple adjacent ops
    could be combined. Unlike Case 15 (Diffuse), this is intra-library
    fusion, not cross-library fusion.

root_cause: |
  Each Legate operation (unary, binary, ternary NumPy ops) becomes a
  separate Legion task, each requiring its own mapping, scheduling, and
  metadata processing. For compute-light element-wise operations, the
  O(N) meta-overhead for N operations dominates actual computation.

gotchas:
  - "Conjugate Gradient showed minimal improvement (~1.0–1.05×) because it was dominated by unfuseable O(n²) operations — not all workloads benefit."
  - "Constant optimization (creating scalars in-place instead of via separate convert tasks) enables larger fusion windows — both optimizations are needed together."
  - "Fusion constraints: only fusable op types, no aliased read-after-write, same launch shape, same projection functors."

fix:
  primary: |
    Enable task fusion in Legate: a buffered window of tasks is scanned
    for sub-windows that can be aggregated into a single launch. Also
    enable constant optimization — scalar constants are created in-place
    instead of through separate `convert` tasks, enabling larger fusion
    windows. (PR nv-legate/legate#113 and nv-legate/cupynumeric#150.)

  alternatives: |
    Manually restructure application code to use bulk operations instead
    of many small element-wise calls. Use graph compilation (Case 14) for
    even more aggressive optimization.

  what_not_to_do: |
    Do NOT expect uniform speedups across all workloads. Operations
    dominated by unfuseable work (e.g., matrix operations in Conjugate
    Gradient) will see minimal benefit.

verification: |
  Measured speedups on 1–16 GPUs: Black-Scholes ~1.6–2×, 27-point
  stencil ~1.6–2×, Logistic Regression ~1.15–2×, Jacobi ~1.15×.
  Conjugate Gradient ~1.0–1.05× (expected minimal due to unfuseable ops).

real_cases:
  - case: "PR legate#113 / cupynumeric#150"
    app: "Black-Scholes, stencil, Logistic Regression, Jacobi, Conjugate Gradient"
    scale: "1–16 GPUs"
    result: "Black-Scholes/stencil ~1.6–2×; Logistic Regression ~1.15–2×"
    key_detail: "Conjugate Gradient barely benefited (~1.0–1.05×) due to unfuseable O(n²) ops"

related_patterns:
  - dynamic_tracing_missing
  - diffuse_cross_library_fusion
  - graph_compilation_metg

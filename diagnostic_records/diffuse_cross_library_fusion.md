id: diffuse_cross_library_fusion
title: Cross-library task and kernel fusion via Diffuse
source: ASPLOS 2025 paper (Yadav, Sundram, Lee et al.); Case 15
confidence: high
user_type: legate

symptoms:
  what_you_see: |
    cuPyNumeric and Legate Sparse applications show many small tasks that
    individually underutilize GPU resources. Separate kernel launches
    prevent data reuse in GPU caches. Per-task runtime overhead visible
    as gaps between kernel executions. Task boundaries imposed by library
    modularity prevent optimization across function and library
    boundaries.

  key_metrics: |
    Per-task GPU utilization: low (underutilized warps). Kernel launch
    overhead visible in gaps between executions. Weak scaling on up to
    128 A100 GPUs (8 GPUs/node, NVLink/NVSwitch, InfiniBand). 12 runs
    per experiment, dropping fastest/slowest, averaging remaining 10.

  distinguishing_features: |
    Unlike intra-library task fusion (Case 9), this addresses CROSS-
    LIBRARY boundaries (e.g., cuPyNumeric + Legate Sparse). Unlike graph
    compilation (Case 14), this includes JIT kernel compilation (MLIR-
    based) to optimize fused task bodies, not just scheduling overhead.

root_cause: |
  Fine-grained tasks from composing independent libraries (cuPyNumeric
  for dense, Legate Sparse for sparse operations) create excessive kernel
  launch overhead and under-utilized GPU warps. Task boundaries imposed
  by library modularity prevent optimization across function and library
  boundaries.

gotchas:
  - "Diffuse uses dynamic dependence analysis on a scale-free task-based IR — it's a fundamentally different approach from static fusion."
  - "The MLIR-based JIT compilation is essential for the performance gains — fusion without kernel optimization leaves the kernel launch overhead."
  - "1.86× is the geometric mean; individual applications may see more or less benefit depending on fusion opportunities."

fix:
  primary: |
    Use Diffuse — dynamic dependence analysis on a scale-free task-based
    IR, fusing tasks across function and library boundaries. Paired with
    JIT compilation (MLIR-based) for kernel optimization of the fused
    task bodies.

  alternatives: |
    Manually restructure code to avoid cross-library composition, keeping
    all operations within a single library. This sacrifices programmability.

  what_not_to_do: |
    Do NOT expect intra-library fusion (Case 9) to capture cross-library
    optimization opportunities. The library boundary is a hard barrier
    for traditional fusion approaches.

verification: |
  1.86× average speedup (geometric mean) over unmodified cuPyNumeric/
  Legate Sparse applications on up to 128 GPUs. Matched or exceeded
  hand-optimized MPI-based PETSc (1.4× geometric mean speedup over
  PETSc).

real_cases:
  - case: "ASPLOS 2025 paper"
    app: "cuPyNumeric + Legate Sparse composed applications"
    scale: "Up to 128 A100 GPUs (8 GPUs/node)"
    result: "1.86× geomean speedup; 1.4× geomean over hand-optimized PETSc"
    key_detail: "Cross-library fusion + MLIR JIT compilation — not achievable by intra-library approaches"

related_patterns:
  - legate_task_fusion
  - graph_compilation_metg

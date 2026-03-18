id: control_replication_missing
title: Missing or disabled Control Replication — single-node bottleneck limits scaling beyond ~16–32 nodes
source: SC 2017 paper; PPoPP 2021 paper; GitHub StanfordLegion/legion#338; Legion 24.03.0 release notes; Case 3; Control replication and distributed sharding section; Anti-pattern reference table; Transcript 020 (Control Replication Part 5); 006 - Jeremy Wilke (Legate Jax Performance Investigation)
confidence: high
user_type: all

symptoms:
  what_you_see: |
    Legion Prof shows most processors sitting idle while the top-level
    task on node 0 is saturated launching tasks. Utility processor on the
    control node is at 100% utilization. Worker nodes show growing idle
    time proportional to node count. A single node is visibly the
    bottleneck — all dependence analysis, mapping, and task management is
    concentrated there. No shard tasks are visible in the profiler.
    Distributed scaling flatlines beyond ~16–32 nodes.

  key_metrics: |
    Parallel efficiency drops sharply beyond 16–64 nodes. Utility
    processor utilization on control node = 100%. METG increases with
    node count. Execution time scales super-linearly with node count
    (instead of remaining constant for weak scaling). One node active,
    others idle. No shard tasks visible in the profiler. Scaling
    efficiency degrades as node count increases. At extreme scale,
    many small control messages may be visible in communication channels.

  distinguishing_features: |
    Unlike missing tracing (Case 4), the bottleneck is on a SINGLE node
    (node 0), not distributed across all utility processors. The control
    node is saturated while worker nodes are idle — a classic Amdahl's
    Law signature. Distinguished from network congestion (Case 16) because
    channel utilization is low; the bottleneck is in task dispatch, not
    communication. Unlike a bad sharding functor (where replication IS
    enabled but work is concentrated on one shard), here there are no
    shards at all — the replicate flag is simply not set. Unlike
    data-movement bottlenecks, the messages are control messages, not data.

root_cause: |
  Legion's implicitly parallel programming model relied on a single
  top-level task running on one node to serially launch all sub-tasks and
  perform all dependence analysis. This created a classic Amdahl's Law
  bottleneck — the serial fraction grew relative to total parallel work
  as node count increased. Without Dynamic Control Replication (DCR),
  dependence analysis, distribution, and mapping all happen on one node.
  The SOOP pipeline on the control node cannot keep up with the aggregate
  execution rate of many worker nodes. Additionally, the replicate flag
  in select_task_options is false by default — the runtime does not
  automatically infer when replication would be beneficial, so if the
  mapper does not explicitly opt in, control replication does not occur.

gotchas:
  - "This is the fundamental scalability limitation of Legion's original design — described by Slaughter as 'quite literally the optimization that saved Legion.' It took seven years to develop Control Replication."
  - "Users commonly expect control replication to happen automatically — it does NOT. The replicate flag is per-task, set in select_task_options, and is false by default."
  - "Static CR (SC 2017 in Regent) and Dynamic CR (PPoPP 2021 in the runtime) are different implementations — the runtime version is more general."
  - "Even with CR, you still need tracing (Case 4) for the per-node overhead to be low enough."
  - "Requesting only 1 shard is silently ignored (warning 1119)."
  - "DCR's determinism check between shards adds ≤2.7% overhead in most cases but can be higher for irregular workloads."
  - "DCR slightly underperforms static CR up to 256 nodes (max 2.7% slowdown), but at 512 nodes DCR is 7.8% better."
  - "The replicate flag is per-task — missing it for even one critical top-level task eliminates the scaling benefit."
  - "If you set replicate=true but use a leaf task variant, you get normal replication (not control replication) — which is correct for leaf tasks but wrong for tasks that launch subtasks."
  - "At sufficient scale, control replication's collective communication overhead can itself become a bottleneck — critical path analysis may not capture this network-level saturation."
  - "There is no built-in mechanism for lightweight ordering of tasks to prevent a lower-priority task from running when a critical-path task is imminent."

fix:
  primary: |
    Mark the top-level task as replicable. In your mapper's
    select_task_options callback, set output.replicate = true for
    top-level tasks that launch subtasks and would benefit from control
    replication. Ensure you are NOT using leaf task variants for these
    tasks (use inner variants). Implement a ShardingFunctor that
    distributes work proportional to node resources. Register sharding
    functors symmetrically on all nodes before runtime startup via
    Runtime::register_sharding_functor(). For Regent programs, static
    Control Replication has been available since SC 2017. For all Legion
    programs, Dynamic Control Replication (DCR) is available in the
    runtime since PPoPP 2021 and is enabled by default in modern Legion.

  alternatives: |
    Use the DefaultMapper, which has heuristics for enabling control
    replication on appropriate tasks. Or derive from DefaultMapper and
    override select_task_options only for tasks where you need custom
    behavior. If CR cannot be used, manually structure the application
    in SPMD style with explicit per-node top-level tasks (this sacrifices
    the implicit parallelism programming model). For applications with
    hierarchical task patterns, ensure the sharding functor understands
    the hierarchy to avoid spraying tasks randomly across the machine.
    At extreme scale, reduce task granularity to decrease control message
    volume, or use static scheduling where possible for pipeline
    parallelism.

  what_not_to_do: |
    Do NOT attempt to fix this by adding more utility processors to
    node 0 — the fundamental issue is serial dependence analysis, not
    insufficient thread resources on the control node. Do NOT run
    distributed applications beyond ~16 nodes without DCR. Do NOT
    request only 1 shard (silently ignored, warning 1119). Do NOT
    implement asymmetric sharding functor registration across nodes.
    Do NOT set replicate=true on leaf tasks expecting control
    replication — leaf tasks get normal replication which is the correct
    (and cheaper) behavior for them. Do NOT set replicate=true for tasks
    that don't launch subtasks — it adds overhead for no benefit. Do NOT
    rely solely on critical path analysis for network-level scaling
    bottlenecks — it may point to local causes when the real issue is
    collective communication overhead.

verification: |
  After enabling DCR, Legion Prof should show shard tasks on multiple
  nodes. Dependence analysis and mapping work should be distributed.
  Utility activity should be balanced across all nodes. Scaling should
  continue beyond 16–32 nodes. Up to 99% parallel efficiency at 1,024
  nodes. DCR showed 11.4× speedup over Dask and 14.9× over TensorFlow
  on comparable workloads. PENNANT with DCR ran 2.3× faster than
  MPI+CUDA on 256 GPUs. HTR solver: 86% parallel efficiency on 9,216
  CPUs, 96.6% on 512 GPUs on Lassen. Soleil-X: 82% weak-scaling
  efficiency on 1,024 GPUs on Sierra.

real_cases:
  - case: "SC 2017 / PPoPP 2021 papers; GitHub legion#338"
    app: "All Legion applications (PENNANT, HTR solver, Soleil-X, etc.)"
    scale: "Up to 1,024 nodes / 9,216 CPUs / 1,024 GPUs"
    result: "99% parallel efficiency at 1,024 nodes; 2.3× faster than MPI+CUDA on 256 GPUs"
    key_detail: "Described by Slaughter as 'quite literally the optimization that saved Legion' — seven years to develop"
  - case: "DCR vs. Dask/TensorFlow comparison"
    app: "[implicitly parallel workloads]"
    scale: "[distributed]"
    result: "11.4× over Dask, 14.9× over TensorFlow"
    key_detail: "DCR's SPMD replication efficiently distributes control overhead."
  - case: "DCR vs. static CR at scale"
    app: "[not specified]"
    scale: "512 nodes"
    result: "DCR 7.8% better than static CR at 512 nodes; max 2.7% worse up to 256 nodes"
    key_detail: "DCR's advantages grow with scale despite slight overhead at smaller counts."
  - case: "[No specific case cited — Transcript 020]"
    app: "[not specified]"
    scale: "[not specified]"
    result: "[not specified]"
    key_detail: "The instructor says 'this is false by default, so it's off' — the opt-in nature is the entire problem"
  - case: "Talk 006 - Legate Jax"
    app: "Legate Jax"
    scale: "[multi-node, exact count not specified]"
    result: "[unresolved — described as 'truly inscrutable']"
    key_detail: "Critical path analysis identified scalar bundles initially, but then 'the network was melting'"

related_patterns:
  - dynamic_tracing_missing
  - graph_compilation_metg
  - missing_tracing
  - bad_sharding_functor
  - bad_sharding_function
  - leaf_inner_variant_confusion
  - task_end_gpu_bubbles

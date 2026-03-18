id: default_mapper_complex_hierarchy
title: DefaultMapper on complex hierarchical task patterns sprays tasks randomly
source: The mapper interface and custom mappers section; Anti-pattern reference table
confidence: high
user_type: all

symptoms:
  what_you_see: |
    High inter-node copy volume visible in channel views. Tasks execute on processors far from their data. Copy operations dominate the critical path. Poor data locality — tasks and their data are spread incoherently across the machine.

  key_metrics: |
    High inter-node copy volume. Poor locality (tasks execute far from data). Up to 26× performance gap vs. optimal mapping. Work stealing activity indicates load imbalance.

  distinguishing_features: |
    Unlike specific memory misplacement (GPU data in SYSTEM_MEM), the issue is the overall task-to-processor-to-memory mapping strategy. Unlike NUMA-unaware mapping (single-node issue), this involves cross-node data movement. The pattern is characterized by Elliott Slaughter's description of tasks being "randomly sprayed across the machine."

root_cause: |
  The DefaultMapper uses reasonable heuristics but has no application-specific knowledge about hierarchical task patterns, data reuse patterns, or computational structure. For complex applications, it maps tasks without understanding locality relationships, causing tasks to be distributed incoherently with excessive inter-node data movement.

gotchas:
  - "The DefaultMapper's select_task_options callback sets valid_instances = true by default, forcing the runtime to compute valid instance sets even when the task won't reuse them — setting it to false saves overhead."
  - "AutoMap (SC '23) showed 26× improvement over DefaultMapper for Pennant, demonstrating how large the gap can be."
  - "Writing a custom mapper requires relatively little code — Legion-SNAP's mapper was fewer than 100 runtime API calls (~2% of application code)."

fix:
  primary: |
    Write an application-specific mapper inheriting from DefaultMapper. Override key callbacks: map_task for memory placement, slice_task for NUMA-aware distribution, select_sharding_functor for distributed sharding. Set output.valid_instances = false when tasks don't reuse instances.

  alternatives: |
    Use AutoMap-style systematic exploration of the mapping space (SC '23). Use ProcessorQuery/MemoryQuery with has_affinity_to and memoize results for scalable machine queries.

  what_not_to_do: |
    Do NOT use the legacy get_all_processors/get_all_memories machine queries — they are not scalable in distributed settings. Do NOT keep DefaultMapper for applications with complex data locality requirements.

verification: |
  After implementing a custom mapper, inter-node copy volume should decrease. Task-to-data locality should improve. Overall throughput should increase (up to 26× in extreme cases). Work stealing activity should decrease if load balancing improves.

real_cases:
  - case: "AutoMap (SC '23)"
    app: "Pennant"
    scale: "mixed CPU/GPU configurations"
    result: "Up to 26× improvement over DefaultMapper"
    key_detail: "Systematic mapping space exploration revealed optimal configurations far from defaults."
  - case: "Legion-SNAP custom mapper"
    app: "Legion-SNAP"
    scale: "[not specified]"
    result: "Application-specific optimization with ~2% code overhead (~100 API calls)"
    key_detail: "Demonstrated that custom mappers are small relative to application code."

related_patterns:
  - "numa_unaware_mapping"
  - "gpu_data_in_system_mem"
  - "legacy_machine_queries"

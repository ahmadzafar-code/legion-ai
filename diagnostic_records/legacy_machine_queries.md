id: legacy_machine_queries
title: Legacy machine queries (get_all_processors/memories) are not scalable at distributed scale
source: The mapper interface and custom mappers section; Anti-pattern reference table
confidence: medium
user_type: legion_cpp

symptoms:
  what_you_see: |
    High mapper-call time visible on utility processors. Mapper calls (map_task, select_task_options) take disproportionately long. Slow startup or slow first-iteration performance that improves if results are cached.

  key_metrics: |
    High mapper-call time in utility processor profiles. Mapper calls take O(nodes) or O(processors) time. Scalability degrades with increasing node count.

  distinguishing_features: |
    Unlike low -ll:util (utility processors saturated by analysis), here the mapper calls themselves are slow. The overhead is in the mapper's machine queries, not in dependence analysis. Profiling mapper-call meta-tasks will show the time is spent inside the mapper, not in runtime analysis.

root_cause: |
  The legacy get_all_processors/get_all_memories API enumerates all hardware resources, which grows linearly with machine size. In distributed settings with many nodes, this becomes expensive. Additionally, calling these queries repeatedly without caching compounds the overhead.

gotchas:
  - "This overhead is in the mapper (user code), not in the runtime — it may not be obvious when looking at utility processor activity."
  - "Memoizing query results is essential — queries can be expensive and results don't change during execution."

fix:
  primary: |
    Use ProcessorQuery/MemoryQuery with has_affinity_to and best_affinity_to for targeted, efficient queries. Memoize query results — store them in mapper state on first call and reuse.

  alternatives: |
    Pre-compute all needed processor/memory mappings during mapper initialization rather than during callbacks.

  what_not_to_do: |
    Do NOT use get_all_processors or get_all_memories in mapper callbacks. Do NOT call ProcessorQuery/MemoryQuery without memoizing results.

verification: |
  After switching to targeted queries and memoization, mapper-call time in utility processor profiles should decrease. Scalability with increasing node count should improve. First-iteration overhead may remain (for initial memoization) but subsequent iterations should be fast.

real_cases: []

related_patterns:
  - "default_mapper_complex_hierarchy"
  - "low_utility_processors"

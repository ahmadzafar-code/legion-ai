id: deep_region_trees
title: Excessively deep region trees increase dependence analysis cost
source: Partitioning strategies and region trees section; Anti-pattern reference table
confidence: medium
user_type: legion_cpp

symptoms:
  what_you_see: |
    High utility processor time spent on dependence analysis. Slow task startup visible as gaps between task completions and next task starts. Utility processors show continuous activity computing least-common-ancestor (LCA) relationships.

  key_metrics: |
    High utility processor time per dependence check. Region trees deeper than 3–4 levels. Partition computation overhead visible before task execution begins.

  distinguishing_features: |
    Unlike aliased-partition serialization (tasks serialize due to assumed overlap), this pattern shows the runtime spending excessive time determining whether regions alias at all via LCA computation. The overhead is in the analysis itself, not in the serialization result. Unlike low -ll:util (insufficient utility processors), the issue is per-check cost, not throughput.

root_cause: |
  Dependence detection between sibling tasks requires checking whether region requirements may alias via least-common-ancestor (LCA) computation in the region tree. Deep trees increase the cost of each LCA check. Additionally, premature materialization of all subregions in a large partition can incur O(N) overhead, even though the runtime uses lazy region tree instantiation.

gotchas:
  - "Lazy region tree instantiation helps for large partition counts, but touching all subregions early (e.g., iterating over them) forces premature materialization with O(N) overhead."
  - "The depth issue compounds with aliased partitions — deep aliased trees are doubly expensive."

fix:
  primary: |
    Flatten region trees to 3–4 levels maximum. Restructure partitioning to use wider, shallower trees rather than deep hierarchies.

  alternatives: |
    Use dependent partitioning operations that create efficient tree structures. Avoid premature materialization of large partition subregions.

  what_not_to_do: |
    Do NOT create deeply nested partition hierarchies unless the application structure absolutely requires it. Do NOT touch all subregions of a large partition early in execution.

verification: |
  After flattening, utility processor time per dependence check should decrease. Gaps between task executions should shrink. Overall pipeline throughput should improve.

real_cases: []

related_patterns:
  - "aliased_partition_when_disjoint"
  - "low_utility_processors"

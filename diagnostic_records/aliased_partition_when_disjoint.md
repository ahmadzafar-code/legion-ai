id: aliased_partition_when_disjoint
title: Partition declared aliased when actually disjoint serializes independent tasks
source: Partitioning strategies and region trees section; Anti-pattern reference table
confidence: medium
user_type: all

symptoms:
  what_you_see: |
    Processor idle time bracketing task execution on the same region tree. Tasks on ostensibly independent subregions execute sequentially. Legion Spy's logical dependence graph shows edges between tasks on subregions that should be independent.

  key_metrics: |
    Serialization proportional to the number of subregions. Tasks execute sequentially despite operating on disjoint data. Partition computation overhead visible as time in runtime meta-tasks on utility processors.

  distinguishing_features: |
    Unlike privilege-induced serialization (same region, conflicting privileges), this pattern involves different subregions that should be independent. The telltale sign is Legion Spy showing dependence edges between tasks on different subregions of a partition the programmer knows to be disjoint. Unlike deep-region-tree overhead (slow LCA computation), the issue is the runtime conservatively assuming overlap.

root_cause: |
  When a partition is not declared DISJOINT_KIND, the runtime assumes subregions may overlap. This forces expensive dependence analysis and potentially serializes tasks that could run in parallel, because overlapping regions with conflicting privileges create dependencies. The runtime cannot prove non-interference without the disjointness guarantee.

gotchas:
  - "The converse — claiming disjointness for an actually-aliased partition — is a correctness bug catchable with -lg:partcheck, but this check 'can take arbitrarily long'."
  - "Dependent partitioning operations (create_equal_partition, etc.) automatically compute disjointness properties, avoiding this trap."
  - "The old coloring-based partitioning API was serial and single-node — switching to dependent partitioning also provides performance benefits (2.6–12.7× on a single thread, 29× distributed on 64 nodes)."

fix:
  primary: |
    Always specify DISJOINT_KIND when the partition is provably disjoint. Use dependent partitioning operations (create_equal_partition, create_partition_by_preimage) that automatically compute and propagate disjointness properties.

  alternatives: |
    For stencil patterns requiring ghost regions, create two partitions of the same region: one disjoint (owned data) and one aliased (ghost data), using SIMULTANEOUS coherence for the ghost partition with explicit copy launchers and phase barriers.

  what_not_to_do: |
    Do NOT claim DISJOINT_KIND for a partition that is actually aliased — this is a correctness bug. Do NOT rely on -lg:partcheck in production — it can take arbitrarily long.

verification: |
  After specifying DISJOINT_KIND, previously serialized tasks should execute in parallel. Legion Spy's dependence graph should no longer show edges between tasks on different subregions. Processor utilization should increase. Use -lg:partcheck in testing (not production) to validate disjointness claims.

real_cases:
  - case: "OOPSLA 2016 dependent partitioning paper"
    app: "[multiple]"
    scale: "64 nodes"
    result: "86–96% code reduction; 2.6–12.7× single-thread speedup; 29× distributed speedup"
    key_detail: "Dependent partitioning API automatically computes disjointness, avoiding manual mis-declaration."

related_patterns:
  - "deep_region_trees"
  - "individual_task_launches"

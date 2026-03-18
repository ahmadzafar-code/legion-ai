id: bad_sharding_function
title: Bad sharding function concentrates all work on one shard/node or causes fatal error
source:
  - Transcript 018 (Control Replication Part 3)
  - Control replication and distributed sharding section; Anti-pattern reference table
confidence: medium
user_type: legion_cpp

symptoms:
  what_you_see: |
    In Legion Prof, control replication IS enabled (shard tasks are visible), but all
    dependence analysis and subtask management occurs on a single shard/node. Other
    shards are idle or doing minimal work. The profiler shows an extreme load imbalance
    across shards. In a more severe variant, the application crashes immediately with
    fatal runtime error #67 during sharding — no performance degradation is observed
    because it is a hard failure at startup.

  key_metrics: |
    - One shard with near 100% utilization for management tasks
    - Other shards at near 0% utilization for management tasks
    - Number of effective shards = 1 regardless of node count
    - No scaling benefit from additional nodes
    - Fatal error #67 (immediate crash during distributed execution when slice points span multiple shards)

  distinguishing_features: |
    Unlike missing control replication (where no shards exist), here shards DO exist
    but all work is concentrated. The sharding function is enabled but degenerate.
    In Legion Spy, per-shard data flow graphs would show one shard with all tasks
    and others empty. The fatal error #67 variant is distinct from other DCR issues
    (silently ignored single shard, warning 1119) — it is an immediate fatal error
    that occurs when the mapper specifies a slice where not all points map to the
    same shard. The error number #67 is the definitive diagnostic for that variant.

root_cause: |
  The sharding function is a mapper-level decision that determines which shard handles
  which portion of the work. A sharding function that maps everything to one shard
  (e.g., always returning shard 0, or using a hash function with poor distribution)
  completely defeats the purpose of control replication. All dependence analysis,
  mapping, and execution scheduling happens on one node. In the fatal variant, the
  ShardingFunctor must ensure that all slice points within an index launch that
  correspond to a given shard are consistently assigned. If a mapper specifies a slice
  where not all points map to the same shard, the runtime detects the inconsistency
  and terminates with error #67.

gotchas:
  - "The sharding function's quality directly determines scalability — there is no runtime fallback for a bad sharding function."
  - "A common mistake is to shard by task ID but use task IDs that all hash to the same shard."
  - "You can verify with Legion Spy: per-shard data flow graphs should be structurally identical (modulo UIDs). If one shard has all the tasks, the sharding function is broken."
  - "The sharding functor must be registered symmetrically on ALL nodes before runtime startup."
  - "The sharding functor must understand hierarchical task patterns to distribute work correctly."
  - "A mismatched shard assignment (slice points spanning multiple shards) causes immediate fatal error #67 — there is no graceful degradation."

fix:
  primary: |
    Implement a sharding function that distributes work evenly across shards.
    Typically this means sharding index launch point tasks by their point index
    modulo the number of shards, or using a spatial partition-based sharding
    that matches your data distribution. Ensure the ShardingFunctor consistently
    assigns all slice points within a single operation to the same shard. Verify
    that work distribution is proportional to node resources.

  alternatives: |
    Use the DefaultMapper's sharding function as a starting point. Validate
    distribution using Legion Spy's per-shard data flow graphs. Use simpler
    sharding strategies (e.g., round-robin by point index) as a baseline,
    then refine for locality.

  what_not_to_do: |
    Do NOT use a constant sharding function (always returning the same shard).
    Do NOT assume that a hash function automatically provides good distribution
    without testing. Do NOT create sharding functors where slice points within
    a single operation span multiple shards. Do NOT register sharding functors
    asymmetrically across nodes.

verification: |
  After fixing, Legion Spy per-shard data flow graphs should show approximately
  equal numbers of tasks per shard. Legion Prof should show balanced utilization
  across shard nodes. Scaling efficiency should improve with node count. Fatal
  error #67 should not occur. Legion Prof should show even utility processor
  distribution across nodes.

real_cases:
  - case: "[No specific case cited]"
    app: "[not specified]"
    scale: "[not specified]"
    result: "[not specified]"
    key_detail: "The instructor says 'hopefully they don't do that' — implying this is a known failure mode"

related_patterns:
  - "missing_control_replication_optin"
  - "no_control_replication"
  - "default_mapper_complex_hierarchy"

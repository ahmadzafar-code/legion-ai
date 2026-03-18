id: remote_allocation_overhead
title: Remote memory allocation incurs messaging overhead to owner node
source: Transcript 004 (Instance Allocation, Memory Managers, GC)
confidence: medium
user_type: legion_cpp

symptoms:
  what_you_see: |
    In Legion Prof, allocation operations on non-owner nodes show significantly longer
    durations than local allocations. Network message tasks appear in the timeline
    associated with instance creation. Application tasks on remote nodes wait longer
    for their instances to be ready.
    [INCOMPLETE — needs review: specific message task names not provided]

  key_metrics: |
    - Allocation latency on non-owner nodes >> allocation latency on owner node
    - Network messages correlated with instance creation events
    - Imbalanced allocation latency across nodes

  distinguishing_features: |
    Unlike local GC overhead (which affects all nodes), this specifically affects nodes
    that are NOT the owner of the target memory. The owner node performs allocations
    normally. Only non-owner nodes pay the messaging penalty.

root_cause: |
  In Legion, only the owner node of a memory can perform allocations and deletions
  directly. Non-owner nodes must send a message to the owner. The runtime optimizes
  this by first checking local knowledge for existing instances that satisfy the
  request, but if no local match exists, a remote message is required. Mappers that
  frequently allocate in remote memories pay this messaging overhead on every allocation.

gotchas:
  - "The runtime tries to find locally-known instances first before sending a message to the owner — but if the mapper always needs NEW instances (not reusing), every allocation becomes a remote message."
  - "This compounds with the instance churn problem: if you churn instances in remote memories, you get both expensive GC AND messaging overhead."
  - "The owner node concept is per-memory, not per-node — a node can own some memories but not others."

fix:
  primary: |
    Prefer allocating in local memories where the mapper's node is the owner.
    Use find_or_create to maximize reuse of existing instances in remote memories,
    minimizing the need for new allocations.

  alternatives: |
    Structure the mapping so that tasks requiring instances in a particular memory
    are mapped to the node that owns that memory. This is a mapper-level decision
    that aligns allocation with ownership.

  what_not_to_do: |
    Do NOT assume all memories are equally fast to allocate in — remote allocation
    has fundamentally higher latency due to the ownership messaging protocol.

verification: |
  After optimizing, remote allocation messages should decrease. Instance reuse rate
  should increase. Allocation latency on non-owner nodes should approach that of
  the owner node (when instances are being reused rather than created).

real_cases:
  - case: "[No specific case cited]"
    app: "[not specified]"
    scale: "[not specified]"
    result: "[not specified]"
    key_detail: "The runtime first checks local knowledge before sending a remote message — the optimization exists but only helps if reusable instances exist locally"

related_patterns:
  - "instance_churn_expensive_gc"

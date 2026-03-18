id: instance_churn_expensive_gc
title: Frequent instance creation/destruction triggers expensive garbage collection
source: Transcript 004 (Instance Allocation, Memory Managers, GC), Category 3; low_processor_utilization_diagnosis.md, Category 4; GitHub issue #1739
confidence: high
user_type: all

symptoms:
  what_you_see: |
    Memory timeline rows show occupancy near capacity with rapid
    instance creation/destruction cycles. Deferred allocations appear
    as shaded regions on instance bars — the interval between create
    and ready indicates time spent waiting for memory to become
    available. On utility processors, meta-tasks "Malloc Instance"
    (enum ID 77) and "Free Instance" (enum ID 78) appear frequently,
    along with "Defer Physical Manager Deletion", "Defer Release
    Acquired Instances", "Copy Fill Deletion", "Free External
    Allocation", and "Defer Delete Future Instance". Application
    processors show gaps while waiting for new instances to be
    allocated.

  key_metrics: |
    - Q4.1: Any memory above 85% peak occupancy
    - Q4.2: Instance creation rate >1000 instances/second on a single memory = severe churn
    - Q4.3: "Malloc Instance" / "Free Instance" consuming significant utility processor time
    - Q4.4: Deferred allocations above 1ms (ready_time - create_time > 1,000,000 ns)
    - All three of Q4.1, Q4.3, and Q4.4 must be positive for confident diagnosis
    - High instance destruction / GC trigger rate
    - Memory utilization sawtooth pattern

  distinguishing_features: |
    Unlike Category 1 (runtime overhead), utility processors are
    dominated by allocation meta-tasks ("Malloc Instance", "Free
    Instance", "Defer Physical Manager Deletion") rather than analysis
    meta-tasks ("Logical Dependence Analysis", "Trigger Task Mapping").
    Unlike Category 2 (communication), channels are idle — the
    bottleneck is allocation, not transfer. Unlike a memory leak
    (monotonically increasing usage), this shows rapid cycling or
    near-capacity occupancy with high churn. Unlike remote allocation
    overhead (where only remote memories are slow), this affects even
    local memory allocation when GC is triggered.

root_cause: |
  Available memory is nearly exhausted, forcing the runtime's lazy
  garbage collector to free invalid instances before allocating new
  ones. Legion's GC protocol is intentionally designed with a bias:
  acquires are fast, collections are expensive. The design assumes
  acquires (finding/reusing instances) are much more common than
  collections (freeing instances). When a mapper creates and destroys
  instances frequently (high instance churn), it triggers the expensive
  GC protocol repeatedly, violating the design assumption. In extreme
  cases, the runtime repeatedly creates and destroys instances for
  every task, turning what should be a one-time allocation into
  per-task overhead. Processors wait for instance allocation to
  complete before they can execute tasks.

  The GC uses a priority ordering: perfect-fit holes first, then larger
  holes (smallest first to minimize fragmentation), then smaller holes.
  This exhaustive search adds cost to every collection cycle.

  Additionally, instance finding can race with the garbage collector —
  while one mapper searches for existing instances, another mapper's
  allocation might trigger GC that deletes the instance being found.

gotchas:
  - "Legion's GC is lazy: memory usage appears to grow monotonically, which users often mistake for a memory leak (documented in GitHub issue #1739). Most instances may be invalid and reclaimable."
  - "The GC bias is intentional and correct — the fix is to change the mapper's allocation strategy, not to 'fix' the GC."
  - "find_or_create_physical_instance exists specifically for this — it is an atomic operation that prevents duplicate creation races AND promotes instance reuse."
  - "Instance finding can race with GC: a mapper might find a suitable instance that gets collected by a concurrent allocation before it can acquire it."
  - "The proposed 'truly-in-use' memory line feature has been requested but is not yet implemented — users cannot easily distinguish 'full of invalid instances' from 'full of valid instances' in the profile."
  - "Warning 1122: 'Detected unbounded pool in trace' means trace replay is creating unbounded instance pools that prevent memory reclamation — requires restructuring the traced code section."
  - "The GC priority ordering (perfect → larger → smaller holes) means fragmented memories pay more for collection."
  - "Communication + Memory pressure co-occur: tight memory stalls buffer allocation for copies, visible as high channel utilization with unusual copy start delays (large ready→start gaps). Fix -ll:ib_rsize and -ll:fsize BEFORE tuning communication."

fix:
  primary: |
    Increase memory allocation:
    - -ll:fsize N for GPU framebuffer (default 256MB — far too small
      for real workloads; typical: 14,000–70,000MB)
    - -ll:zsize N for zero-copy (default 64MB)
    - -ll:csize N for CPU memory (default 512MB)
    Set these to 80–90% of physical capacity.

    Additionally, use find_or_create_physical_instance instead of
    separate find + create calls. This atomically searches for a
    reusable instance and creates one only if none exists, preventing
    both duplicate creation and unnecessary churn.

  alternatives: |
    - Configure mapper instance reuse: the map_task callback should
      prefer existing instances with valid data over creating new ones,
      eliminating both allocation overhead and redundant copies.
    - Implement instance caching in the mapper — maintain a mapper-local
      pool of instances and reuse them across task mappings. The
      DefaultMapper does this.
    - Reduce the frequency of instance destruction by keeping instances
      alive longer.
    - LEGATE_FIELD_REUSE_FREQ (default 32): Controls how often Legate
      performs distributed consensus match to identify reclaimable
      RegionFields. Lower values (e.g., 16) reclaim faster at cost
      of more frequent collective operations. Higher values (e.g., 64)
      reduce overhead but increase memory pressure.
    - -lg:eager_alloc_percentage N (e.g., 10): Reduces deferred
      allocation stalls.
    - If Warning 1122 fires ("Detected unbounded pool in trace"),
      restructure the traced code section to bound instance pools.

  what_not_to_do: |
    Do NOT create a new instance for every task mapping — this is the
    primary cause of churn. Do NOT call separate find then create —
    the non-atomic sequence is vulnerable to races and duplicates.
    Do NOT assume monotonically growing memory usage is a memory leak —
    Legion's lazy GC creates this appearance (GitHub issue #1739). Do
    NOT lower LEGATE_FIELD_REUSE_FREQ too aggressively (e.g., 1) as
    the frequent collective operations become their own bottleneck.
    Do NOT leave -ll:fsize at the default 256MB for any real GPU
    workload.

verification: |
  After applying fixes:
  1. Q4.1 peak occupancy should drop below 85%.
  2. Q4.2 instance creation rate should decrease significantly.
  3. Q4.3 "Malloc Instance" / "Free Instance" time on utility
     processors should drop.
  4. Q4.4 deferred allocations above 1ms should decrease or disappear.
  5. Application processor gaps caused by allocation waits should close.
  6. GC triggers should become rare.
  7. Memory utilization should stabilize.

real_cases:
  - case: "GitHub issue #1739"
    app: "Not specified"
    scale: "Not specified"
    result: "Identified as documentation/UX issue — users mistook lazy GC behavior for memory leak"
    key_detail: "Memory appears full but most instances are invalid and reclaimable; proposed 'truly-in-use' memory line not yet implemented"
  - case: "[No specific case cited]"
    app: "[not specified]"
    scale: "[not specified]"
    result: "[not specified]"
    key_detail: "The instructor uses the phrase 'acquires should be much more common than collects — collection should be much more sporadic' to describe the design intent"

related_patterns:
  - "remote_allocation_overhead"
  - "runtime_overhead_no_tracing"
  - "communication_blocking_systemic"
  - "memory_pressure_with_communication"

  ```yaml

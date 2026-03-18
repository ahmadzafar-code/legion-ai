id: serialized_mapper_bottleneck
title: Serialized mapper mode creates scheduling bottleneck under concurrency
source: Transcript 003 (Scheduling and Mapper Calls), Category 3
confidence: medium
user_type: legion_cpp

symptoms:
  what_you_see: |
    In Legion Prof, mapper calls on utility processors appear strictly sequentialized —
    no two mapper calls overlap in time. Even when multiple tasks are ready to map
    concurrently, only one mapper call proceeds at a time. Application processors
    show idle gaps waiting for their tasks to be mapped.

  key_metrics: |
    - Mapper call concurrency = 1 at all times (despite multiple ready tasks)
    - Queuing delay visible before mapper calls
    - Low mapper throughput relative to task arrival rate

  distinguishing_features: |
    Unlike expensive-mapper-calls (where each call is slow), here individual calls may
    be fast but they are artificially sequentialized. The bottleneck is the serialization
    lock, not the mapper logic itself. Switching to concurrent mode would immediately
    show overlapping mapper calls.

root_cause: |
  The serialized (re-entrant) mapper manager guarantees that at most one mapper call
  executes at a time for a given mapper object. This provides safety for non-thread-safe
  mapper state but creates a serialization bottleneck when many tasks need mapping
  simultaneously. The serializing mapper manager has more state and synchronization
  overhead than the concurrent variant.

gotchas:
  - "The runtime DOES pause serialized mapper calls during long-latency operations (like instance creation) to allow other calls through — but this only helps if instance operations are the bottleneck, not the mapper logic itself."
  - "Serialized mode also uses a pause/resume mechanism to prevent deadlocks when mapper calls block on runtime work — removing serialized mode requires ensuring your mapper is actually thread-safe."
  - "Choosing concurrent mode with a non-thread-safe mapper introduces data races that can cause subtle, hard-to-reproduce bugs."

fix:
  primary: |
    If your mapper implementation is thread-safe (no shared mutable state, or properly
    synchronized), switch to the concurrent mapper manager. This allows multiple mapper
    calls to execute simultaneously.

  alternatives: |
    If you cannot make the mapper fully thread-safe, optimize each mapper call to be
    as fast as possible to minimize serialization impact. Use fine-grained locking
    inside the mapper for specific shared data structures rather than relying on
    the runtime's global serialization.

  what_not_to_do: |
    Do NOT switch to concurrent mode without verifying thread safety — data races in
    the mapper are extremely difficult to diagnose because they manifest as incorrect
    mapping decisions that may only fail under specific timing conditions.

verification: |
  After switching to concurrent mode, Legion Prof should show overlapping mapper calls
  on utility processors. Mapper throughput should increase, and idle gaps on application
  processors should shrink.

real_cases:
  - case: "[No specific case cited]"
    app: "[not specified]"
    scale: "[not specified]"
    result: "[not specified]"
    key_detail: "The instructor emphasizes this is a conscious tradeoff — serialized mode 'has more state and has to do more synchronization for clients'"

related_patterns:
  - "expensive_mapper_calls"
  - "serialized_mapper_deadlock"

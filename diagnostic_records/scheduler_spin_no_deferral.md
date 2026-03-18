id: scheduler_spin_no_deferral
title: Scheduler spin-loop from mapper not providing deferral event
source: Transcript 002 (Tasks, Context, Forward Progress), Transcript 003 (Scheduling and Mapper Calls), Category 3
confidence: high
user_type: legion_cpp

symptoms:
  what_you_see: |
    In Legion Prof, the utility processor for the affected node shows 100% utilization with
    continuous select_task_to_map calls in rapid succession. No actual task mapping occurs
    between these calls. Other meta-tasks on the same processor show no progress — the
    timeline has no gaps between mapper calls but also no actual work getting done. Application
    tasks that are ready remain unmapped and idle on application processors.

  key_metrics: |
    - Utility processor utilization at or near 100% with only select_task_to_map calls
    - Zero tasks mapped per select_task_to_map invocation
    - No deferral event returned by mapper
    - Application processor utilization near 0% (tasks starved)

  distinguishing_features: |
    Unlike expensive-mapper-call patterns where the mapper is doing useful work slowly,
    here the mapper returns quickly but without doing anything. The signature is extremely
    high call frequency with zero productive output. In recent runtime versions this
    raises an explicit runtime error rather than silently spinning.

root_cause: |
  When the mapper's select_task_to_map callback returns without either (a) mapping any of
  the ready tasks or (b) providing a deferral event, the scheduler loop immediately
  re-invokes the mapper. This creates a hot spin that starves all other meta-task work
  on that processor. The scheduler uses continuation tasks with carefully tuned priorities
  to allow interleaving; the spin prevents those continuations from making progress.

  Historical note: an early Legion bug had the continuation task priority set too high,
  which exacerbated this into total starvation even when the mapper did provide a deferral.

gotchas:
  - "In current runtime versions, this is a runtime ERROR, not just a performance problem. If you see a deferral-event error, this is the cause."
  - "The mapper may THINK it mapped something but the task was relocated to a remote mapper — from the local scheduler's perspective, nothing was mapped locally."
  - "Easily confused with an expensive mapper call — but expensive calls have long durations per call, while this pattern has extremely short durations repeated endlessly."

fix:
  primary: |
    Ensure your mapper's select_task_to_map implementation always either:
    (a) Maps at least one task from the ready queue, OR
    (b) Returns a valid deferral event (e.g., an event that triggers when a new task becomes ready).
    The runtime enforces this invariant and raises an error if violated.

  alternatives: |
    If the mapper legitimately cannot map anything (e.g., waiting for a resource),
    it must provide a deferral event tied to the condition it is waiting for.
    Using MapperRuntime::create_mapper_event() and triggering it when the condition resolves.

  what_not_to_do: |
    Do NOT busy-wait inside select_task_to_map hoping the condition changes — this holds
    the mapper serialization lock (in serialized mode) and prevents ALL other mapper calls.

verification: |
  After fixing, utility processor utilization should drop significantly during periods
  when no tasks are ready to map. The select_task_to_map call frequency should match
  the rate at which tasks become ready, not a continuous hot loop. No runtime deferral-event
  errors should appear.

real_cases:
  - case: "Historical Legion bug (mentioned in lecture)"
    app: "[not specified]"
    scale: "[not specified]"
    result: "Priority fix eliminated starvation loop"
    key_detail: "The continuation task priority was set too high, causing starvation even with correct mapper behavior"

related_patterns:
  - "expensive_mapper_calls"
  - "serialized_mapper_bottleneck"

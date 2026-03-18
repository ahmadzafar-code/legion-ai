id: leaf_inner_task_marking
title: Unmarked or mismatched leaf/inner task variants cause unnecessary overhead or runtime errors
source: Transcript 020 (Control Replication Part 5); Task granularity and launch overhead section; Anti-pattern reference table; Transcript 002 (Tasks, Context, and Forward Progress)
confidence: medium
user_type: all

symptoms:
  what_you_see: |
    Case 1 (leaf variant used for inner task): Subtask launches fail or produce
    runtime errors. The task was replicated normally (not control-replicated) but
    it tries to launch subtasks, which requires control replication.

    Case 2 (inner variant used for leaf task): Unnecessary control replication
    overhead is visible in Legion Prof — shard management tasks, consensus
    operations, and distributed analysis appear for a task that doesn't launch
    any subtasks. The timeline shows management overhead with no subtasks.

    Case 3 (unmarked tasks): Runtime overhead on trivial tasks. Warning 1087 fires
    for tasks that only launch sub-operations but aren't marked inner. Tasks that
    perform no sub-operations still have full runtime bookkeeping overhead, with
    unnecessary inner context creation and destruction visible as increased per-task
    runtime cost.

  key_metrics: |
    - Case 1: Runtime errors on subtask launch from replicated task
    - Case 2: Control replication overhead tasks visible with zero subtasks launched
    - Unnecessary shard management operations in profiler
    - Warning 1087 present in output
    - Runtime overhead on simple tasks higher than expected
    - Unnecessary bookkeeping visible in utility processor activity for leaf tasks
    - Inner context creation for tasks that launch no subtasks
    - Per-task overhead higher than necessary

  distinguishing_features: |
    Case 1 is a correctness error (task fails). Cases 2 and 3 are performance issues
    (task succeeds but with unnecessary overhead). Distinguished from missing
    replication by the fact that replication IS happening — just the wrong kind.
    Unlike tasks-too-small (fundamental granularity issue), cases 2/3 are unnecessary
    overhead on tasks that could be optimized with a simple annotation. Warning 1087
    is the key diagnostic for unmarked inner tasks. The fix is trivial (one line of
    code) but the impact compounds over many tasks.

root_cause: |
  In Legion, leaf task variants get normal replication (cheap, independent copies)
  while inner task variants get control replication (expensive, coordinated copies
  that can launch subtasks). If the mapper selects a leaf variant for a task that
  needs to launch subtasks, the task cannot do so. If the mapper selects an inner
  variant for a task that never launches subtasks, it pays control replication
  overhead unnecessarily. Additionally, without set_leaf(true), the runtime prepares
  full sub-task bookkeeping infrastructure (a full inner context) even for tasks that
  never launch sub-operations. Without set_inner(true), the runtime doesn't know a
  task only launches sub-operations and cannot optimize accordingly. These annotations
  allow the runtime to skip unnecessary preparation.

gotchas:
  - "The distinction between replication and control replication in Legion is ENTIRELY determined by whether you use leaf or inner task variants."
  - "set_leaf(true) means the task launches NO sub-operations — not even copies or fills."
  - "set_inner(true) means the task ONLY launches sub-operations — it performs no direct computation."
  - "Mislabeling a task as leaf when it actually launches sub-operations will cause runtime errors."
  - "Task variants must be registered as leaf variants at registration time; you can't change this dynamically."
  - "Not all tasks CAN be leaf — only tasks that truly never launch subtasks or perform runtime operations."
  - "Shards don't have to be one-per-node — you can have multiple shards on the same node or shards on only some nodes."
  - "If you're not sure whether a task will launch subtasks, inner is the safe (but expensive) choice."
  - "This is an optimization miss, not a bug — the application runs correctly but slower (for cases 2/3)."

fix:
  primary: |
    Mark tasks with set_leaf(true) if they perform no sub-operations (no child tasks,
    no copies, no fills). Mark tasks with set_inner(true) if they only launch
    sub-operations. In the mapper's select_task_options, ensure that tasks which
    launch subtasks use inner task variants (enabling control replication), and tasks
    which do NOT launch subtasks use leaf task variants (enabling cheaper normal
    replication). In Regent, use the __demand(__leaf) annotation on leaf tasks.

  alternatives: |
    Review all task registrations and ensure variant types match actual task behavior.
    Use the DefaultMapper which typically makes reasonable variant choices.
    Review task implementations and identify which ones are pure computation with
    no runtime interactions beyond accessing their mapped regions.

  what_not_to_do: |
    Do NOT use leaf variants for tasks that launch subtasks — this is a correctness
    error, not just a performance issue. Do NOT mark a task as inner if it performs
    direct computation.

verification: |
  After fixing, Case 1 runtime errors should disappear. For Cases 2/3, control
  replication overhead should be replaced by simpler normal replication, visible as
  reduced shard management tasks in the profiler. Warning 1087 should disappear.
  Runtime overhead per task should decrease. Utility processor activity for these
  tasks should be reduced. The runtime should create lightweight contexts instead of
  full inner contexts for the optimized tasks.

real_cases:
  - case: "[No specific case cited]"
    app: "[not specified]"
    scale: "[not specified]"
    result: "[not specified]"
    key_detail: "The instructor explicitly states: 'if your task is not going to be leaf tasks, it's actually going to be able to launch sub tasks, then you have to do control application'"
  - case: "[No specific case cited]"
    app: "[not specified]"
    scale: "[not specified]"
    result: "[not specified]"
    key_detail: "The instructor notes: 'It's just an optimization that you're missing by not making a leaf context'"

related_patterns:
  - "missing_control_replication_optin"
  - "bad_sharding_function"
  - "tasks_too_small"
  - "generic_accessors"

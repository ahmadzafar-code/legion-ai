id: insufficient_parallelism_mapper_serialization
title: Independent tasks serialized by mapper placement decisions
source: low_processor_utilization_diagnosis.md, Category 3
confidence: medium
user_type: legion_cpp

symptoms:
  what_you_see: |
    Gaps appear on most application processors, but one or a few
    processors show continuous task execution. Utility processors are
    NOT saturated (distinguishing from Category 1). The task graph
    contains sufficient independent tasks, but they are placed on the
    same processor by the mapper.

  key_metrics: |
    - Q3.2: Low overall utilization
    - Q3.3: Utility processors idle during gaps (not a runtime bottleneck)
    - Uneven distribution of tasks across processors visible in timeline
    - Sufficient tasks exist (task count > processor count)

  distinguishing_features: |
    Unlike "too few tasks" (Category 3a), the task count IS sufficient.
    Unlike dependency serialization (Category 3b), the task graph IS
    wide. The problem is purely in the mapper's map_task callback
    placing independent tasks on the same processor or creating false
    serialization through instance choices.

root_cause: |
  The mapper's map_task callback selects the same processor for
  independent tasks, or creates instance sharing patterns that
  inadvertently serialize tasks. This is a mapper bug, not an
  algorithmic limitation.

gotchas:
  - "This is distinct from the sharding issue in Category 2 (communication) — here the tasks are serialized on one processor, not spread across nodes causing excessive copies."
  - "Verifying this requires examining the mapper's task placement logic, which may not be directly visible in the profile without comparing task op_ids and processor assignments."

fix:
  primary: |
    Fix the mapper's map_task callback to distribute independent tasks
    across available processors. Ensure instance choices do not create
    false serialization.

  alternatives: |
    - Use the DefaultMapper which distributes tasks round-robin if no
      custom mapper is needed.
    - Review the mapper's select_task_options callback for unnecessary
      constraints.

  what_not_to_do: |
    Do NOT attempt to fix this with runtime flags — this is a mapper
    code bug that requires code changes.

verification: |
  After fixing mapper placement:
  1. Tasks should be evenly distributed across application processors
     in the timeline.
  2. Q3.2 time slices with <20% utilization should decrease.
  3. All application processors should show similar utilization levels.

real_cases:
  - case: "[INCOMPLETE — needs review]"
    app: "[INCOMPLETE — needs review]"
    scale: "[INCOMPLETE — needs review]"
    result: "[INCOMPLETE — needs review]"
    key_detail: "Document identifies this as a mapper bug sub-variant but provides no specific case"

related_patterns:
  - "insufficient_parallelism_too_few_tasks"
  - "communication_blocking_localized"

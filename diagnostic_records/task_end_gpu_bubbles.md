id: task_end_gpu_bubbles
title: GPU Utilization Bubbles from Task-End Synchronization
source: "006 - Jeremy Wilke (Legate Jax Performance Investigation)"
confidence: medium
user_type: legate

symptoms:
  what_you_see: |
    Periodic gaps (bubbles) in GPU utilization between tasks, even when
    tasks are being submitted onto the same GPU stream. The gaps appear
    at task boundaries. GPU rows show a sawtooth or intermittent pattern
    rather than continuous execution.

  key_metrics: |
    - GPU utilization shows periodic drops at task boundaries.
    - [INCOMPLETE — needs review] No specific gap duration or utilization
      threshold given.
    - Tasks are submitting to the same stream (ruling out multi-stream
      interference).

  distinguishing_features: |
    Unlike runtime_limited_no_tracing (where GPUs starve because the
    runtime can't feed them), here the tasks ARE being submitted but
    each task waits for its GPU effects to become visible before being
    marked done. The bubbles are per-task, not per-pipeline-stage.
    The GPU is periodically idle BETWEEN tasks even though work exists.

root_cause: |
  "Tasks by default are not marked done until all of the data effects
  on the GPU are visible." Even when submitting onto the same CUDA
  stream, the task-completion mechanism waits for GPU effects to be
  visible to the runtime before marking the task done and allowing
  dependent tasks to proceed. This creates a synchronization bubble
  at every task boundary.

gotchas:
  - "This is a default behavior, not a misconfiguration. It exists to ensure data consistency."
  - "Submitting to the same stream does NOT avoid this — the issue is task-completion visibility, not stream ordering."
  - "[INCOMPLETE — needs review] The document does not describe the specific fix applied or whether one exists. This may require Legion-level changes to task completion semantics."

fix:
  primary: |
    [INCOMPLETE — needs review] The source document identifies the
    problem but does not describe a specific fix. Potential approaches:
    - Investigate task completion policies that allow marking tasks done
      before GPU effects are globally visible (if data dependencies
      are stream-ordered anyway).
    - Use task fusion to amortize the per-task synchronization cost
      over more work.

  alternatives: |
    - Increase per-task GPU work to amortize the synchronization overhead.
    - Investigate whether Legion provides any "fire and forget" or
      relaxed completion modes for same-stream task chains.

  what_not_to_do: |
    Do NOT assume this is a CUDA stream synchronization issue requiring
    -cuda:legacysync or similar flags. The problem is at the Legion
    task-completion layer, not the CUDA stream layer.

verification: |
  After fixing, GPU utilization should show more continuous execution
  with smaller gaps at task boundaries. Overall GPU utilization
  percentage should increase.

real_cases:
  - case: "Talk 006 - Legate Jax"
    app: "Legate Jax"
    scale: "[not specified]"
    result: "[qualitative — identified as a contributing factor to scaling bottleneck]"
    key_detail: "Discovered alongside 'a truly inscrutable scaling bottleneck' involving many small control replication messages"

related_patterns:
  - "control_replication_scaling_bottleneck"

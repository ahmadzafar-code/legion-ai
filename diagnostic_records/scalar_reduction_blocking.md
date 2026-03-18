id: scalar_reduction_blocking
title: Scalar reduction blocks GPU with host-side synchronization
source: GPU differential diagnosis guide, Cause 1; Legion issue #440 (Soleil-X / Regent CUDA codegen); GitHub StanfordLegion/legion#440; Case 2
confidence: high
user_type: regent

symptoms:
  what_you_see: |
    Legion Prof GPU timeline shows gaps aligned with specific tasks that
    perform scalar reductions (e.g., `Flow_AddTurbulentSource`,
    `Particles_DeleteEscapingParticles`). The GPU is idle during the
    reduction postamble of these tasks. The CPU processor row shows brief
    active computation during the same interval — this is the host-side
    reduction running serially. Utility processors are quiet (not the
    bottleneck). Gaps are more prominent on faster GPUs (e.g., Sherlock)
    where kernel time is short relative to host-side blocking time, and
    may be invisible on older/slower GPUs (e.g., Sapling).

  key_metrics: |
    - GPU task `waiting` time large relative to `running` time (high wait_ratio)
    - GPU utilization drops to zero during scalar reduction postamble
    - CPU processor shows short active (`running > 0`) entries overlapping the GPU gap
    - Utility processor activity: low/none during [T1, T2]
    - Gap aligned with tasks whose `title` matches known reduction operations
    - Cross-GPU comparison: latency-bound, not compute-bound (faster GPUs show worse relative impact)

  distinguishing_features: |
    CPU is ACTIVE (performing host-side reduction computation) during the gap —
    unlike Cause 4 (blocking Python) where the CPU/Python processor is
    blocked/waiting. Unlike Cause 2 / Case 5 (thread oversubscription / CUDA
    stream interference), utility processors are quiet and the gaps are
    task-specific, aligned with reduction operations, not periodic across all
    tasks. Unlike Causes 5/7, no channel activity or system-wide idleness is
    present. The gaps correlate with specific task names containing reductions.

root_cause: |
  The Regent CUDA code generator's `generate_reduction_postamble` function in
  `cudahelper.t` called `cudaDeviceSynchronize()`, followed by a blocking
  `cudaMemcpy(DeviceToHost)`, then performed the final scalar reduction on the
  CPU in a serial loop. The GPU sat completely idle during this host-side
  processing. The blocking synchronization serialized overlappable work. Any
  GPU task that calls `cudaDeviceSynchronize` or blocking `cudaMemcpy` during
  execution will produce this pattern.

gotchas:
  - "On older/slower GPUs the gap is imperceptible — the pattern only becomes visible on fast GPUs where kernel time is short relative to host-side blocking time. Upgrade to faster GPUs and it suddenly appears."
  - "May be confused with Cause 4 (blocking Python) if you don't check whether CPU is actively computing vs. blocked/waiting during the gap"
  - "The gap appears inside or aligned to a specific task, not between arbitrary tasks — if you see it between unrelated tasks, look elsewhere"
  - "Do NOT confuse with CUDA stream interference (Case 5) — that shows periodic gaps across ALL tasks, not task-specific gaps on reduction tasks"
  - "The cudaDeviceSynchronize calls were retained in the fix but have negligible impact because Realm's hijack redirects them to stream-level synchronization"

fix:
  primary: |
    Change the Regent compiler to use `DeferredBuffer` (or `DeferredReduction`)
    for scalar reductions on GPUs. A fixup kernel reads values and performs the
    final reduction on the GPU itself, writing the scalar result to zero-copy
    memory. The key principle is that no GPU task should call
    `cudaDeviceSynchronize` or blocking `cudaMemcpy` during its execution.
    (This was a compiler-level fix by Wonchan Lee, not an application-level
    change.)

  alternatives: |
    - If the Regent compiler fix is not available, manually restructure
      reduction tasks to avoid host-side scalar computation. Consider keeping
      reduction results on-device.
    - For cuPyNumeric: the runtime handles reductions internally, but ensure
      reduction tasks use GPU variants with asynchronous completion.
    - For custom Legion C++ code: use Realm async copy APIs instead of blocking
      CUDA calls.

  what_not_to_do: |
    Do NOT assume all GPU gaps near reduction tasks are this pattern — check
    whether the CPU is actually computing during the gap. If the CPU is idle or
    blocked, the cause is different (Cause 4 or Cause 7). Do NOT add
    `-cuda:legacysync` for this pattern — the issue is in the Regent code
    generator's reduction handling, not in CUDA stream interference. Legacy
    sync would not help and may hurt overall throughput.

verification: |
  After switching to DeferredBuffer/DeferredReduction, the CPU processor row
  should no longer show active computation overlapping GPU gaps for those
  reduction tasks. The GPU task's wait_ratio should drop significantly. GPU
  timeline gaps during reduction tasks are eliminated. 2–3× improvement in
  scalar reduction performance expected. Re-run the profiler and confirm on
  multi-GPU configurations that GPU contention is resolved.

real_cases:
  - case: "Legion issue #440"
    app: "Soleil-X (Regent CUDA code generator)"
    scale: "LLNL Lassen"
    result: "2–3× improvement in scalar reduction performance"
    key_detail: "Root cause was in generate_reduction_postamble in cudahelper.t — cudaDeviceSynchronize + blocking cudaMemcpy(DeviceToHost) + serial host-side loop"
  - case: "GitHub legion#440"
    app: "Soleil-X (multi-physics solver)"
    scale: "Multi-GPU on Sherlock and Lassen"
    result: "2–3× improvement in scalar reduction performance"
    key_detail: "Only visible on faster GPUs; invisible on older Sapling cluster GPUs"

related_patterns:
  - "explicit_sync_calls"
  - "blocking_python_operations"
  - "dg_legion_gpu_thread_interference"
  - "dynamic_tracing_missing"
```

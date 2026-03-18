id: insufficient_memory_allocation
title: Insufficient -ll:csize or -ll:fsize causes runtime assertion failures
source: Memory kinds and placement decisions section; Anti-pattern reference table
confidence: high
user_type: all

symptoms:
  what_you_see: |
    Runtime assertion failure with message: "Default mapper failed allocation of size X bytes for region requirement of inline mapping in task Y in memory Z." Application crashes rather than exhibiting a performance pattern.

  key_metrics: |
    Allocation failure message in runtime output. Crash occurs deterministically when working set exceeds allocated memory.

  distinguishing_features: |
    This is a crash/error, not a performance degradation pattern. Distinguished from other crashes by the specific allocation failure message referencing memory size, region requirement, task, and memory kind.

root_cause: |
  The memory allocation flags (-ll:csize for system memory, default 512 MB; -ll:fsize for GPU framebuffer, default 256 MB) are set below the application's working set. The runtime cannot create physical instances large enough to satisfy region requirements.

gotchas:
  - "Default sizes are conservative (512 MB system, 256 MB GPU framebuffer) and often insufficient for real applications."
  - "GitHub issue #1287 documents this as a common stumbling block."
  - "-ll:fsize must not exceed physical VRAM."
  - "Zero-copy memory (-ll:zsize, default 64 MB) is a shared allocation — oversubscribing it creates contention even if it doesn't crash."

fix:
  primary: |
    Increase memory flags: -ll:csize ≥ working set for system memory, -ll:fsize ≤ physical VRAM for GPU framebuffer. Also consider -ll:rsize for registered RDMA memory and -ll:zsize for zero-copy memory.

  alternatives: |
    Reduce working set per task by partitioning data more finely. Use out-of-core strategies with explicit data management.

  what_not_to_do: |
    Do NOT set -ll:fsize larger than physical VRAM. Do NOT ignore allocation failure messages — they indicate a fundamental sizing problem.

verification: |
  After increasing memory flags, allocation failures should disappear. Application should run to completion. Monitor memory utilization in Legion Prof to ensure adequate headroom.

real_cases:
  - case: "GitHub issue #1287"
    app: "[not specified]"
    scale: "[not specified]"
    result: "[crash resolved by increasing memory flags]"
    key_detail: "Common failure mode for new users with default memory settings."

related_patterns:
  - "gpu_data_in_system_mem"

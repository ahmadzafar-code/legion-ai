id: numa_unaware_mapping
title: NUMA-unaware OMP processor mapping causes 2× slowdown on bandwidth-bound tasks
source: Memory kinds and placement decisions section; Anti-pattern reference table
confidence: high
user_type: legion_cpp

symptoms:
  what_you_see: |
    Unexpectedly long task execution times on OMP processors despite no explicit copy operations in the channel view. Tasks run correctly but take roughly 2× longer than expected. No SYSTEM_MEM → GPU_FB_MEM copies (it's a CPU/OMP issue). Performance varies depending on which processor the task lands on.

  key_metrics: |
    2× performance loss for bandwidth-bound tasks. Tasks run on wrong socket relative to their data. No copy channel activity (NUMA misplacement manifests as slow execution, not explicit copies).

  distinguishing_features: |
    Unlike GPU memory misplacement (explicit copies visible in channels), NUMA misplacement shows NO explicit copies — the data is technically accessible but through remote NUMA interconnect at reduced bandwidth. Unlike task granularity issues (narrow task bars with gaps), tasks here are appropriately sized but individually slow.

root_cause: |
  The DefaultMapper's OMP processor handling discards NUMA locality information from slice_task, causing tasks to execute on processors with poor memory affinity. On multi-socket systems, accessing memory on a remote NUMA node has significantly higher latency and lower bandwidth, causing ~2× slowdown for bandwidth-bound workloads.

gotchas:
  - "This is a DefaultMapper-specific bug (GitHub issue #1140), not a fundamental Legion limitation."
  - "The symptom is subtle — tasks complete correctly but slowly, with no obvious copy overhead."
  - "Only manifests on multi-socket NUMA systems; single-socket systems are unaffected."
  - "Bandwidth-bound tasks are disproportionately affected; compute-bound tasks may not show significant degradation."

fix:
  primary: |
    Write a custom mapper that overrides slice_task to respect socket locality for OMP processors. Ensure the mapper uses SOCKET_MEM for NUMA-local memory placement.

  alternatives: |
    Use MemoryQuery with has_affinity_to(proc) to select NUMA-local memories. Pin tasks to specific sockets using processor constraints.

  what_not_to_do: |
    Do NOT rely on the DefaultMapper for NUMA-aware OMP task placement (known issue #1140). Do NOT assume all system memory has uniform access time on multi-socket systems.

verification: |
  After applying NUMA-aware mapping, task execution times should decrease by approximately 2× for bandwidth-bound tasks. Memory access patterns should be local to the socket where the task executes. Use hardware performance counters (e.g., perf) to verify NUMA-local memory access.

real_cases:
  - case: "GitHub issue #1140"
    app: "[not specified]"
    scale: "multi-socket NUMA systems"
    result: "2× performance loss due to NUMA misplacement"
    key_detail: "DefaultMapper discards NUMA locality from slice_task for OMP processors."

related_patterns:
  - "gpu_data_in_system_mem"
  - "default_mapper_complex_hierarchy"

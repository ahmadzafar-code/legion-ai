id: gpu_data_in_system_mem
title: GPU task data placed in SYSTEM_MEM forces PCIe copies before every GPU task
source: Memory kinds and placement decisions section; Anti-pattern reference table
confidence: medium
user_type: all

symptoms:
  what_you_see: |
    Large copy operations in Legion Prof's channel view showing SYSTEM_MEM → GPU_FB_MEM transfers before every GPU task execution. GPU processors show idle time waiting for data transfers to complete. Consistent host→device copy pattern repeating every iteration.

  key_metrics: |
    Channel shows SYSTEM→GPU_FB every iteration. GPU tasks consistently show host→device copies equal to their full working set. Copy bandwidth limited to PCIe rates (~12–32 GB/s) rather than framebuffer-local rates (~900+ GB/s). Performance 10–50× slower than framebuffer access for memory-bound tasks.

  distinguishing_features: |
    Unlike excess-field copies (too many fields transferred), the issue is the memory tier, not the volume. The channel view will specifically show SYSTEM_MEM as the source. Unlike NUMA misplacement (which shows as long task execution times without explicit copies), this shows explicit copy operations. The DefaultMapper correctly handles GPU placement — this is primarily a custom mapper issue.

root_cause: |
  The mapper selected SYSTEM_MEM or Z_COPY_MEM instead of GPU_FB_MEM for physical instances used by GPU tasks. Every GPU memory access then routes through PCIe (10–50× slower than framebuffer). Zero-copy memory (Z_COPY_MEM) is CPU+GPU addressable but PCIe-limited for GPU access. Custom mappers frequently make this mistake; the DefaultMapper handles it correctly.

gotchas:
  - "Z_COPY_MEM is GPU-addressable and may appear to 'work' but is PCIe-limited — it's a subtle performance trap that doesn't cause errors."
  - "The DefaultMapper handles GPU placement correctly — if you see this pattern with DefaultMapper, suspect a different issue."
  - "Insufficient -ll:fsize causes fallback to system memory — check for allocation failure warnings."

fix:
  primary: |
    Write custom mappers using MemoryQuery(machine).has_affinity_to(proc).best_affinity_to(proc) to select GPU_FB_MEM for GPU tasks. Set -ll:fsize ≤ physical VRAM to allocate sufficient framebuffer.

  alternatives: |
    Use GPU_MANAGED_MEM (-ll:msize) for hardware-coherent host/device memory on architectures that support it. For data that is read-once by GPU, Z_COPY_MEM may be acceptable to avoid a copy, but only if the access pattern is streaming.

  what_not_to_do: |
    Do NOT place frequently-accessed GPU data in SYSTEM_MEM or Z_COPY_MEM. Do NOT set -ll:fsize larger than physical VRAM. Do NOT assume Z_COPY_MEM is equivalent to GPU_FB_MEM for performance.

verification: |
  After fixing, SYSTEM_MEM → GPU_FB_MEM copies should disappear from steady-state iterations (copies on first iteration to populate framebuffer are expected). GPU task execution times should decrease. Channel activity should shift to GPU_FB_MEM-local operations.

real_cases:
  - case: "AutoMap (SC '23)"
    app: "[multiple]"
    scale: "multi-socket systems"
    result: "1.5× speedup from non-trivial memory placement (e.g., 9 collection arguments in Z_COPY_MEM)"
    key_detail: "Systematic exploration of mapping space revealed non-obvious optimal placements."

related_patterns:
  - "insufficient_memory_allocation"
  - "numa_unaware_mapping"
  - "default_mapper_complex_hierarchy"

id: hdf5_region_remap_superlinear
title: HDF5 attach/detach causing super-linear time growth from region unmap/remap
source: GitHub StanfordLegion/legion#984; Case 7
confidence: high
user_type: legion_cpp

symptoms:
  what_you_see: |
    Legion Prof shows the application CPU processor busy for 50–100 ms
    stretches without requesting the runtime to do additional work.
    Utility processors are NOT busy. Realm DMA channels are idle. With
    `-lg:unsafe_launch`, the application hangs. I/O iteration times grow
    super-linearly: 15s, 46s, 93s, 165s, 254s for successive iterations
    (vs. 4s each for direct HDF5 calls).

  key_metrics: |
    Application CPU utilization: 100% busy but not launching work.
    Utility processor utilization: low (idle). DMA channel utilization:
    idle. I/O iteration timing growth: super-linear (15s → 46s → 93s →
    165s → 254s). VTune hotspots: `check_region_dependence`,
    `PhysicalRegion::~PhysicalRegion`, `PhysicalRegion::is_mapped`.

  distinguishing_features: |
    Unlike dependence analysis overhead (Case 4), utility processors are
    IDLE — the overhead is on the application processor itself. Unlike
    the partition cache bug (Case 6), the overhead grows super-linearly
    (O(n²) or worse), not linearly. The `-lg:unsafe_launch` hang test
    is the key distinguisher: it confirms the runtime is unmapping and
    remapping regions, not blocked on a future.

root_cause: |
  Using Legion's HDF5 attach/acquire/release/detach integration caused
  the runtime to unmap and remap physical regions around every sub-task
  launch. VTune confirmed region lifecycle operations as the bottleneck.
  The super-linear growth pattern (O(n²) or worse) comes from dependence
  checking as regions accumulate — each new operation tests against all
  prior operations.

gotchas:
  - "The `-lg:unsafe_launch` hang test is diagnostic: if it hangs, the runtime is in the middle of unmapping regions. If it doesn't hang, the issue is elsewhere."
  - "VTune/perf is required to diagnose this — Legion Prof alone shows 'app processor busy' but doesn't explain why."
  - "The super-linear growth is the key signal — constant overhead would suggest a different root cause."

fix:
  primary: |
    Restructure the region mapping lifecycle: regions should be unmapped
    for long periods while launching sub-tasks, and only remapped when
    the application needs direct access. Avoid attach/acquire/release/
    detach cycles within tight loops.

  alternatives: |
    Use direct HDF5 I/O calls bypassing Legion's region integration for
    performance-critical I/O paths. The direct HDF5 baseline of 4s per
    iteration serves as the target.

  what_not_to_do: |
    Do NOT assume the growing iteration times are a memory leak or
    fragmentation issue. The super-linear growth comes from region
    dependence checking complexity, not memory management.

verification: |
  Issue closed as resolved. I/O iteration times should be constant
  (target: ~4s per iteration matching direct HDF5). Application CPU
  should spend time launching work rather than in region lifecycle
  operations.

real_cases:
  - case: "GitHub legion#984"
    app: "HDF5 I/O workload"
    scale: "Single node"
    result: "From super-linear growth (15s→254s) to constant ~4s target"
    key_detail: "Super-linear growth pattern was the key diagnostic; VTune confirmed region lifecycle as hotspot"

related_patterns:
  - visibility_algorithms_quadratic

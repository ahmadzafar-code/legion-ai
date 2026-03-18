id: visibility_algorithms_quadratic
title: O(n²) dependence analysis for aliased/overlapping regions
source: PPoPP 2023 paper (Bauer, Slaughter, Treichler, Lee, Garland, Aiken); Case 12
confidence: high
user_type: legion_cpp

symptoms:
  what_you_see: |
    Applications with complex partitioning schemes (e.g., ghost regions
    overlapping primary regions in unstructured mesh codes) show growing
    runtime overhead as the number of operations touching overlapping
    regions increases. Profiler shows increasing gaps between tasks as
    the program runs longer within an epoch. Dependence analysis time
    grows non-linearly.

  key_metrics: |
    Dependence analysis time as a function of operation count: non-linear
    growth. Utility processor time growing non-linearly over time.
    Profiler gaps between tasks increasing as the program runs longer
    within an epoch.

  distinguishing_features: |
    Unlike constant-overhead missing tracing (Case 4), the overhead HERE
    grows non-linearly within a single epoch/iteration — it gets worse
    the longer the program runs. The presence of complex aliased
    partitions (ghost regions, overlapping index spaces) is the key
    indicator. Distinguished from HDF5 region remap (Case 7) because
    utility processors ARE busy (not the application processor).

root_cause: |
  The original dependence analysis algorithm (painter's algorithm) was
  O(n²) in the number of operations touching overlapping regions. For
  each new operation, it tested against every prior operation in history
  for region overlap. Applications with complex aliasing patterns (common
  in real scientific codes) hit this quadratic wall.

gotchas:
  - "This only manifests with complex partition hierarchies — simple non-overlapping partitions won't trigger the O(n²) behavior."
  - "The benefit of the fix grows with application complexity — simple codes see no improvement because they never hit the quadratic wall."
  - "Legion 25.03.0 added KD-tree infrastructure for further asymptotic improvements beyond the PPoPP 2023 algorithms."

fix:
  primary: |
    Upgrade to a Legion version with improved visibility algorithms
    (post-PPoPP 2023). Three algorithms available: Painter's Algorithm
    (baseline), Warnock's Algorithm (recursive spatial subdivision), and
    Ray Casting. Version 25.03.0 added KD-tree infrastructure for
    further asymptotic improvements.

  alternatives: |
    Restructure partitions to minimize aliasing where possible. Reduce
    the number of overlapping regions or simplify the partition hierarchy.

  what_not_to_do: |
    Do NOT confuse this with constant per-task overhead (Case 4). If
    overhead grows over time within an epoch, it's the O(n²) dependence
    analysis, not missing tracing. Tracing won't help if the trace
    itself is expensive to analyze.

verification: |
  Performance improvements across Legion applications with complex
  partitioning, including unstructured mesh codes with ghost regions.
  The benefit grows with application complexity — measure dependence
  analysis time as a function of operation count and verify it no longer
  grows quadratically.

real_cases:
  - case: "PPoPP 2023 paper"
    app: "Unstructured mesh codes with ghost regions"
    scale: "Varies (benefit grows with partition complexity)"
    result: "Asymptotically better scaling for complex partition hierarchies"
    key_detail: "Three increasingly sophisticated visibility algorithms adapted from computer graphics"

related_patterns:
  - hdf5_region_remap_superlinear
  - dynamic_tracing_missing

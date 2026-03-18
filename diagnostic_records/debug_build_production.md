id: debug_build_production
title: Debug build flags in production cause order-of-magnitude slowdown
source:
  - "016 - Elliott Slaughter, Section 1: New Features"
  - Anti-pattern reference table; Legion Prof diagnostic methodology section
confidence: medium
user_type: all

symptoms:
  what_you_see: |
    A "nice bright red warning at the very top of the profile" in Legion
    Prof. Order-of-magnitude slowdown compared to expected performance.
    Every data access is checked. Profiling results are meaningless
    because the debug overhead dominates all timings.

  key_metrics: |
    - Red warning banner at top of Legion Prof output.
    - All task durations inflated relative to optimized builds.
    - DEBUG=1 in build. -DPRIVILEGE_CHECKS, -DBOUNDS_CHECKS, or -DLEGION_SPY flags present.
    - Order-of-magnitude performance difference vs. release build (10×+).

  distinguishing_features: |
    The red banner in Legion Prof is the definitive indicator. Unlike all
    other patterns (which show specific bottleneck signatures), this causes
    uniform slowdown across all operations. If the slowdown is uniform and
    extreme (10×+), suspect debug flags before investigating runtime
    anti-patterns. Always check for the red banner before starting any
    performance diagnosis.

root_cause: |
  The application was compiled or the runtime was built in debug mode,
  which enables assertions, extra safety checks (privilege checks, bounds
  checks, spy logging), and disables optimizations. These per-access
  runtime checks add significant overhead to every operation. Profiling
  this build produces misleading timing data that cannot be used for
  performance diagnosis.

gotchas:
  - "This should be the VERY FIRST thing you check in any profile. All subsequent diagnosis is meaningless if the build was debug mode."
  - "Always profile with DEBUG=0 and no debug flags — profiling a debug build gives meaningless results."
  - "Legion Prof setup explicitly requires: 'Always build with DEBUG=0 and strip all debug flags before profiling.'"
  - "Not all debug-mode builds produce this warning — it depends on the Legion version and build configuration. If in doubt, verify the build type independently."
  - "-DPRIVILEGE_CHECKS=1 is useful for verifying correctness after privilege changes but must be removed for performance measurement."

fix:
  primary: |
    Build with DEBUG=0 and strip all debug flags: -DPRIVILEGE_CHECKS,
    -DBOUNDS_CHECKS, -DLEGION_SPY. For Legion: build with
    CMAKE_BUILD_TYPE=Release or equivalent. Then re-profile.

  alternatives: |
    Use debug builds only for correctness verification, never for
    performance measurement. There is no way to extract valid performance
    data from a debug-mode profile.

  what_not_to_do: |
    Do NOT attempt to diagnose performance problems from a debug-mode
    profile. Do NOT mentally "subtract" the debug overhead — it affects
    different operations non-uniformly. Do NOT profile or benchmark with
    debug flags enabled. Do NOT report performance results from debug
    builds. Do NOT leave -DLEGION_SPY enabled in production.

verification: |
  After building with DEBUG=0, CMAKE_BUILD_TYPE=Release, and stripping
  all debug flags, the red warning banner should disappear, performance
  should improve by an order of magnitude, and all timings should be
  representative of production performance. Re-profile to identify actual
  runtime bottlenecks.

real_cases: []

related_patterns:
  - "[all other patterns — debug flags must be eliminated before diagnosing any other anti-pattern]"

  ```yaml

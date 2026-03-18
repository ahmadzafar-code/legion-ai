id: regent_pennant_compiler_cascade
title: Regent compiler cascade — 10,000× slowdown from flags, codegen, and cache blocking
source: SC 2015 paper (Slaughter et al.); Elliott Slaughter blog (elliottslaughter.com/2024/02/legion-paper-history); Case 1
confidence: high
user_type: regent

symptoms:
  what_you_see: |
    Sequential execution of Regent-compiled PENNANT is orders of magnitude
    slower than reference Fortran/C++ on a single core. No parallel
    overhead is involved — the problem is visible in purely sequential
    single-core runs. Timeline is irrelevant; this is a wall-clock-time
    comparison against a known reference.

  key_metrics: |
    Wall-clock time: ~10,000× slower than reference Fortran.
    Hardware performance counters: anomalously high L1/L2 cache miss rates
    in inner loops. Generated assembly quality far worse than reference.
    Phased breakdown: ~5× from missing -O2, ~100× from codegen bugs,
    ~1.5× from missing cache blocking.

  distinguishing_features: |
    This is a COMPILE-TIME / CODE-GENERATION problem, not a runtime
    scheduling or parallelism problem. Distinguished from runtime overhead
    patterns by the fact that it manifests on a single core with no task
    parallelism active. Hardware performance counters (cache misses, IPC)
    are the diagnostic instrument, not Legion Prof.

root_cause: |
  Three compounding problems: (1) Legion runtime compiled with DEBUG mode /
  -O0 instead of -O2 (5× penalty). (2) Numerous Regent compiler
  code-generation bugs producing unnecessary copies and redundant
  operations (~100× combined). (3) Missing cache-blocking optimization in
  the Regent PENNANT port causing L1/L2 cache thrashing in inner loops
  (~1.5× penalty). Multiplicative cascade: 5 × 100 × 1.5 ≈ 750–10,000×.

gotchas:
  - "The cache-blocking issue was initially dismissed as 'only affecting performance, not correctness' — but it accounted for the final 1.5× gap to parity."
  - "The 10,000× was a multiplicative cascade of three independent issues. Fixing any one alone would still leave enormous slowdowns from the other two."
  - "Do NOT confuse this with runtime overhead. If the problem manifests on a single core with no parallelism, it's a code-generation or compiler issue, not a scheduling issue."

fix:
  primary: |
    Enable compiler optimization flags: set `DEBUG=0` and compile with
    `-O2`. Fix Regent compiler code-generation bugs (this required six
    months of engineering effort). Implement cache-blocking optimization
    in the application port. Wonchan Lee additionally wrote a vectorizer
    for the Regent compiler.

  alternatives: |
    For quick diagnosis, compare generated assembly against reference code.
    Use hardware performance counters (perf stat, VTune) to identify cache
    miss hotspots before investigating runtime-level issues.

  what_not_to_do: |
    Do NOT investigate Legion Prof timelines or runtime overhead when the
    single-core sequential performance is the problem. Do NOT assume
    "close enough" after fixing one layer — the multiplicative nature
    means each remaining factor still causes large absolute slowdowns.

verification: |
  Regent PENNANT achieves performance comparable to hand-tuned Legion C++
  and MPI+X code on three benchmarks (PENNANT, mini-circuit, stencil).
  Multi-node distributed scaling succeeds.

real_cases:
  - case: "SC 2015 paper"
    app: "PENNANT (Lagrangian hydrodynamics)"
    scale: "Single core → multi-node"
    result: "From 10,000× slower to parity with Fortran and MPI+X"
    key_detail: "Six months of compiler bug fixes; vectorizer written in final two weeks before SC deadline"

related_patterns:
  - dynamic_tracing_missing
  - control_replication_bottleneck

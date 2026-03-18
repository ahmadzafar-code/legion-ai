id: critical_path_structural_coverage_gap
title: Critical path annotations missing on ~50% of profiled operations due to DAG architecture
source: criticalpath-research.md, all sections; GitHub Issue #481; Elliott Slaughter December 2024 Legion Retreat presentation
confidence: medium — documented architectural pattern with profiler internals analysis, no single diagnosed user case cited
user_type: all

symptoms:
  what_you_see: |
    In Legion Prof's timeline view, application tasks and copy/fill operations
    have critical path annotations (highlighting or dependency edges), but
    meta-tasks on utility processors, mapper calls, and runtime overhead entries
    have NO critical path information at all. Visually, roughly half the
    operations in the profile appear to be "invisible" to the critical path
    analysis — they have full timing data (spawn, create, ready, start, stop)
    but no critical path field. The critical path line appears to jump over
    or skip entire categories of work visible in the timeline.

  key_metrics: |
    - critical_path field = None/null on meta-tasks, mapper calls, runtime overhead entries
    - ~50% of profiled operations lack any critical path annotation
    - Application tasks and copy/fill operations DO have critical path data
    - All five TimeRange timestamps (spawn, create, ready, start, stop) are
      present on excluded operations — only the critical path field is absent

  distinguishing_features: |
    Unlike the dynamic-collectives missing flag pattern (critical_path_dynamic_collectives_missing_flag),
    this pattern occurs even with all correct flags set. The coverage gap is
    structural: the petgraph DAG in the Rust profiler (legion_prof_rs, state.rs)
    only models operations for which the C++ runtime emits event dependency
    (trigger) data. Meta-tasks, mapper calls, and runtime overhead never receive
    that data by design — they are timing-profiled but architecturally excluded
    from the application-level dependency graph. No flag or configuration change
    can include them.

root_cause: |
  The Rust profiler (legion_prof_rs) constructs a directed acyclic graph using
  the petgraph ^0.7 crate. Only operations with recorded event dependencies
  enter this DAG — specifically application tasks and data movement operations
  (copies, fills) whose triggering events are tracked by both the Legion
  runtime and Realm. Meta-tasks (dependence analysis, scheduling on utility
  processors), mapper calls (user-supplied mapper callbacks), and runtime
  overhead entries lack the event dependency structure needed for DAG edges.
  The critical path computation models the application's task/copy graph, not
  internal runtime scheduling. When the profiler encounters an operation type
  outside the DAG, its critical_path field is left as None (Rust Option type)
  rather than using a sentinel value. The operation still appears in output
  with all timing data — only the critical path annotation is absent.

gotchas:
  - "Users commonly assume ~50% coverage means a bug or missing flag — it is actually the expected structural boundary of the current implementation."
  - "The profiler does NOT skip excluded entries or insert placeholder values; operations appear with full timing data but null critical path. This can look like partial data corruption rather than an architectural limitation."
  - "The old Python profiler (legion_prof.py) also had critical path support but required BOTH -lg:prof AND -lg:spy log data and used a different visualization model (press 'a' in legacy HTML viewer). The Rust profiler's critical path is a different implementation, not a direct port."
  - "Even within the DAG, not all causal relationships are fully captured. The documentation acknowledges: bubbles can be caused by critical path dependencies OR pending data movement operations, and distinguishing these requires application-specific knowledge."
  - "'Improve rendering of critical paths' is listed as a near-term priority (December 2024 retreat), so the exact coverage boundary may change in future releases."

fix:
  primary: |
    No fix exists — this is a structural limitation, not a bug. The ~50%
    coverage is the expected behavior of the current critical path
    implementation. Users should interpret critical path results as covering
    only the application task and data movement layer, not runtime internals.

  alternatives: |
    - For understanding runtime overhead contribution to wall time, use
      utility processor utilization metrics and mapper call timing data
      separately from the critical path analysis.
    - For the most complete dependency picture, examine the source code in
      state.rs directly (clone the legion_prof_rs repository) to understand
      exactly which operation types enter the petgraph DAG and how edges
      are constructed — the public API docs do not expose internal critical
      path computation types.
    - Monitor Legion releases for improvements — "Improve rendering of
      critical paths" is a stated near-term priority as of December 2024.

  what_not_to_do: |
    Do NOT increase the -lg:prof <N> parameter expecting more critical path
    detail — N controls the number of nodes profiled, not a verbosity level.
    See pattern: prof_flag_verbosity_misconception.
    Do NOT assume missing critical path annotations indicate data corruption
    or incomplete profiling runs.

verification: |
  This is a known limitation, not a fixable issue. To confirm you are seeing
  the structural boundary (and not a different problem):
  1. Check that application tasks DO have critical path annotations — if
     even tasks lack them, a different issue is at play.
  2. Check that the operations missing critical path data are specifically
     meta-tasks, mapper calls, or runtime overhead entries.
  3. If using dynamic collectives, verify -lg:prof_all_critical_arrivals is
     set (see pattern: critical_path_dynamic_collectives_missing_flag).

real_cases:
  - case: "GitHub Issue #481 (Legion Prof Bug Fixes/Improvements)"
    app: "[general — not application-specific]"
    scale: "[N/A]"
    result: "[No resolution — issue opened February 2019 by Mike Bauer, still open. Originally listed critical path visualization as desired enhancement.]"
    key_detail: "The issue predates the Rust profiler's critical path implementation. Critical path was added to the Rust profiler between December 2022 and December 2024 without a dedicated release announcement."

related_patterns:
  - "prof_flag_verbosity_misconception"
  - "critical_path_dynamic_collectives_missing_flag"
  - "critical_path_data_movement_bubbles"

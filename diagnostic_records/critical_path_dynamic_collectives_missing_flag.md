id: critical_path_dynamic_collectives_missing_flag
title: Missing -lg:prof_all_critical_arrivals flag breaks critical path with dynamic collectives
source: criticalpath-research.md, section "The -lg:prof flag does not control profiling detail level"; legion_context.cc line 10547
confidence: medium — documented error condition with specific error code and source location, but no specific diagnosed user case cited
user_type: all

symptoms:
  what_you_see: |
    When running an application that uses dynamic collectives with Legion Prof
    critical path analysis enabled, one of two outcomes occurs:
    
    1. FATAL ERROR: The runtime emits fatal error 2020 with the message
       "Critical path analysis with dynamic collectives requires that you use
       the '-lg:prof_all_critical_arrivals' flag." — the application terminates.
    
    2. SILENT DEGRADATION: If the error is not triggered (conditions unclear
       from document), critical path coverage may be significantly reduced
       beyond the normal ~50% structural boundary because the runtime cannot
       track dependency chains through collective barriers. Operations on
       either side of a collective barrier lack the dependency edges needed
       to connect them in the petgraph DAG, creating additional gaps in
       critical path annotations.

  key_metrics: |
    - Fatal error 2020 from legion_context.cc line 10547
    - critical_path coverage potentially well below 50% (below normal structural baseline)
    - Application uses dynamic collectives (MPI-style collective barriers in Legion)
    - -lg:prof_all_critical_arrivals flag is NOT present on command line

  distinguishing_features: |
    Unlike the structural coverage gap (critical_path_structural_coverage_gap),
    this pattern is FIXABLE with a flag. The structural gap affects meta-tasks
    and mapper calls which can never have critical path data; this pattern
    affects application tasks and data movement that SHOULD have critical path
    data but can't because dependency chains through collectives are untracked.
    The fatal error 2020 is a definitive diagnostic — if you see it, this is
    unambiguously the problem. Without the error, distinguishing this from
    the structural gap requires checking whether application tasks near
    collective operations specifically lack critical path annotations.

root_cause: |
  Dynamic collectives in Legion create synchronization barriers where multiple
  operations contribute to and wait on a collective result. Without the
  -lg:prof_all_critical_arrivals flag, the runtime does not record the
  dependency information needed to trace critical paths through these barriers.
  The profiler's petgraph DAG cannot construct edges across collective
  boundaries, breaking the dependency chain. This is implemented as a check
  in legion_context.cc at line 10547 which can trigger fatal error 2020.

gotchas:
  - "Not all applications use dynamic collectives — this flag is irrelevant for applications that don't. Adding it unnecessarily may add profiling overhead without benefit."
  - "The fatal error 2020 is helpful when it fires, but it's unclear from the document whether all dynamic-collective scenarios trigger the error or whether some silently degrade coverage."
  - "This flag addresses only the collective-barrier gap. Even with it set, the structural ~50% gap for meta-tasks/mapper calls/runtime overhead remains."
  - "Users may conflate this fixable issue with the unfixable structural gap, or vice versa — both present as 'missing critical path data' but have very different solutions."

fix:
  primary: |
    Add -lg:prof_all_critical_arrivals to the application's command line
    alongside the existing -lg:prof <N> flag. This enables the runtime to
    track dependency chains through dynamic collective barriers, allowing
    the profiler to construct DAG edges across those synchronization points.

  alternatives: |
    If the application does not actually require dynamic collectives, removing
    their use eliminates the need for this flag entirely. However, this is
    an application architecture change, not a profiling configuration fix.

  what_not_to_do: |
    Do NOT increase -lg:prof <N> expecting it to fix this — N controls
    node count, not profiling detail. Do NOT assume this flag will raise
    coverage above the ~50% structural boundary — it only fixes the
    additional gap caused by untracked collectives.

verification: |
  After adding -lg:prof_all_critical_arrivals:
  1. The fatal error 2020 should no longer occur.
  2. Application tasks and copy/fill operations near collective barriers
     should now have critical path annotations where they previously
     did not.
  3. Overall critical path coverage should approach the normal ~50%
     structural baseline (meta-tasks and mapper calls will still lack
     annotations — that is expected).

real_cases:
  - case: "[No specific case cited]"
    app: "[N/A — any application using dynamic collectives]"
    scale: "[N/A]"
    result: "[N/A]"
    key_detail: "Error originates at legion_context.cc line 10547. Fatal error code is 2020."

related_patterns:
  - "critical_path_structural_coverage_gap"
  - "prof_flag_verbosity_misconception"

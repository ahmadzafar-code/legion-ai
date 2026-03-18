id: must_epoch_same_processor
title: Two must-epoch tasks mapped to the same processor cause fatal runtime error
source: Phase barriers and must-epoch launchers section; Anti-pattern reference table
confidence: medium
user_type: legion_cpp

symptoms:
  what_you_see: |
    Immediate runtime crash/fatal error. Application terminates during must-epoch mapping. No performance degradation — it's a hard failure.

  key_metrics: |
    Immediate crash. Fatal runtime error during map_must_epoch callback.

  distinguishing_features: |
    Unlike other mapping failures (which may produce warnings), this is an immediate fatal error. Unlike phase barrier deadlocks (which hang), this crashes. The error occurs during mapping, before any must-epoch tasks execute.

root_cause: |
  MustEpochLauncher guarantees that all contained tasks execute in parallel and can synchronize. The runtime verifies distinct processor assignment — if two tasks are mapped to the same processor, they cannot run simultaneously, violating the must-epoch contract.

gotchas:
  - "The map_must_epoch callback must ensure each task maps to a unique processor."
  - "All SIMULTANEOUS requirements in a must-epoch must share the same physical instance — different instances cause undefined behavior."
  - "Concurrent task barriers are not permitted in replicated tasks (error 628)."
  - "No deadlock prevention exists for multiple concurrent must-epoch launches overlapping on 2+ processors (GitHub issue #659)."

fix:
  primary: |
    In map_must_epoch, ensure each task maps to a unique processor. Verify that all SIMULTANEOUS coherence requirements share the same physical instance.

  alternatives: |
    Reduce the number of tasks in the must-epoch to match available processors. Restructure to avoid must-epoch if possible.

  what_not_to_do: |
    Do NOT map two must-epoch tasks to the same processor. Do NOT use different physical instances for SIMULTANEOUS regions within a must-epoch.

verification: |
  After fixing, must-epoch should execute without runtime errors. All tasks should run in parallel on distinct processors. SIMULTANEOUS regions should use shared physical instances.

real_cases:
  - case: "GitHub issue #659"
    app: "[not specified]"
    scale: "[not specified]"
    result: "[documents deadlock potential with multiple concurrent must-epoch launches]"
    key_detail: "No deadlock prevention for overlapping must-epoch launches on 2+ processors."

related_patterns:
  - "simultaneous_without_barriers"
  - "phase_barrier_participant_mismatch"

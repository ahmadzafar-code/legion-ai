id: phase_barrier_generation_exhaustion
title: Phase barrier generation exhaustion after ~2^32 generations causes silent failure
source: Phase barriers and must-epoch launchers section; Anti-pattern reference table
confidence: medium
user_type: legion_cpp

symptoms:
  what_you_see: |
    Silent barrier failure after many iterations. exists() returns false on a previously valid barrier. Application may hang, produce incorrect results, or crash after running correctly for a long time.

  key_metrics: |
    ~2^32 generations exhausted. exists() returns false on barrier. Failure occurs after extended execution (many iterations).

  distinguishing_features: |
    Unlike participant mismatch (immediate or near-immediate deadlock/premature firing), this pattern manifests only after prolonged execution (~2^32 barrier advances). The application works correctly for a long time before failing. The diagnostic is checking exists() return value.

root_cause: |
  Phase barrier generations are stored in a finite-width counter (~2^32). After exhausting all generations through advance_phase_barrier calls, the barrier becomes invalid. This is a fundamental limitation of the barrier implementation.

gotchas:
  - "This only manifests in very long-running applications with many iterations — it may not appear in testing."
  - "The failure is silent — no runtime error, just exists() returning false."
  - "The application must proactively track generation count and create new barriers before exhaustion."

fix:
  primary: |
    Track the generation count of phase barriers and create new replacement barriers before reaching ~2^32 generations. Swap the old barrier for the new one in the application's barrier management code.

  alternatives: |
    For shorter-running applications, this may not be a concern. Document the maximum iteration count as a known limitation.

  what_not_to_do: |
    Do NOT assume barriers last forever. Do NOT ignore exists() return values. Do NOT wait for failure to occur before implementing generation tracking.

verification: |
  After implementing generation tracking with proactive barrier recreation, exists() should always return true for active barriers. Application should run indefinitely without barrier-related failures.

real_cases: []

related_patterns:
  - "phase_barrier_participant_mismatch"
  - "simultaneous_without_barriers"

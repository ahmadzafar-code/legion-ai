id: simultaneous_without_barriers
title: SIMULTANEOUS coherence used without phase barrier synchronization
source: Region requirements, privileges, and coherence modes section; Phase barriers and must-epoch launchers section
confidence: medium
user_type: legion_cpp

symptoms:
  what_you_see: |
    Correctness failures (data races, undefined results) rather than a performance pattern. If tasks deadlock due to barrier issues, long colored bars in "waiting" state appear on processor timelines. Mapping failures may occur if the mapper does not reuse the existing instance (the copy restriction requires only one physical instance for SIMULTANEOUS coherence).

  key_metrics: |
    Correctness failures, not performance metrics. Potential mapping failures or runtime errors if the copy restriction is violated.

  distinguishing_features: |
    This is primarily a correctness anti-pattern that can also manifest as performance issues (deadlocks appearing as indefinite waits). Unlike ATOMIC coherence issues (which involve multiple physical instances), SIMULTANEOUS requires exactly one physical instance with explicit synchronization.

root_cause: |
  SIMULTANEOUS coherence relaxes the runtime's usual serialization guarantees, allowing multiple tasks to access the same physical instance concurrently. Without phase barriers (AcquireLauncher/ReleaseLauncher) to synchronize access, data races occur. The copy restriction means the mapper must reuse the single existing physical instance — creating a new one causes mapping failures.

gotchas:
  - "SIMULTANEOUS coherence has a copy restriction: only one physical instance is permitted. The mapper MUST reuse the existing instance, not create a new one."
  - "ATOMIC coherence is different: it requires serializability without strict ordering, but if two atomic tasks are mapped to different physical instances, behavior is undefined."
  - "Phase barriers are lightweight producer-consumer primitives, not traditional barriers — misunderstanding their semantics leads to incorrect synchronization."

fix:
  primary: |
    Always pair SIMULTANEOUS coherence with AcquireLauncher/ReleaseLauncher and phase barriers. Create phase barriers with create_phase_barrier(ctx, participant_count) matching the actual number of producers/consumers.

  alternatives: |
    For ghost-cell exchange patterns: create two phase barriers per direction (ready and empty), advance both each iteration, and use CopyLauncher with arrival/wait barriers for data movement.

  what_not_to_do: |
    Do NOT use SIMULTANEOUS coherence without explicit synchronization. Do NOT create multiple physical instances for SIMULTANEOUS regions. Do NOT mismatch participant counts (too many → barrier never fires; too few → premature firing causing data races).

verification: |
  Run with Legion Spy to verify correct synchronization. Phase barrier waits should complete in bounded time. No data races should appear. In Legion Prof, tasks should show coordinated execution with brief waiting periods for barrier synchronization, not indefinite waits.

real_cases: []

related_patterns:
  - "phase_barrier_participant_mismatch"
  - "phase_barrier_generation_exhaustion"
  - "must_epoch_same_processor"

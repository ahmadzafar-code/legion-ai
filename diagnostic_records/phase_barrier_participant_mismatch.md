id: phase_barrier_participant_mismatch
title: Mismatched phase barrier participant count causes deadlock or premature firing
source: Phase barriers and must-epoch launchers section; Anti-pattern reference table
confidence: medium
user_type: legion_cpp

symptoms:
  what_you_see: |
    Tasks stuck in waiting state indefinitely (deadlock) — visible as long colored bars in "waiting" state on processor timelines. Or: data races from premature barrier firing (too few participants). In the deadlock case, the application hangs and does not progress.

  key_metrics: |
    Tasks stuck in waiting state indefinitely. Or: correctness failures from premature barrier firing. No progress visible in Legion Prof after a certain point.

  distinguishing_features: |
    Unlike other deadlocks (must-epoch processor conflicts), this is caused by barrier semantics. Too many participants → barrier never fires (deadlock). Too few participants → premature firing (data race). The diagnostic is checking the participant count in create_phase_barrier against actual producers/consumers.

root_cause: |
  Phase barriers fire when the specified number of arrivals is reached. If the participant count exceeds the actual number of producers, the barrier never accumulates enough arrivals and deadlocks. If the count is too low, the barrier fires before all producers have completed, causing data races.

gotchas:
  - "The participant count must exactly match the number of tasks that will call arrive on the barrier."
  - "Changing the number of tasks in an iteration without updating barrier participant counts is a common source of this bug."
  - "No deadlock prevention exists for multiple concurrent must-epoch launches overlapping on 2+ processors (GitHub issue #659)."

fix:
  primary: |
    Set create_phase_barrier(ctx, participant_count) with participant_count exactly matching the number of tasks that will arrive on the barrier.

  alternatives: |
    Use dynamic barrier participant adjustment if the number of producers/consumers varies between iterations.

  what_not_to_do: |
    Do NOT hardcode participant counts if the number of participating tasks can change. Do NOT assume the runtime will detect mismatched counts — it cannot.

verification: |
  After correcting participant counts, barriers should fire in bounded time. No deadlocks or premature firings. Tasks should show brief waiting periods for synchronization, not indefinite waits.

real_cases: []

related_patterns:
  - "simultaneous_without_barriers"
  - "phase_barrier_generation_exhaustion"

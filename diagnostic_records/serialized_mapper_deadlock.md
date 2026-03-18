id: serialized_mapper_deadlock
title: Deadlock in serialized mapper when call blocks on runtime work
source: Transcript 003 (Scheduling and Mapper Calls), Transcript 002
confidence: medium
user_type: legion_cpp

symptoms:
  what_you_see: |
    Application hangs completely. In Legion Prof (if a partial profile can be captured),
    the utility processor shows a mapper call that never completes. No further mapper
    calls execute. Application processors are idle waiting for mapping.
    [INCOMPLETE — needs review: profiler may not capture hangs well]

  key_metrics: |
    - Application hung / no progress
    - Single mapper call with infinite duration
    - All other mapper calls queued behind it

  distinguishing_features: |
    Unlike a slow mapper call (which eventually completes), this is an infinite hang.
    Unlike a scheduler spin (which shows rapid calls), there is exactly one call that
    never returns. The root cause is a dependency cycle involving the serialization lock.

root_cause: |
  In serialized mapper mode, if a mapper call blocks waiting for runtime work that
  itself requires a mapper call on the same mapper, you get a deadlock: the blocked
  call holds the serialization lock, and the needed mapper call cannot acquire it.
  The runtime implements a pause/resume mechanism specifically to prevent this —
  the serializing mapper manager marks that the mapper call has "left" (releasing
  the lock) when it detects potential blocking. If this mechanism fails or the
  mapper blocks in a way the runtime doesn't expect, deadlock occurs.

gotchas:
  - "The runtime's pause/resume mechanism handles KNOWN blocking points (like instance creation). If your mapper blocks on something the runtime doesn't know about (e.g., a custom synchronization primitive), the pause won't trigger."
  - "The general discipline stated in lectures: 'never hold any internal runtime locks when calling the mapper' — but the serialization lock IS a mapper-level lock, not a runtime lock, so it has its own special handling."
  - "This is the same class of bug as holding runtime locks during mapper calls (mentioned by the instructor as a critical invariant to maintain)."

fix:
  primary: |
    Avoid blocking inside mapper calls on operations that might need the same mapper.
    If you must do blocking operations, ensure they go through the runtime's allocation
    APIs (which trigger the pause/resume mechanism) rather than custom synchronization.

  alternatives: |
    Switch to concurrent mapper mode (if thread-safe), which eliminates the
    serialization lock entirely. Or restructure the mapper to defer blocking
    operations using mapper events.

  what_not_to_do: |
    Do NOT use custom locks, condition variables, or blocking I/O inside mapper calls
    when using serialized mapper mode — the runtime cannot pause around operations
    it doesn't know about.

verification: |
  After fixing, the application should no longer hang. Mapper calls should complete
  in finite time. The pause/resume mechanism should be visible in detailed profiling
  as brief gaps in mapper call execution.

real_cases:
  - case: "[No specific case cited]"
    app: "[not specified]"
    scale: "[not specified]"
    result: "[not specified]"
    key_detail: "The instructor explicitly states 'make sure that you're never holding any runtime locks because that tends to go badly, especially if you've got contention and people's mapper calls take a really long time'"

related_patterns:
  - "serialized_mapper_bottleneck"
  - "expensive_mapper_calls"

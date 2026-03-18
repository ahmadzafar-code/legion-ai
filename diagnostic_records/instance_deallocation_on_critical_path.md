id: instance_deallocation_on_critical_path
title: Instance Allocation Blocked by Pending Deallocation
source: "017 - Michael Bauer, Section 3: Critical Path Categories Demonstrated"
confidence: medium
user_type: all

symptoms:
  what_you_see: |
    An instance in Legion Prof has its critical path marked YELLOW
    (not definitively on critical path, but suspicious). Following the
    annotation reveals the instance was "waiting for another instance
    to be deallocated in order to allocate this instance."

  key_metrics: |
    - Instance critical path color: yellow.
    - Critical path annotation: waiting on deallocation.
    - [INCOMPLETE — needs review] No specific duration thresholds given.

  distinguishing_features: |
    This is specifically about INSTANCE allocation, not task scheduling.
    The yellow color indicates Legion Prof "couldn't quite prove it's
    on the critical path" — it may or may not be the actual bottleneck.
    This is different from GPU memory pressure (which would show
    allocation failures) — here the allocation succeeds but is delayed.

root_cause: |
  Physical instances in Legion occupy memory. When memory is constrained,
  a new instance may not be allocatable until an old one is deallocated.
  If the deallocation depends on a task completing or a trace releasing
  instance handles, the allocation is blocked. This creates a chain:
  task completion → instance deallocation → new instance allocation →
  next task can proceed.

gotchas:
  - "Yellow critical path means 'maybe' — it's not proven to be the bottleneck. Investigate further before optimizing."
  - "Traces historically did not release instance handles when not being replayed, causing memory pressure. This was fixed ('Traces release instance handles when not being replayed, eligible for GC') but older Legion versions may still have this issue."
  - "Tension between tracing and memory: 'tracing does not like to free things. Whereas obviously some of this requires freeing things pretty aggressively' (Talk 015 - Colin Unger)."

fix:
  primary: |
    Investigate memory pressure:
    1. Is the memory capacity sufficient for the working set?
    2. Are old instances being held unnecessarily (by traces or by
       the application)?
    3. Upgrade Legion to a version where traces release instance
       handles when not being replayed.

  alternatives: |
    - Reduce working set size by restructuring data partitioning.
    - Use eager instance collection/deallocation policies if available.
    - If tracing is holding instances: evaluate the tracing-memory
      tradeoff (see tracing_memory_tension).

  what_not_to_do: |
    Do NOT over-optimize based on a yellow annotation alone — verify
    it's actually on the critical path before restructuring memory usage.

verification: |
  After addressing memory pressure, instance allocation delays should
  disappear from critical path annotations. If the fix was upgrading
  trace instance-handle release, check that instances are being GC'd
  between trace replays.

real_cases: []

related_patterns:
  - "tracing_memory_tension"
  - "critical_path_not_created_yet"

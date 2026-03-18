id: healthy_lazy_gc_memory_appearance
title: Memory appearing full is normal — Legion's lazy GC keeps invalid instances until needed
source: low_processor_utilization_diagnosis.md (Category 4); Legion Runtime anti-patterns reference (GC section); GitHub issue #1739
confidence: high
user_type: all

symptoms:
  what_you_see: |
    Memory timeline rows show utilization climbing to near-capacity and staying
    there. The memory appears "full" throughout the profile. Users may mistake
    this for a memory leak. However: no allocation failure errors occur, no
    "Malloc Instance" / "Free Instance" meta-task storms appear on utility
    processors, and task execution proceeds normally without stalls.

  key_metrics: |
    - Memory occupancy appears high (80-95%) but stable (not monotonically growing)
    - No allocation failure runtime errors
    - No "Malloc Instance" or "Free Instance" meta-task storms on utility processors
    - Task execution NOT stalled by allocation delays
    - No deferred allocation gaps (shaded regions on instance bars)

  distinguishing_features: |
    Unlike actual memory pressure (which causes allocation stalls, GC meta-task
    storms, and eventual OOM), lazy GC memory appearance shows high occupancy
    WITHOUT performance degradation. The critical distinction: high occupancy
    + normal execution = lazy GC (healthy). High occupancy + allocation stalls
    + GC storms = actual memory pressure (unhealthy). GitHub issue #1739
    documents this as a common source of user confusion.

root_cause: |
  This is not a problem. Legion's garbage collector is intentionally LAZY —
  it only frees invalid instances when memory pressure demands it. The
  design philosophy is "acquires fast, collections expensive." Invalid
  instances (no longer needed but not yet freed) remain in memory, making
  it appear full. The runtime will GC them when it needs the space.

gotchas:
  - "A proposed 'truly-in-use' memory line that would distinguish valid from invalid instances has been requested but is NOT yet implemented."
  - "Users frequently file bugs saying 'Legion has a memory leak' when they see this — it's the #1 GC-related support question."
  - "If memory grows monotonically AND the application crashes with OOM, that IS a real problem (see legate_gc_oom record). The distinction is whether execution proceeds normally or not."
  - "On GPU, framebuffer memory appearing full is especially alarming but equally normal. Check for -ll:fsize adequacy only if allocation failures occur."

fix:
  primary: |
    No fix needed. If the user is concerned, explain that Legion's GC is
    lazy by design and high occupancy without stalls is normal behavior.

  alternatives: |
    If the user wants to verify, check for: (1) allocation failure errors
    (if none → healthy), (2) GC meta-task storms on utility processors
    (if none → healthy), (3) deferred allocation gaps on instance bars
    (if none → healthy).

  what_not_to_do: |
    Do NOT diagnose high memory occupancy as memory pressure without
    evidence of allocation failures or GC storms.
    Do NOT recommend reducing memory usage when the profile shows normal
    execution with high occupancy.
    Do NOT confuse lazy GC behavior with memory leaks.

verification: |
  Healthy lazy GC: high occupancy + normal execution + no errors.
  Unhealthy memory pressure: high occupancy + stalls + GC storms + possible OOM.

real_cases:
  - case: "GitHub issue #1739"
    app: "[multiple — common user confusion]"
    scale: "Any"
    result: "Users frequently report 'memory leak' that is actually lazy GC"
    key_detail: "The proposed 'truly-in-use' memory metric is the long-term solution but is not yet implemented"

related_patterns:
  - "memory_pressure_instance_churn"
  - "legate_gc_oom"
  - "cupynumeric_eager_pool_exhaustion"

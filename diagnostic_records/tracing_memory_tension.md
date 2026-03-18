id: tracing_memory_tension
title: Tracing Conflicts With Aggressive Memory Reclamation
source: "015 - Colin Unger (FlexFlow); 002 - Michael Bauer (Tracing Improvements)"
confidence: medium
user_type: all

symptoms:
  what_you_see: |
    [INCOMPLETE — needs review] No specific profiler signature described.
    Likely manifests as memory pressure indicators (instance deallocation
    on critical path, out-of-memory errors) in applications that use
    tracing and also need to free memory aggressively. May see instances
    persisting longer than expected in the instance timeline.

  key_metrics: |
    - [INCOMPLETE — needs review] No specific metrics given.
    - Watch for: instances with long lifetimes, yellow critical path
      annotations on instance deallocations, or OOM conditions that
      only appear when tracing is enabled.

  distinguishing_features: |
    This pattern specifically affects applications that need both tracing
    (for runtime overhead reduction) AND aggressive memory management.
    It differs from general memory pressure because disabling tracing
    would alleviate the memory issue (but worsen runtime overhead).

root_cause: |
  Tracing captures a recorded sequence of operations including instance
  references. Historically, traces held instance handles even when not
  being replayed, preventing garbage collection. Even with the fix
  ("traces release instance handles when not being replayed"), there
  is inherent tension: the trace must remember which instances were used
  so it can replay correctly, which limits how aggressively instances
  can be freed. As Unger noted: "tracing does not like to free things.
  Whereas obviously some of this requires freeing things pretty
  aggressively."

gotchas:
  - "This is a fundamental design tension, not necessarily a bug. Tracing and memory reclamation have inherently competing goals."
  - "The fix for trace instance-handle release only applies when traces are 'not being replayed' — during replay, instances must still be held."
  - "Non-uniform traces (mixing traced and non-traced code) may help by limiting the scope of instance retention."

fix:
  primary: |
    Ensure Legion version includes the fix: "Traces release instance
    handles when not being replayed (eligible for GC)." Use non-uniform
    traces to limit the scope of instance retention to only the repeated
    sections.

  alternatives: |
    - Restructure computation to reduce peak instance count within
      traced regions.
    - Use explicit trace boundaries (BeginTrace/EndTrace) to keep
      traces small, reducing the number of instances held per trace.
    - Future: trace compilation may further optimize instance usage.

  what_not_to_do: |
    Do NOT disable tracing to fix memory pressure unless you've verified
    the runtime overhead without tracing is acceptable. The cure is
    likely worse than the disease.

verification: |
  After upgrading or adjusting trace scope, monitor instance lifetimes
  in Legion Prof. Instances should be GC'd promptly when traces are
  not being replayed. Memory high-water mark should decrease.

real_cases:
  - case: "Talk 015 - FlexFlow"
    app: "FlexFlow (LLM)"
    scale: "[not specified]"
    result: "[qualitative — noted as an ongoing challenge]"
    key_detail: "Colin Unger explicitly described the tension: combining memory optimization with tracing 'is not always the smoothest operation'"

related_patterns:
  - "instance_deallocation_on_critical_path"
  - "runtime_limited_no_tracing"

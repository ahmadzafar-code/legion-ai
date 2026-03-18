id: trace_template_memory_bloat
title: Physical trace template accumulation consumes excessive memory
source: Transcript 022 (Tracing Part 2)
confidence: medium
user_type: all

symptoms:
  what_you_see: |
    Memory usage grows over early iterations then stabilizes at a high level.
    Multiple distinct physical trace capture events appear in the profiler across
    early iterations. No single trace template is consistently replayed.
    [INCOMPLETE — needs review: specific memory metrics not provided]

  key_metrics: |
    - Multiple trace capture events across iterations
    - High memory consumption attributed to trace templates
    - Template count approaching the runtime bound (approximately 5-10)

  distinguishing_features: |
    Unlike a memory leak (unbounded growth), memory stabilizes once the template
    bound is reached. Unlike a single trace invalidation, MULTIPLE different
    templates are captured, indicating the mapper makes different decisions each time.

root_cause: |
  When mapper decisions keep changing, the runtime captures new physical trace
  templates for each unique set of mapping decisions. Each template is memory-intensive
  (it records all physical analysis decisions). The runtime bounds the number of
  templates (approximately 5-10) to prevent unbounded memory growth, but even
  this bounded set can consume significant memory. Once the bound is reached,
  old templates are evicted and must be recaptured if those mapping decisions
  recur.

gotchas:
  - "In earlier Legion versions, templates could grow without bound — this was found to be a memory problem in practice and the bound was added."
  - "The bound means some recapture overhead is ACCEPTED to avoid memory bloat — this is a deliberate tradeoff."
  - "If the mapper produces more than ~5-10 distinct mapping configurations, physical tracing will never stabilize and will continuously churn templates."

fix:
  primary: |
    Stabilize mapper decisions to reduce the number of distinct physical trace
    templates needed. Ideally, the mapper should converge to a single stable
    mapping configuration so only one template is captured and replayed.

  alternatives: |
    If mapper instability is inherent to the application (e.g., adaptive refinement),
    consider disabling physical tracing for those sections and relying on logical
    tracing only.

  what_not_to_do: |
    Do NOT attempt to increase the template bound — the bound exists because
    templates are memory-intensive and the real fix is mapper stability.

verification: |
  After stabilizing, only one or two trace capture events should appear (in the
  first iteration), followed by consistent replay in subsequent iterations.
  Memory consumption should be lower and stable from the start.

real_cases:
  - case: "[No specific case cited]"
    app: "[not specified]"
    scale: "[not specified]"
    result: "[not specified]"
    key_detail: "The runtime bounds template count to 'five or 10 or something like that' to avoid memory accumulation"

related_patterns:
  - "physical_trace_invalidation_by_mapper"

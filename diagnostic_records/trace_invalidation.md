id: trace_invalidation
title: Trace replay fails due to changed operations or unstable mapper decisions
source: Transcript 022 (Tracing Part 2), Transcript 021 (Tracing Part 1), Tracing and memoization section, Anti-pattern reference table
confidence: medium
user_type: all

symptoms:
  what_you_see: |
    In Legion Prof, no Replay Physical Trace tasks appear in iterations after the
    first — instead, physical trace capture keeps recurring. Utility processors show
    mapping calls in every iteration rather than just the first. The expected
    performance speedup from tracing does not materialize. In the logical-trace
    variant, runtime error 474 fires on the second iteration (first replay attempt)
    after a successful first iteration.

  key_metrics: |
    - Replay Physical Trace tasks absent in iterations 2+
    - Physical trace capture tasks appearing every iteration
    - Mapper calls present in every iteration (should only appear in iteration 1 with tracing)
    - No steady-state performance improvement over non-traced execution
    - Error 474 fires on second iteration (logical trace variant)
    - First iteration completes successfully; trace record succeeds but replay fails

  distinguishing_features: |
    Unlike missing tracing entirely (no trace-related tasks at all), here trace
    CAPTURE tasks appear — but replay never happens. Unlike logical trace issues
    (where the operation sequence changes), in the physical-trace variant the
    operation sequence is stable but the mapping decisions changed. When error 474
    is present, the failure is a correctness error on the SECOND iteration after a
    successful first iteration — error number 474 is the definitive diagnostic for
    the logical-trace variant.

root_cause: |
  Traces record the exact sequence of operations during the first iteration and
  replay them on subsequent iterations. If the operations issued between
  begin_trace and end_trace differ between record and replay (e.g.,
  data-dependent branching, different task counts), the trace replay encounters
  an inconsistency and fails with error 474.

  Physical tracing additionally captures the physical mapping decisions (which
  instances, which memories). If the mapper changes its decisions outside the
  trace boundary — e.g., different instance placements for precondition data —
  the input instances at trace replay time don't match those captured, and the
  trace cannot be replayed. The runtime falls back to recapturing silently.

  Logical tracing only requires the same operation sequence; physical tracing
  additionally requires stable mapping decisions.

gotchas:
  - "Logical tracing and physical tracing have DIFFERENT preconditions. Users who understand logical tracing may incorrectly assume physical tracing 'just works' too."
  - "All operations within a trace must be deterministic between iterations — no data-dependent branching, no variable-length loops."
  - "The mapper doesn't have to change decisions INSIDE the trace — changes OUTSIDE the trace (affecting input instances) are enough to invalidate physical replay."
  - "Begin/end trace are user PROMISES that the runtime trusts. Violating the trace invariant (different operations across replays) produces silently wrong results — there is insufficient checking code."
  - "The runtime bounds the number of physical trace templates (approximately 5-10) to avoid memory bloat. If the mapper keeps changing decisions, you'll see continuous recapture with high memory consumption for templates."
  - "Replication inside physical traces is not supported (warning 1117)."
  - "Traces cannot be nested."
  - "Automatic tracing (Apophenia) may detect and avoid problematic sequences, but manual traces with error 474 require code restructuring."

fix:
  primary: |
    Ensure all operations within begin_trace/end_trace are deterministic across
    iterations. Move any data-dependent control flow outside the traced region.
    For physical tracing, additionally ensure mapper decisions are STABLE across
    iterations both inside and outside trace boundaries. Use deterministic mapping
    heuristics and prefer instance reuse (find_or_create) which naturally produces
    stable placements.

  alternatives: |
    - Use separate trace IDs for different code paths. Split the iteration into
      traced (deterministic) and untraced (variable) sections.
    - If mapper instability is unavoidable, consider whether physical tracing
      provides benefit — the overhead of repeated capture may exceed the mapping
      cost being avoided. Logical tracing alone (which only requires stable
      operation sequences) may still provide benefit.

  what_not_to_do: |
    Do NOT assume physical trace replay is working just because you added begin/end
    trace calls. Always verify replay tasks appear in the profiler.
    Do NOT include data-dependent branching inside traced regions.
    Do NOT nest traces.
    Do NOT ignore error 474 — it indicates the trace is fundamentally incompatible
    with the code structure.
    Do NOT violate the trace invariant (launching different operations across replays)
    — the runtime currently trusts the user and will produce wrong results silently.

verification: |
  After fixing, Replay Physical Trace tasks should appear on utility processors
  starting from iteration 2. Mapper calls should disappear in steady-state
  iterations. Utility processor utilization should drop significantly after the
  first iteration. Error 474 should not occur. Traces should record on the first
  iteration and replay successfully on all subsequent iterations.

real_cases:
  - case: "[No specific case cited]"
    app: "[not specified]"
    scale: "[not specified]"
    result: "[not specified]"
    key_detail: "The instructor notes 'sometimes these templates are actually somewhat memory intensive' — the runtime bounds template count to 5-10 to prevent memory bloat"

related_patterns:
  - "trace_template_memory_bloat"
  - "missing_tracing"
```

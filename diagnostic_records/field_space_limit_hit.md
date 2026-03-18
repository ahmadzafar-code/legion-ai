id: field_space_limit_hit
title: Exceeding field space limit due to bitmask representation
source: Transcript 008 (Logical Dependence Analysis Part 2)
confidence: low
user_type: all

symptoms:
  what_you_see: |
    Runtime error when attempting to add more fields to a field space than the
    upper bound allows. No profiler signature — this is a compile/init-time error.

  key_metrics: |
    - Number of fields in a field space exceeding the runtime-defined limit
    - Error message about field space capacity

  distinguishing_features: |
    This is a hard error, not a performance issue. Distinguished from other allocation
    errors by the specific field space context.

root_cause: |
  Field spaces in Legion have an upper bound on the number of fields because fields
  are encoded as bitmasks for efficient field-parallel operations. The bitmask
  representation is a conscious performance tradeoff that limits the number of fields
  per field space.

gotchas:
  - "Users may not understand WHY the limit exists — the bitmask representation is an internal implementation detail."
  - "The limit is per field space, not global. You can use multiple field spaces if you need more fields."
  - "The exact limit depends on the bitmask width configured at compile time."

fix:
  primary: |
    Split fields across multiple field spaces if you need more fields than the
    per-field-space limit allows. Alternatively, redesign the data layout to use
    fewer fields (e.g., using structured types within fields).

  alternatives: |
    Recompile Legion with a larger bitmask width if the default is too small
    (at the cost of more memory per bitmask operation).

  what_not_to_do: |
    Do NOT try to work around the limit by dynamically creating/destroying fields —
    the bitmask representation is fundamental to the dependence analysis.

verification: |
  After restructuring, the runtime should accept the field space creation without
  errors. Verify that dependence analysis still correctly tracks all needed fields.

real_cases:
  - case: "[No specific case cited]"
    app: "[not specified]"
    scale: "[not specified]"
    result: "[not specified]"
    key_detail: "The instructor explains: 'this is why field spaces have an upper bound that Legion supports'"

related_patterns: []

---

## Summary

- Total records extracted: 14
- High confidence: 1 (scheduler_spin_no_deferral — real diagnosed historical bug)
- Medium confidence: 10 (documented patterns with partial profiler signatures)
- Low confidence: 3 (mentioned in passing or inferred from lecture context)
- Gaps identified:
  - **No Legion Prof session in this corpus**: The lectures mention a dedicated Legion Prof session (by Elliott Slaughter) that was NOT included in the 23 transcripts extracted. That session would likely contain the most valuable profiler-specific diagnostic patterns.
  - **No quantitative real cases**: None of the patterns cite specific GitHub issues, paper results, or quantitative measurements. All real_cases fields are empty or reference only the historical starvation bug anecdote.
  - **Profiler signatures are largely inferred**: The source material is runtime internals lectures, not profiling guides. Profiler task names, metric thresholds, and visual patterns are extrapolated from runtime behavior descriptions.
  - **Copy-fill aggregator ordering (Transcript 012, 013)**: Described as a critical correctness concern for read-only cases but no diagnostic pattern was created because the profiler signature and user-facing fix are not described — this is a runtime-internal concern.
  - **Distributed GC state machine (Transcript 005)**: The distributed collectable objects and downgrade owner election are described as "tricky" but without enough user-facing diagnostic information to create a record.
  - **Index space dual-purpose lifetime issues (Transcript 006)**: Described as creating complexity but without a user-visible diagnostic pattern.
  - **Epoch tracking in field-parallel mode (Transcript 014)**: Described as complex but without a user-facing diagnostic pattern.
  - **Tracing code quality warning (Transcript 021, 022)**: The code is explicitly called "messy" and "subject to change" — this is a development warning, not a diagnostic pattern, but is important context for anyone working on tracing-related issues.


## Source: Profiler Methodology

id: automatic_tracing_apophenia
title: Automatic trace detection (Apophenia) matched manual annotations
source: ASPLOS 2025 paper (Yadav, Bauer, Broman, Garland, Aiken, Kjolstad); Case 13
confidence: high
user_type: all

symptoms:
  what_you_see: |
    Complex applications (HTR solver, cuPyNumeric programs) suffer from
    runtime overhead at scale without manual trace annotations. No
    "Replay Physical Trace" meta-tasks appear in profiler output. Per-
    task overhead at the ~1 ms floor. Manual trace annotation is impractical
    for library-composed programs where trace boundaries span multiple
    independently-authored libraries.

  key_metrics: |
    Per-task overhead: ~1 ms without tracing. Presence vs. absence of
    "Replay Physical Trace" meta-tasks. Strong scaling curves degrading
    without trace memoization.

  distinguishing_features: |
    This is the same symptom as Case 4 (missing tracing), but the root
    cause is different: tracing IS desired but CANNOT be manually
    annotated because the program is composed of multiple independent
    libraries (e.g., cuPyNumeric + Legate Sparse). The fix is automatic,
    not manual annotation.

root_cause: |
  Manual tracing required programmer expertise to correctly identify
  trace boundaries. Missing or incorrect annotations meant no speedup
  from memoization. For library-composed programs like cuPyNumeric,
  manual annotation was impractical because trace boundaries span
  multiple independently-authored libraries.

gotchas:
  - "Apophenia achieves 0.92×–1.03× of manual tracing performance — occasionally slightly slower due to imperfect trace boundary detection."
  - "Available in Legion 25.03.0+ as 'Automatic Traces' — on by default."
  - "For previously untraced programs, speedups range widely from 0.91× to 2.82× — the 0.91× case indicates the trace detection overhead slightly exceeded the benefit for that particular workload."

fix:
  primary: |
    Upgrade to Legion 25.03.0 or later, where Apophenia (Automatic
    Traces) is enabled by default. No programmer annotation required.
    The system uses string mining algorithms on the sequence of runtime
    operations to identify repeating patterns and memoize their
    dependence analysis.

  alternatives: |
    For Legion < 25.03.0, use manual trace annotations via
    `runtime->begin_trace()` / `runtime->end_trace()` or `-dm:memoize`
    for DefaultMapper programs.

  what_not_to_do: |
    Do NOT spend time manually annotating trace boundaries in library-
    composed programs if you can upgrade to 25.03.0+. The automatic
    system handles cross-library boundaries that manual annotation cannot.

verification: |
  "Replay Physical Trace" tasks should appear on utility processors
  without any manual annotation. Performance: 0.92×–1.03× of manually
  traced programs. For previously untraced programs: 0.91×–2.82×
  speedups. Evaluated on HTR solver, CFD code, and cuPyNumeric on
  Perlmutter, Eos, and DGX H100 nodes.

real_cases:
  - case: "ASPLOS 2025 paper"
    app: "HTR solver, CFD code, cuPyNumeric applications"
    scale: "Perlmutter and Eos supercomputers, DGX H100 nodes"
    result: "0.92×–1.03× of manual tracing; 0.91×–2.82× for previously untraced"
    key_detail: "String mining algorithms on runtime operation sequences — no programmer annotation needed"

related_patterns:
  - dynamic_tracing_missing
  - circuit_missing_tracing_network
  - graph_compilation_metg

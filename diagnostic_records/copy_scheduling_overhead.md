id: copy_scheduling_overhead
title: Realm copy scheduling overhead dominates small copy operations
source:
  - "SC 2020 paper (Slaughter, Wu, Fu et al.) — Task Bench; Case 10"
  - "Copy operations and data movement overhead section"
confidence: high
user_type: all

symptoms:
  what_you_see: |
    Small copy operations take disproportionately long relative to their data size. Copy overhead visible in channel views as wide bars for small data transfers. High copy count with low per-copy data volume. Task Bench experiments show anomalously high METG values for Realm when communication payloads are small — the system cannot efficiently execute tasks shorter than ~1 ms despite theoretical ability to go lower.

  key_metrics: |
    METG(50%): the minimum task granularity at which 50% parallel efficiency is achieved. Realm METG anomalously high for small copy sizes. Per-copy setup cost dominant for small messages. Copy scheduling overhead >10× improved by subgraph API. Many small copies dominating overall transfer time. Per-copy overhead large relative to actual data transfer time. Runtime overhead varies by >5 orders of magnitude across 15 systems tested.

  distinguishing_features: |
    Unlike dependence analysis overhead (Case 4), this is in Realm's DMA subsystem, not in Legion's dependence analysis. The overhead is proportional to the NUMBER of copies, not the size of the task graph. Unlike excessive copy volume (too much data), the issue is per-copy scheduling overhead for many small copies. Unlike low -ll:bgwork (DMA serialization), the copies may execute concurrently but each has high fixed overhead. The pattern is many small, relatively slow copies rather than few large, slow copies. METG metric specifically isolates runtime overhead from application computation overhead.

root_cause: |
  Realm's DMA subsystem was optimized for large bulk copies but had disproportionately high overhead for small copies. The per-copy setup cost dominated for small messages, inflating the effective minimum task granularity. The subgraph API improved small copy overhead by >10×, but applications not using the optimized path still incur the legacy overhead.

gotchas:
  - "This is a Realm-level issue, not a Legion-level issue — Legion Prof may not directly show it. Task Bench's METG methodology was needed to isolate it."
  - "Chapel developers independently found the same pattern in their runtime via Task Bench and fixed it."
  - "Tracing can help by memoizing copy scheduling decisions across iterations."
  - "The subgraph API improvement (>10×) is available in newer Legion/Realm versions — ensure you're using a recent build."

fix:
  primary: |
    Enable tracing to memoize copy scheduling across iterations. Use a recent Legion/Realm version with the subgraph API optimization, which reduces per-copy setup overhead by >10×.

  alternatives: |
    At the application level, coarsen communication by batching small copies into fewer larger transfers where possible. Restructure data layouts to reduce copy count. Aggregate small transfers where possible.

  what_not_to_do: |
    Do NOT assume small-copy overhead is inherent to task-based runtimes. Task Bench showed >5 orders of magnitude variation across systems, meaning it's an implementation quality issue, not a fundamental limit. Do NOT assume small copies are free. Do NOT ignore per-copy scheduling overhead in performance analysis.

verification: |
  Improved METG by over an order of magnitude (>10×) for Realm. Chapel achieved 2× improvement. Both confirmed by respective system developers. After enabling tracing and/or using the subgraph API, per-copy overhead should decrease. Total copy time should decrease even if data volume remains the same.

real_cases:
  - case: "SC 2020 paper (Task Bench)"
    app: "Task Bench parameterized benchmark"
    scale: "Varying task granularities and communication patterns"
    result: ">10× METG improvement for Realm; 2× for Chapel"
    key_detail: "Task Bench's METG methodology was the key diagnostic — it isolated runtime overhead across 15 different systems"
  - case: "Task Bench Realm analysis"
    app: "Task Bench"
    scale: "[benchmarking]"
    result: "Subgraph API improved small copy overhead by >10×"
    key_detail: "Copy scheduling was identified as a major Realm bottleneck."

related_patterns:
  - graph_compilation_metg
  - dynamic_tracing_missing
  - missing_tracing
  - low_bgwork
```

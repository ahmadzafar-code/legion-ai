id: mapper_wrong_placement
title: Mapper places tasks on wrong processors causing unnecessary data movement
source: GPU differential diagnosis guide, Cause 8; Legion issue #1640 (Modified Circuit benchmark)
confidence: high
user_type: legion_cpp

symptoms:
  what_you_see: |
    Channel rows show copy operations during [T1, T2] that move data
    unnecessarily — the data already exists in a memory closer to where the task
    will execute. The GPU is idle waiting for these copies to complete. Unlike
    Cause 5 (genuine network congestion), the copies move data between memories
    on the SAME node or create redundant transfers between nodes when a local
    copy was available. Mapper call entries on utility processors may show
    `map_task` decisions that selected processors on different nodes from where
    the task's input regions reside.

  key_metrics: |
    - Channel rows show copy activity during [T1, T2]
    - Copies are intra-node or redundant cross-node (data already exists locally)
    - Total copy volume per iteration far exceeds theoretical minimum
    - `map_task` entries on utility processors select remote processors
    - Performance improves significantly with `-dm:same_address_space 1`

  distinguishing_features: |
    Unlike Cause 5 (network congestion), the copies are UNNECESSARY — data is
    being moved to a processor that didn't need it or already had a local copy.
    Cause 5 copies move data between different nodes as required by the algorithm.
    The key test: run `-dm:same_address_space 1` to constrain the mapper to local
    placement. If the gap disappears, Cause 8 is confirmed. The provenance
    tracking feature in Legion Prof (~2024) connects mapper decisions to their
    origins. The copy matrix analysis (`-C` flag) identifies unnecessary
    memory-pair traffic.

root_cause: |
  The mapper's placement decisions send tasks to processors that are remote from
  where the task's input data resides, causing unnecessary data movement for
  nearly every task. In issue #1640, the circuit mapper "randomly sprayed tasks
  across the machine in a completely incoherent fashion" because it couldn't
  handle the wrapper-task hierarchy pattern. Tasks in the second hierarchy level
  were mapped remotely from where they were sharded. This is subtle because
  mapper decisions are performance-only (cannot affect correctness) — the program
  produces correct results while running far slower than it should.

gotchas:
  - "The program produces CORRECT results despite the bad mapping — correctness does not indicate good performance"
  - "Issue #1640 had three co-occurring causes — bad mapper placement was only one; it co-occurred with missing tracing (Cause 3) and network congestion (Cause 5)"
  - "Provenance tracking (added ~2024) can directly identify which mapper calls produce bad placement decisions"
  - "The wrapper-task hierarchy pattern specifically confused the circuit mapper in issue #1640 — custom mappers that don't account for task hierarchy patterns can exhibit the same issue"

fix:
  primary: |
    Write a custom mapper that respects data locality — tasks should be mapped to
    processors on the same node where their input data resides. Use
    `-dm:same_address_space 1` as a quick constraint to verify the diagnosis.
    For index launches, ensure the sharding function aligns task indices with
    data partitioning. Add provenance metadata to trace which mapper calls
    produce which placement decisions.

  alternatives: |
    Use provenance tracking in Legion Prof to identify which specific `map_task`
    calls are producing bad decisions. Fix the sharding function for index
    launches to align with the data partition. Compare total copy volume against
    theoretical minimum to quantify the waste.

  what_not_to_do: |
    Do NOT assume channel activity means network congestion (Cause 5) — check
    whether the copies are necessary before trying to reduce communication volume.
    Do NOT leave `-dm:same_address_space 1` as a permanent fix — it constrains
    the mapper and prevents legitimate remote placement; it is a diagnostic tool,
    not a solution.

verification: |
  After fixing the mapper: total copy volume per iteration should approach the
  theoretical minimum. Intra-node and redundant cross-node copies should
  disappear. Running with and without `-dm:same_address_space 1` should produce
  similar performance (indicating the mapper is already placing tasks locally).
  The copy matrix analysis (`-C` flag) should show a cleaner traffic pattern.

real_cases:
  - case: "Legion issue #1640"
    app: "Modified Circuit benchmark"
    scale: "16–32 nodes on Perlmutter"
    result: "Part of multi-cause fix (one of three co-occurring causes)"
    key_detail: "Circuit mapper 'randomly sprayed tasks across the machine in a completely incoherent fashion' because it couldn't handle the wrapper-task hierarchy pattern"

related_patterns:
  - "network_congestion"
  - "missing_tracing"

---

## Summary
- Total records extracted: 8
- High confidence: 5 (real diagnosed cases with issue numbers and verification)
  - `scalar_reduction_blocking` — issue #440, Soleil-X, 2–3× improvement
  - `thread_oversubscription_stream_interference` — issue #1203, DG-Legion on Summit
  - `missing_tracing` — issue #1640, Modified Circuit, scaling failure
  - `network_congestion` — issue #1640, Modified Circuit, profiler auto-warning
  - `mapper_wrong_placement` — issue #1640, Modified Circuit, incoherent mapping
- Medium confidence: 3 (documented patterns with profiler signatures but no single dedicated issue)
  - `blocking_python_operations` — cuPyNumeric best practices, general pattern
  - `explicit_sync_calls` — issue #440 overlap + Jax-on-Realm analysis
  - `insufficient_parallelism` — profiling guide + Jax-on-Realm retreat presentation
- Low confidence: 0
- Gaps identified:
  - **DuckDB schema**: The document explicitly states column names are inferred and should be verified with `DESCRIBE` — no canonical schema documentation exists yet (feature added v25.06.0, ~8 months old)
  - **Apophenia details**: The automatic tracing system (v25.09.0, ASPLOS 2025) is referenced but its interaction with all eight causes is not fully explored — e.g., does it eliminate Cause 3 entirely for all application patterns?
  - **Jax-on-Realm quantitative results**: The retreat presentation is referenced for Causes 6 and 7 but no quantitative performance numbers are given
  - **Co-occurrence matrix**: Issue #1640 documents three co-occurring causes, and issue #1203 documents two — but the document does not provide a systematic treatment of which other cause combinations commonly co-occur
  - **Asynchronous CUDA task launch (v24.06.0)**: Referenced for Cause 6 but no real-case performance comparison with vs. without this feature is provided
  - **Provenance tracking (~2024)**: Referenced for Cause 8 diagnosis but no detailed usage instructions or example output are included


## Source: Anti-Patterns

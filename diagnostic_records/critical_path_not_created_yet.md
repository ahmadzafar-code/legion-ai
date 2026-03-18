id: critical_path_not_created_yet
title: Task Blocked by "Not Created Yet" — Trace Replay on Critical Path
source: "017 - Michael Bauer, Section 3: Critical Path Categories Demonstrated"
confidence: high
user_type: all

symptoms:
  what_you_see: |
    A task has a gap before it on the timeline. Clicking on the task
    reveals its critical path annotation says "not created yet." Following
    the critical path link leads to a "Replay Physical Trace" task on
    the utility processor — the trace replay had to finish before this
    task could be created.

  key_metrics: |
    - Critical path annotation: "not created yet" (red).
    - Blocking item: Replay Physical Trace task on utility processor.
    - Gap duration corresponds to the trace replay duration.

  distinguishing_features: |
    Unlike "previous task still running" (another critical path category),
    the blocking entity is not a data-producing task but the runtime's
    trace replay mechanism itself. Unlike runtime_limited_no_tracing,
    tracing IS enabled here — the issue is that trace replay itself is
    on the critical path, meaning the trace is large or complex enough
    to become a bottleneck.

root_cause: |
  When tracing is enabled, the runtime replays previously recorded traces
  to skip dependence analysis. However, the replay itself takes time and
  runs on a utility processor. If the replay is slow relative to task
  execution (e.g., many small tasks, large trace), the replay becomes
  the critical path item — tasks cannot be created until the replay
  catches up.

gotchas:
  - "This indicates tracing IS working but the trace replay itself is the bottleneck. Enabling tracing was the right move; the issue is trace EFFICIENCY."
  - "Future roadmap items include trace compilation/optimization and lowering traces to Realm graphs and CUDA graphs, which should reduce replay overhead."
  - "Do not disable tracing because of this — the no-tracing alternative is almost certainly worse (full dependence analysis)."

fix:
  primary: |
    Investigate trace complexity:
    1. Are traces capturing more work than necessary? Tighter trace
       boundaries may help.
    2. Are there many very small tasks? Increasing task granularity
       reduces the number of trace entries.
    3. Wait for trace compilation/optimization features (on the roadmap:
       "trace compilation optimization, support for predication, lowering
       to Realm graphs and CUDA graphs").

  alternatives: |
    - Increase task granularity to reduce per-task trace overhead.
    - If using automatic tracing, the detected traces may be suboptimal;
      explicit traces with tighter boundaries may be better.

  what_not_to_do: |
    Do NOT disable tracing — the "not created yet" gap from trace replay
    is almost always much shorter than what full dependence analysis
    would cost. Do NOT assume this is unfixable — trace compilation is
    on the roadmap.

verification: |
  After adjusting trace boundaries or increasing task granularity,
  the duration of Replay Physical Trace tasks should shrink. Critical
  path analysis should show a different bottleneck (ideally "previous
  task still running" = compute-bound).

real_cases:
  - case: "Talk 017 live demo"
    app: "[unnamed application]"
    scale: "[not specified]"
    result: "Demonstrated as one of the critical path categories"
    key_detail: "Replay Physical Trace on utility processor was the blocking item for task creation"

related_patterns:
  - "runtime_limited_no_tracing"

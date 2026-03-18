id: prof_flag_verbosity_misconception
title: Misinterpreting -lg:prof N as a profiling verbosity or detail level
source: criticalpath-research.md, section "The -lg:prof flag does not control profiling detail level"
confidence: medium — documented misconception with authoritative clarification, no specific diagnosed user case cited
user_type: all

symptoms:
  what_you_see: |
    User runs application with -lg:prof 2 (or higher N) expecting more detailed
    critical path data or additional profiling information compared to -lg:prof 1.
    The resulting profile has the same critical path coverage (~50%) regardless
    of the N value. On a multi-node run, increasing N may profile MORE NODES
    but does not add any additional per-operation detail or critical path
    annotations. The timeline view looks identical in terms of which operation
    types have critical path data.

  key_metrics: |
    - critical_path coverage unchanged between -lg:prof 1 and -lg:prof 2+
    - Number of profiled nodes changes with N (on multi-node runs)
    - No additional operation categories gain critical path annotations
    - Profile file size may increase (more nodes profiled) without more detail per node

  distinguishing_features: |
    Unlike the structural coverage gap (critical_path_structural_coverage_gap),
    this pattern is about user expectations, not profiler behavior. The profiler
    is working correctly — the user has misunderstood what the flag controls.
    Unlike missing -lg:prof_all_critical_arrivals, there is no error message
    and no change in which operation types are covered.

root_cause: |
  The -lg:prof <N> flag specifies the number of nodes to be profiled in a
  multi-node execution, not a detail or verbosity level. Setting N lower than
  the total node count simply profiles a subset of nodes. There is no
  documented -lg:prof "level 2" that enables additional critical path data.
  The documentation states: "run with -lg:prof <N> where N is the number of
  nodes to be profiled. (N can be less than the total number of nodes — this
  profiles a subset of the nodes.)"

  The misconception likely arises from the common convention in other tools
  where a numeric argument to a profiling/logging flag sets verbosity
  (e.g., -v 1 vs -v 2).

gotchas:
  - "On a single-node run, -lg:prof 1 and -lg:prof 2 produce identical output since there's only one node to profile — this can reinforce the misconception that 'it doesn't matter what you set.'"
  - "On multi-node runs, increasing N actually DOES change the output (more nodes profiled), which can mislead users into thinking 'higher N = more detail' when it really means 'more nodes covered.'"
  - "There is no -lg:prof 0 that disables profiling — the flag's presence enables profiling, and N controls scope."

fix:
  primary: |
    Understand that -lg:prof <N> sets the number of nodes profiled, not
    detail level. To profile all nodes, set N equal to the total node count.
    To reduce profiling overhead, set N lower than total nodes to profile a
    subset. Neither choice affects critical path coverage or per-operation
    detail.

  alternatives: |
    The only flag that specifically controls critical path data completeness
    is -lg:prof_all_critical_arrivals, which is required for applications
    using dynamic collectives. No other flag increases critical path coverage
    beyond the structural boundary.

  what_not_to_do: |
    Do NOT set -lg:prof to increasingly higher values expecting more detailed
    profiling — this just profiles more nodes and increases overhead/output
    size without adding per-operation detail. Do NOT search for undocumented
    "level 2" or "level 3" profiling modes.

verification: |
  Run the same application twice: once with -lg:prof 1 and once with
  -lg:prof <total_nodes>. On the same node's data, the critical path
  coverage percentage should be identical. The only difference is how
  many nodes have profiling data.

real_cases:
  - case: "[No specific case cited]"
    app: "[N/A]"
    scale: "[N/A]"
    result: "[N/A]"
    key_detail: "Document describes this as 'a common misconception' without citing a specific user or issue."

related_patterns:
  - "critical_path_structural_coverage_gap"
  - "critical_path_dynamic_collectives_missing_flag"

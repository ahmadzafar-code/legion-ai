id: unnecessary_collective_hints
title: False collective hints on index launches waste runtime checking overhead
source: Transcript 015 (Collective Views Part 1)
confidence: medium
user_type: legion_cpp

symptoms:
  what_you_see: |
    In Legion Prof, index launch operations take longer than expected. Additional
    checking/matching overhead is visible on utility processors during index launch
    processing. No actual collective optimizations occur despite the overhead.
    [INCOMPLETE — needs review: specific task names for collective matching not provided]

  key_metrics: |
    - Collective matching check time > 0 per index launch
    - Zero successful collective matches found
    - Index launch overhead proportional to number of point tasks

  distinguishing_features: |
    Unlike actual collective behavior (where matching succeeds and reduces communication),
    here the matching always fails. The overhead scales with index launch size — on
    large index launches this becomes significant.

root_cause: |
  When the mapper sets the collective hint on an index launch, the runtime checks
  every point task for potential matches between tasks using the same logical regions.
  If there is no actual collective behavior, all checks fail but the runtime still
  performs them. The mapper is allowed to be wrong (correctness is unaffected) but
  the wasted checking time accumulates, especially on large index launches.

gotchas:
  - "The mapper CAN be wrong about collective hints without correctness issues — but the performance penalty exists."
  - "The overhead scales with the number of point tasks in the index launch."
  - "This is easy to miss because there are no errors — just slightly slower index launches."

fix:
  primary: |
    Only set the collective hint in the mapper when the index launch actually has
    collective behavior (multiple point tasks accessing the same logical regions in
    compatible ways). Remove the hint for index launches without collective patterns.

  alternatives: |
    Profile with and without the collective hint to measure the actual overhead
    before deciding whether to remove it.

  what_not_to_do: |
    Do NOT set collective hints as a default/blanket policy on all index launches
    "just in case" — the checking overhead adds up.

verification: |
  After removing unnecessary hints, index launch processing time should decrease.
  No collective matching overhead should appear in the profiler for those launches.

real_cases:
  - case: "[No specific case cited]"
    app: "[not specified]"
    scale: "[not specified]"
    result: "[not specified]"
    key_detail: "The instructor says 'You may pay a little performance penalty for the runtime doing the check' — framing it as a non-trivial cost"

related_patterns: []

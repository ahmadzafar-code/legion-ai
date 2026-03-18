id: missing_write_discard
title: Missing WRITE_DISCARD for fully overwritten data causes unnecessary pre-copies
source: Copy operations and data movement overhead section; Anti-pattern reference table
confidence: medium
user_type: all

symptoms:
  what_you_see: |
    Copy channel active before write-only tasks. Unnecessary data movement visible in channel view as copies that bring stale data to the target memory before a task that will completely overwrite it. Copies appear on the critical path (press `a`).

  key_metrics: |
    Copy channel shows transfers before tasks that only write. Copy volume equals full region size even when no prior data is needed. Critical path includes copies that could be eliminated.

  distinguishing_features: |
    Unlike excess-field copies (too many fields), the correct fields are being copied but the entire copy is unnecessary because the task will overwrite everything. Unlike GPU memory misplacement (wrong memory tier), the memory tier is correct but the copy direction is wrong (bringing data TO a task that doesn't need the prior values).

root_cause: |
  Without WRITE_DISCARD, the runtime must assume the task needs the prior values and ensures the physical instance is up-to-date before the task runs. With WRITE_DISCARD, the runtime knows the task will completely overwrite the data and skips the pre-copy, saving bandwidth and latency.

gotchas:
  - "WRITE_DISCARD is only correct if the task overwrites EVERY element. Failing to do so leaves undefined data — a silent correctness bug."
  - "This is the same privilege correctness trap as READ_WRITE → WRITE_DISCARD conversion: must verify with -DPRIVILEGE_CHECKS=1."
  - "The performance benefit is largest for large regions where the pre-copy is expensive."

fix:
  primary: |
    Use WRITE_DISCARD privilege for any task that completely overwrites a region without reading prior values.

  alternatives: |
    If only a subset of elements is overwritten, WRITE_DISCARD is not applicable. Consider restructuring the task to overwrite all elements, or partition the region to isolate the overwritten subset.

  what_not_to_do: |
    Do NOT use WRITE_DISCARD unless the task overwrites every element. Run with -DPRIVILEGE_CHECKS=1 after making changes to verify correctness.

verification: |
  After applying WRITE_DISCARD, pre-task copies should disappear from the channel view for the affected region. Critical path should no longer include these eliminated copies. Task execution should begin sooner after the previous task completes.

real_cases: []

related_patterns:
  - "read_write_over_privilege"
  - "excess_fields_in_requirements"

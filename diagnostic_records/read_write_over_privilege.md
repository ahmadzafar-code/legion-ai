id: read_write_over_privilege
title: READ_WRITE privilege used when READ_ONLY or WRITE_DISCARD suffices
source: Region requirements, privileges, and coherence modes section; Anti-pattern reference table
confidence: medium
user_type: all

symptoms:
  what_you_see: |
    Sequential task chains on a single processor in Legion Prof's timeline view, with other processors sitting idle. The critical-path overlay (press `a`) traces directly through these serialized tasks. No copy activity on channels — the bottleneck is privilege-induced serialization, not data movement.

  key_metrics: |
    Processor utilization <50% while copy channels are idle. Tasks on the same region execute sequentially despite no data dependency requiring mutual exclusion.

  distinguishing_features: |
    Unlike excess-field copy overhead (which shows high channel utilization), this pattern shows idle channels AND idle processors. Unlike aliased-partition serialization, the tasks operate on the same region (not disjoint subregions). Legion Spy's dependence graph will show true dependence edges between tasks that could logically be concurrent.

root_cause: |
  Two tasks with READ_WRITE on the same region create a read-write/read-write conflict in the dependence table, forcing program-order serialization. The runtime must assume each task both reads and modifies the data, so no parallelism is possible. The three independence axes (region disjointness, field disjointness, privilege compatibility) all fail when both tasks declare READ_WRITE on overlapping regions and fields.

gotchas:
  - "Tasks may appear to need READ_WRITE because they modify a small subset of elements, but if another task only reads the region, the reader should use READ_ONLY to break the serialization."
  - "WRITE_DISCARD is more powerful than READ_WRITE for pure output tasks — it skips bringing data up to date — but failing to overwrite every element leaves undefined data, a silent correctness bug detectable only with -DPRIVILEGE_CHECKS=1 or Legion Spy."
  - "This pattern can be masked in small-scale runs where serialization overhead is hidden by other latencies; it becomes acute at scale."

fix:
  primary: |
    Downgrade READ_WRITE to READ_ONLY wherever the task does not modify the region. Use WRITE_DISCARD wherever the task completely overwrites the region without reading prior values.

  alternatives: |
    Split region requirements to isolate read-only fields from read-write fields using add_field(idx, FID). Use NO_ACCESS_FLAG for region requirements that exist solely for privilege passing (e.g., copy launchers). For reduction patterns, use REDUCE privilege with the appropriate reduction operator.

  what_not_to_do: |
    Do NOT blindly change READ_WRITE to WRITE_DISCARD without verifying every element is overwritten — this introduces silent correctness bugs that are difficult to detect without explicit checking flags. Do NOT assume adding more processors will fix serialization — the bottleneck is logical, not resource-based.

verification: |
  After downgrading privileges, Legion Prof should show previously serialized tasks running in parallel on separate processors. Processor utilization should increase. The critical-path overlay should no longer trace through these tasks sequentially. Run with -DPRIVILEGE_CHECKS=1 to verify correctness after any privilege changes.

real_cases:
  - case: "SC '14 structure slicing paper"
    app: "S3D"
    scale: "[not specified]"
    result: "3.68× speedup (combined field splitting + privilege optimization)"
    key_detail: "Moving only needed fields and correct privileges contributed to the combined speedup over monolithic region transfers."

related_patterns:
  - "excess_fields_in_requirements"
  - "missing_write_discard"
  - "aliased_partition_when_disjoint"

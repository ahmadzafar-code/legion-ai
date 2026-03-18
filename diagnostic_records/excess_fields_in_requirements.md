id: excess_fields_in_requirements
title: Too many fields requested in region requirements inflates copy volume
source: Region requirements, privileges, and coherence modes section
confidence: medium
user_type: legion_cpp

symptoms:
  what_you_see: |
    High channel utilization for copy operations between memories in Legion Prof's channel view. Copy operations are large relative to the actual data a task needs. Processors may show idle time waiting for copies to complete.

  key_metrics: |
    Total bytes transferred per task significantly exceeds the product of active_field_count × region_element_count × element_size. Channel utilization is high. Copy volume is disproportionate to useful data access.

  distinguishing_features: |
    Unlike privilege-induced serialization (idle channels, idle processors), this pattern shows BUSY channels with processors waiting. Unlike poor memory placement (system→GPU copies), the copies may be between appropriate memory tiers but are simply too large. The excess is visible by comparing bytes transferred against what the task actually reads/writes.

root_cause: |
  When a RegionRequirement names more fields than the task actually uses, the runtime must ensure all named fields are present in the target physical instance. This forces the runtime to move all declared fields even when only a subset is accessed, inflating copy volume proportionally to the number of excess fields.

gotchas:
  - "The runtime has no way to know which fields a task actually touches at runtime — it relies solely on the declared requirements. Over-declaring is always safe for correctness but harmful for performance."
  - "Structure slicing (field-level physical instances) only helps if the requirements are also split to name individual fields."
  - "A single region requirement with many fields forces them into the same physical instance, preventing field-level copy optimization."

fix:
  primary: |
    Split region requirements to name only the fields actually accessed using add_field(idx, FID) on separate RegionRequirement objects. Each requirement should contain only the fields the task will read or write.

  alternatives: |
    Use structure slicing (field-level physical instances) in the mapper to store fields in separate instances, combined with per-field region requirements.

  what_not_to_do: |
    Do NOT assume the runtime will optimize away unused field copies — it cannot. Do NOT group all fields in a single requirement for convenience.

verification: |
  After splitting requirements, Legion Prof channel views should show reduced copy volumes. Total bytes transferred per task should approach active_field_count × element_count × element_size. Channel utilization should decrease and processor idle time waiting on copies should shrink.

real_cases:
  - case: "SC '14 structure slicing paper"
    app: "S3D"
    scale: "[not specified]"
    result: "3.68× speedup contribution from moving only needed fields"
    key_detail: "Structure slicing was the key technique enabling field-level data movement optimization."

related_patterns:
  - "read_write_over_privilege"
  - "missing_write_discard"

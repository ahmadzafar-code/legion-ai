id: low_bgwork
title: Low -ll:bgwork for data-intensive apps causes DMA serialization
source: Copy operations and data movement overhead section; Anti-pattern reference table
confidence: medium
user_type: all

symptoms:
  what_you_see: |
    Copy channel shows sequential transfers when parallel DMA operations would be expected. Copy operations queue up and execute one at a time. Processors wait for copies to complete when multiple copies could overlap.

  key_metrics: |
    Copy channel shows sequential (non-overlapping) transfers. Channel utilization appears low despite many pending copies. DMA throughput below hardware capability.

  distinguishing_features: |
    Unlike missing WRITE_DISCARD (unnecessary copies), the copies here are necessary but serialized. Unlike excess fields (too much data per copy), individual copies are appropriately sized but cannot execute concurrently. The key indicator is sequential copy patterns in the channel view.

root_cause: |
  The -ll:bgwork flag controls the number of background worker threads that handle DMA operations in Realm. With insufficient background workers, DMA transfers serialize even when the hardware supports concurrent transfers. The default may be too low for data-intensive applications.

gotchas:
  - "This primarily affects applications with many independent copy operations that could overlap."
  - "Increasing -ll:bgwork beyond hardware DMA channel count provides no benefit."
  - "Must also ensure sufficient cores are available (don't combine with -ll:cpu = all cores)."

fix:
  primary: |
    Increase -ll:bgwork to 3–4 for data-intensive applications. Typical production configuration: -ll:cpu = physical_cores - 2, -ll:util 2, -ll:bgwork 3-4.

  alternatives: |
    For iterative applications, tracing eliminates repeated copy scheduling overhead. Use the mapper's postmap_task callback to pre-stage copies overlapping with computation.

  what_not_to_do: |
    Do NOT set -ll:bgwork higher than the number of independent DMA channels on the hardware. Do NOT increase -ll:bgwork without reserving cores (reduce -ll:cpu accordingly).

verification: |
  After increasing -ll:bgwork, copy channel should show overlapping (parallel) transfers. Copy throughput should increase. Processor idle time waiting on copies should decrease.

real_cases: []

related_patterns:
  - "cpu_uses_all_cores"
  - "low_utility_processors"

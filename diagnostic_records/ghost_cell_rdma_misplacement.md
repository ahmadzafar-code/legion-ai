id: ghost_cell_rdma_misplacement
title: Ghost-cell exchanges through SYSTEM_MEM instead of REGDMA_MEM add copy overhead
source: Copy operations and data movement overhead section
confidence: medium
user_type: legion_cpp

symptoms:
  what_you_see: |
    Ghost-cell exchange copy operations route through SYSTEM_MEM with additional PCIe traversal visible in channel views. Inter-node copy latency higher than expected. Channel view shows SYSTEM_MEM as intermediate stage in ghost transfers.

  key_metrics: |
    Extra memory copy and PCIe traversal per ghost exchange. Inter-node copy latency exceeds RDMA-capable transfers. Channel view shows SYSTEM_MEM involvement in distributed ghost exchanges.

  distinguishing_features: |
    Unlike GPU memory misplacement (local GPU access issue), this specifically affects distributed ghost-cell patterns. The signature is SYSTEM_MEM appearing as an intermediate in what should be direct RDMA transfers between nodes. GASNet's lack of GPUDirect support means inter-node GPU transfers must stage through host memory.

root_cause: |
  Ghost-cell exchanges that route through SYSTEM_MEM instead of REGDMA_MEM require an additional memory copy and PCIe traversal. REGDMA_MEM (-ll:rsize) is pinned memory enabling GASNet one-sided RDMA, bypassing the extra copy. Without allocating registered memory, the runtime falls back to unpinned system memory transfers.

gotchas:
  - "GASNet lacks GPUDirect support, so inter-node GPU transfers must stage through host memory regardless — REGDMA_MEM minimizes but cannot eliminate this staging."
  - "-ll:rsize defaults to 0 MB — registered memory must be explicitly allocated."
  - "MPI+GPUDirect can avoid host staging entirely, giving it a latency advantage over Legion for inter-node GPU communication."

fix:
  primary: |
    Allocate registered memory with -ll:rsize for frequently communicated ghost instances in distributed settings. Use REGDMA_MEM for ghost-cell physical instances in the mapper.

  alternatives: |
    For NVLink-connected GPUs, peer-to-peer transfers (~300–600 GB/s) bypass this issue entirely for intra-node communication.

  what_not_to_do: |
    Do NOT leave -ll:rsize at 0 for distributed applications with ghost-cell exchanges. Do NOT expect RDMA performance without pinned memory.

verification: |
  After allocating REGDMA_MEM, ghost-cell transfer latency should decrease. Channel view should show REGDMA_MEM as the source/destination for ghost transfers instead of SYSTEM_MEM.

real_cases: []

related_patterns:
  - "gpu_data_in_system_mem"
  - "low_bgwork"


## Source: Low Utilization

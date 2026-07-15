---
title: Region Tree Node
slug: region-tree-node
summary: Runtime data structure backing every application-visible region-tree handle (IndexSpaceNode, IndexPartitionNode, FieldSpaceNode, RegionTreeNode-base); a ValidDistributedCollectable that holds the per-handle state logical analysis walks.
tags: [data-model, dependence-analysis, memory, for-program-reasoning]
subsystem: legion
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/youtube_transcripts/runtime_school_2023/transcripts/006_Legion_Runtime_Internals_-_Lesson_6_-_Region_Tree_Nodes_and_Reference_Counting_Invariants.txt
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/region_tree.h
related:
  - wiki/concepts/region-tree.md
  - wiki/concepts/distributed-collectable.md
  - wiki/concepts/reference-counting-invariants.md
  - wiki/concepts/logical-region.md
  - wiki/concepts/partition.md
  - wiki/concepts/logical-analysis.md
---

## TL;DR
A region-tree node is the runtime's per-handle data structure for each application-visible region-tree object — `IndexSpaceNode` for an `IndexSpace`, `IndexPartitionNode` for an `IndexPartition`, `FieldSpaceNode` for a `FieldSpace`, the unified `RegionTreeNode` (and its `LogicalRegionNode` / `LogicalPartitionNode` subclasses) for the logical regions and partitions on top. All inherit from `ValidDistributedCollectable` (`distributed-collectable.md`) — they have the extra valid-reference state because logical analysis (`logical-analysis.md`) walks them while operations are in flight. The confusion: a region-tree node is **not** application-visible; the application holds an opaque handle and the runtime resolves it to a node via the `RegionTreeForest` (`region-tree.md`).

## Mental model
Region-tree nodes are the runtime's internal representation of every `logical-region.md`-related abstraction. When the application calls `runtime->forest->get_node(handle)`, the runtime returns one of these node objects, and the operation pipeline does its work over the tree of nodes. The valid-reference protection prevents nodes from being collected while logical analysis is still walking them.

## Mechanism & API
From `runtime/legion/region_tree.h` (per Runtime School Lesson 6):

- **`IndexTreeNode`** — common base for index-space and index-partition nodes; a `ValidDistributedCollectable`.
- **`IndexSpaceNode`** — represents an `index-space.md` handle. Stores the domain, child partitions, valid-instance bookkeeping.
- **`IndexPartitionNode`** — represents an `IndexPartition` handle. Stores parent, color space, children, disjointness/completeness flags.
- **`FieldSpaceNode`** — represents a `field-space.md` handle. Tracks allocated `FieldID`s, sizes, serializers.
- **`RegionTreeNode`** — base for logical-region-side nodes; holds the per-field logical state used by `logical-analysis.md`.
- **`LogicalRegionNode` / `LogicalPartitionNode`** — specific kinds of region-tree nodes.

The `RegionTreeForest` (one per `Runtime`, eventually destined to merge into `Runtime` per the Runtime School transcript) is the lookup entry point. Application handles resolve to node pointers via `get_node`.

**Why ValidDistributedCollectable matters here** (per Runtime School Lesson 6):
- Logical analysis (`logical-analysis.md`) walks region-tree nodes for every operation in stage 2 of the pipeline.
- A node might be the target of a `destroy_*` call (resource references drop) while an in-flight operation is still mid-walk (valid references nonzero).
- The valid-reference state keeps the node alive until the in-flight operation completes — without it, the operation would dereference freed memory.

**Distributed replication**: nodes are replicated to every node that touches their handle; the standard `distributed-collectable.md` machinery keeps counts consistent.

## Invariants
- One region-tree node per application-visible handle. Multiple `create_logical_region` calls with the same `(is, fs)` produce **distinct** nodes (different `tree_id`).
- Region-tree nodes are **distributed-collectable**: present on every node that touches them; reference-counted across processes.
- The **valid reference** state keeps a node alive while any in-flight operation might still need it (`reference-counting-invariants.md`).
- Partition children of an index-space node are organized by color; lookup is O(log #children) typical.
- Logical-region-side nodes and the corresponding index-space + field-space nodes share the same `tree_id` lineage.

## Performance implications
- Region-tree depth matters: deep hierarchical partitioning increases the number of nodes touched per operation in logical analysis.
- Multi-node runs replicate every touched node; a region used on 1000 nodes has 1000 replicas of its tree node.
- The `-lg:filter <N>` flag trims long per-node user lists during physical analysis to manage size.
- Most application code doesn't notice the cost of nodes — they're cheap relative to physical instances.

## Debug signals
- **`tools/legion_gc.py`** (with `-DLEGION_GC`) reports per-kind collectable accounting; many region-tree nodes leaked = a handle ownership bug.
- **Backtrace on a node-related assertion** (`LEGION_BACKTRACE=1`) usually points at handle misuse — destroying parent before child, using a destroyed handle.
- **`-level legion=2`** logs node creation and destruction events.

## Failure modes
- Destroying a parent (index space) while children (logical regions on it) still in use → caught by debug-mode assertions in most cases.
- Use-after-destroy of a handle → undefined behavior; manifests as assertion or crash.

## Source pointers
- **Header**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/region_tree.h
- **Lecture**: `raw/youtube_transcripts/runtime_school_2023/transcripts/006_..._Region_Tree_Nodes_and_Reference_Counting_Invariants.txt`

## Related
- `wiki/concepts/region-tree.md` — umbrella structure these nodes compose.
- `wiki/concepts/distributed-collectable.md` — base class.
- `wiki/concepts/reference-counting-invariants.md` — rules governing lifecycle.
- `wiki/concepts/logical-region.md` — the application-visible handle a node backs.
- `wiki/concepts/partition.md` — also backed by nodes.
- `wiki/concepts/logical-analysis.md` — the pass that walks these nodes.

---
title: Region Tree
slug: region-tree
summary: The persistent runtime data structure representing index spaces, field spaces, logical regions, and their hierarchical partitions; the substrate logical analysis walks.
tags: [data-model, dependence-analysis, for-program-reasoning]
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
  - wiki/concepts/logical-region.md
  - wiki/concepts/partition.md
  - wiki/concepts/logical-analysis.md
  - wiki/concepts/equivalence-set.md
  - wiki/concepts/region-tree-node.md
  - wiki/concepts/distributed-collectable.md
  - wiki/concepts/garbage-collection.md
  - wiki/concepts/reference-counting-invariants.md
---

## TL;DR
The region tree is the in-memory forest the runtime maintains for every index space, index partition, field space, and logical region the application creates. Each application handle (`IndexSpace`, `LogicalRegion`, `LogicalPartition`, …) corresponds to one node in the tree. Logical analysis (`logical-analysis.md`) walks these nodes to compute dependencies; physical analysis attaches equivalence-set state to them. The confusion: there isn't *one* tree — there's a "region tree forest" with separate trees for index spaces, field spaces, and the cross-product region trees they produce.

## Mental model
The region tree is to Legion's logical regions what a directory tree is to a filesystem: persistent metadata that exists alongside the data, holds parent-child relationships, and supports navigation queries. Application handles (`IndexSpace`, `LogicalRegion`, …) are like file descriptors — opaque names backed by a runtime node. Partitions create children; subregions are reached by traversing children.

## Mechanism & API
The relevant types live in `runtime/legion/region_tree.h`:
- **`RegionTreeForest`** — entry point class. `runtime->forest->get_node(handle)` returns the node for any handle. (Runtime School L6 notes this class is likely to merge into `Runtime` in the future.)
- **`IndexTreeNode`** — common base for index-space and index-partition nodes; a `DistributedCollectable` with the extra "valid reference" state used for GC.
- **`IndexSpaceNode`** — the runtime representation of an `IndexSpace`. Stores the domain, child partitions, valid-instance bookkeeping.
- **`IndexPartitionNode`** — the runtime representation of an `IndexPartition`. Stores parent, color space, children, and disjointness/completeness flags.
- **`FieldSpaceNode`** — represents a `FieldSpace`. Tracks allocated fields, serializers, distributed reference counts.
- **`RegionTreeNode`** — base class for logical-region and logical-partition nodes; stores the per-field logical state used by `logical-analysis.md`.

Lifecycle: nodes are created when the application calls `create_*` runtime methods; they're garbage-collected via the **distributed collectable** mechanism once no references remain. Multi-node runs replicate a node onto every process that touches its handle; the distributed collectable does reference counting across nodes (Runtime School L5 covers this in depth).

Most application code never touches region-tree types directly. They become visible when:
- Reading **Legion Spy**'s region-tree diagram (`legion_spy.py -d` + the region-tree output).
- Debugging GC issues with `-DLEGION_GC` and `tools/legion_gc.py`.
- Writing custom mappers that introspect the region tree (`runtime->get_index_space_domain`, `get_logical_subregion_by_color`, etc.).

## Invariants
- One node per application-visible handle. Multiple `create_logical_region` calls with the same `(index_space, field_space)` produce **distinct** region tree nodes (each with its own `tree_id`).
- Nodes are **distributed collectables**: present on every node that touches them; reference-counted across processes.
- `IndexTreeNode` carries the "valid reference" extra GC state to keep the region tree alive while *any* in-flight operation might still need it.
- Partition children of an index-space node are organized by color space; lookup by color is O(log #children) typical.
- Logical region nodes and their corresponding index-space + field-space nodes share the same tree-ID lineage so the region tree's cross product is self-consistent.

## Performance implications
- **Region-tree depth** matters: deep hierarchical partitioning increases logical-analysis cost per operation, since each region requirement walks the tree from the parent down to the touched subregion.
- Creating many regions with the same `(index_space, field_space)` (different `tree_id`s) is fine — they're cheap — but they have no aliasing relationship, which is the point.
- Distributed-collectable accounting is **per node touched**: a logical region used on every node of a 1000-node run has 1000 replicas of its tree node, each with reference counts.
- `-lg:filter <N>` trims long user lists held by region-tree nodes during physical analysis; useful for long-running workloads.

## Debug signals
- **Legion Spy region-tree output**: a `dot` graph of the index-tree forest and corresponding logical regions; great for seeing partition disjointness and field allocations at a glance.
- **`-DLEGION_GC`** + `tools/legion_gc.py`: traces region-tree node lifecycle; leaked nodes appear as never-collected.
- **Backtrace mode** (`LEGION_BACKTRACE=1`): assertions in `region_tree.cc` typically indicate handle misuse or premature destruction.

## Failure modes
- Destroying an index space or field space while a subordinate logical region is still in flight: usually caught by debug-mode assertions; in release builds may cause crashes during physical analysis.

## Source pointers
- **Header**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/region_tree.h
- **Implementation**: https://github.com/StanfordLegion/legion/tree/master/runtime/legion (`region_tree.cc`)
- **Lecture**: `raw/youtube_transcripts/runtime_school_2023/transcripts/006_..._Region_Tree_Nodes_and_Reference_Counting_Invariants.txt`
- **Related lecture (distributed collectables)**: `raw/youtube_transcripts/runtime_school_2023/transcripts/005_..._Distributed_Collectable_Objects.txt`

## Related
- `wiki/concepts/logical-region.md` — the application-level handle backed by a region-tree node.
- `wiki/concepts/partition.md` — partitions create new nodes.
- `wiki/concepts/logical-analysis.md` — walks the region tree to compute dependencies.
- `wiki/concepts/equivalence-set.md` — attached to region-tree nodes by physical analysis.

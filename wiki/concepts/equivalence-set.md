---
title: Equivalence Set
slug: equivalence-set
summary: The runtime's per-field unit of data lineage tracking; a maximal subset of points within a region whose dependence history and most-recent valid physical instances are jointly tracked.
tags: [data-model, dependence-analysis, instances, for-program-reasoning]
subsystem: legion
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/youtube_transcripts/runtime_school_2023/transcripts/009_Legion_Runtime_Internals_-_Lesson_9_-_Physical_Analysis_Part_1.txt
github:
  - https://github.com/StanfordLegion/legion/tree/master/runtime/legion
related:
  - wiki/concepts/physical-analysis.md
  - wiki/concepts/physical-instance.md
  - wiki/concepts/logical-region.md
  - wiki/concepts/partition.md
  - wiki/concepts/trace-recording.md
  - wiki/concepts/trace-replay.md
  - wiki/concepts/visibility-algorithm.md
  - wiki/concepts/collective-view.md
---

## TL;DR
An equivalence set is a runtime data structure that tracks, for some specific subset of points in some logical region, *which physical instances currently hold valid data per field, and what operations have written what*. Physical analysis (stage 5) finds the equivalence sets that cover its region requirement, queries them for valid instances, and registers the operation as a new user. The confusion: equivalence sets are not visible to the application — they're an internal partitioning of the region's points that exists to make physical analysis efficient. They split and merge as partitions evolve.

## Mental model
Equivalence sets are the runtime's *page table* for tracking which physical instances are current at each point of each region. Where a CPU's page table maps virtual addresses to physical pages, an equivalence set maps a `(field, point-subset)` to "which instances are valid here, and who wrote them last". When a new partition introduces a finer point set than any existing equivalence set, the affected equivalence sets are **split**; when two adjacent regions are accessed together, equivalence sets may **align**.

## Mechanism & API
From `runtime/legion/legion_analysis.h` (the file containing the implementation):
- A `VersionInfo` is the per-operation, per-region-requirement bundle of equivalence-set pointers, indexed by field mask.
- `perform_versioning_analysis(req, version_info)` (per the Lesson 9 transcript) is the asynchronous call that populates `VersionInfo` for one region requirement before the operation can run physical analysis.
- The runtime maintains the equivalence-set forest in the region-tree node for each region; new partitions trigger splits.

What an equivalence set stores per point-set:
- **Per field**, a set of physical instances currently valid for those points (the "valid instance set").
- A summary of recent users (which operations have read or written which fields), used to compute fine-grained per-point dependencies during stage 5.
- Reference counts for distributed-collectable lifecycle (see `runtime/legion/region_tree.h`).

A `VersionInfo` for a region requirement is conceptually:
```text
{ field_mask₁ → [eq_set_a, eq_set_b, ...],
  field_mask₂ → [eq_set_c, ...] }
```
i.e., the operation's fields are partitioned by which equivalence set covers each.

## Invariants
- An equivalence set's **point set is immutable until split**. A split creates two child sets whose union equals the parent.
- Each equivalence set tracks **per-field** valid instances: the same set can be valid for field X and stale for field Y.
- An operation that writes any field of any point in an equivalence set becomes the new "most recent writer" of that field for those points.
- Equivalence sets are **distributed-collectable**: multiple nodes can hold pointers to the same set; reference counting via the standard mechanism.
- `perform_versioning_analysis` is **asynchronous** — it can take time to materialize equivalence sets when splits are needed; the operation waits on the returned precondition event before running physical updates.

## Performance implications
- **Equivalence-set fragmentation is a real cost.** Highly hierarchical or many-aliased partitioning can fragment the equivalence-set forest, multiplying the lookup cost in physical analysis.
- The **versioning step shows up as utility-processor work** in Legion Prof prior to mapping each operation.
- `-lg:filter <N>` trims long user lists on equivalence sets at the cost of a slightly less precise dependence check; useful when memory pressure from user-list growth becomes significant.
- The visibility-algorithm paper (`visibility2023.pdf`) describes scalable algorithms for the per-field equivalence-set queries that dominate physical analysis at distributed scale.

## Debug signals
- **Heavy Legion Prof utility-row activity right before each operation's execution** = equivalence-set discovery is slow. Causes are usually fragmentation or stale instance lists.
- **`-DLEGION_GC`** + `tools/legion_gc.py`: trace equivalence-set lifecycles. Many short-lived ones = fragmentation.
- **`-level legion_analysis=2`**: logs versioning-analysis steps per operation.

## Failure modes
- Equivalence-set fragmentation contributes to [instance fragmentation](../pitfalls/instance-fragmentation.md) and [excessive data movement](../pitfalls/excessive-data-movement.md).

## Source pointers
- **Runtime (`legion_analysis.h`, `runtime.cc`)**: https://github.com/StanfordLegion/legion/tree/master/runtime/legion
- **Lectures**: `raw/youtube_transcripts/runtime_school_2023/` Lessons 9–14
- **Paper (visibility algorithms)**: `raw/publications/pdfs/visibility2023.pdf`

## Related
- `wiki/concepts/physical-analysis.md` — where equivalence sets are consumed.
- `wiki/concepts/physical-instance.md` — what equivalence sets track validity of.
- `wiki/concepts/logical-region.md` — the surface above the equivalence-set forest.
- `wiki/concepts/partition.md` — partitions cause equivalence-set splits.

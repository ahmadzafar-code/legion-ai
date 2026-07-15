---
title: Physical Analysis
slug: physical-analysis
summary: Pipeline stage 5; the per-point, post-mapping pass that finds the equivalence sets covering each region requirement, issues update copies, and emits the precise Realm event graph.
tags: [dependence-analysis, execution, instances, for-perf-debug]
subsystem: legion
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/youtube_transcripts/runtime_school_2023/transcripts/009_Legion_Runtime_Internals_-_Lesson_9_-_Physical_Analysis_Part_1.txt
  - raw/youtube_transcripts/runtime_school_2023/transcripts/010_Legion_Runtime_Internals_-_Lesson_10_-_Physical_Analysis_Part_2.txt
github:
  - https://github.com/StanfordLegion/legion/tree/master/runtime/legion
related:
  - wiki/concepts/operation-pipeline.md
  - wiki/concepts/dependence-analysis.md
  - wiki/concepts/logical-analysis.md
  - wiki/concepts/physical-instance.md
  - wiki/concepts/event.md
  - wiki/concepts/tracing.md
  - wiki/concepts/equivalence-set.md
  - wiki/concepts/dma-system.md
  - wiki/concepts/visibility-algorithm.md
  - wiki/concepts/event-graph.md
  - wiki/concepts/versioning.md
---

## TL;DR
Physical analysis is the second dependence-analysis pass — pipeline stage 5, after the mapper has chosen instances. It performs **versioning analysis** to find the **equivalence sets** that cover each region requirement, then issues the update copies, fills, and reductions needed to make the chosen instances valid for the operation. The output is a set of Realm `Event`s that the operation's execution will wait on. The confusion: physical analysis is where the **precise per-point dependence analysis** actually happens — logical analysis only sees operation-granularity.

## Mental model
If logical analysis is the front-end pass that says "task B depends on task A somewhere", physical analysis is the back-end pass that says "task B's point (3,4) reads field X at points (3,4); task A wrote field X at points (3..7); therefore B point (3,4) waits on A points (3,4)". The mapper between the two picks where the data lives; physical analysis figures out what copies and Realm events make that work.

## Mechanism & API
After stage 4 (mapping) completes, the runtime invokes `trigger_ready` on the operation, which leads to:

1. **Versioning analysis**:
   ```cpp
   runtime->perform_versioning_analysis(region_requirement, version_info, ...);
   ```
   The result is a `VersionInfo` containing a *field mask → equivalence-set pointer* table — the set of equivalence sets that, together, cover the points and fields the operation needs.

2. **Equivalence sets** (defined in `LegionAnalysis.h`):
   - An equivalence set names a maximal subset of points within a region that share a common dependence history (the same prior operations have written it).
   - Each equivalence set stores the most-recent valid physical instances per field for its point set.
   - Equivalence sets are constructed lazily as overlapping subregions get mapped; they split when a new partition introduces a finer point set.

3. **Physical update + registration**:
   ```cpp
   ApEvent done = op->physical_perform_updates_and_registration(version_info, ...);
   ```
   - Walks each equivalence set, checks whether the mapper's chosen instances are valid for its point set, and issues `IndexSpaceCopy`/`IndexSpaceFill`/`IndexSpaceReduce` operations to update them.
   - Registers the operation as the new most-recent user of those equivalence sets for its fields.
   - Returns an `ApEvent` indicating when the operation's preconditions are satisfied.

4. The execution stage (6) then spawns the Realm task with the precondition event.

Versioning analysis is **asynchronous**: it can take time to resolve when equivalence sets need to be created or migrated. The returned precondition events let the runtime keep moving while resolution finishes.

## Invariants
- Physical analysis runs **per operation, per region requirement** — but **per-point precisely**. The Realm event graph encodes the precise dependencies.
- An equivalence set's point set is **immutable until split**; splits happen when finer partitions force them.
- Equivalence sets track **per-field** valid instances; the same set can be valid for some fields and stale for others.
- Physical analysis only runs **after mapping** for that operation — its inputs are the instances the mapper chose.
- Multiple operations whose logical analysis marked them parallel can run physical analysis **concurrently**.

## Performance implications
- Cost dominated by `#equivalence sets visited × #fields × valid-instance lookup`. Hierarchical/dependent partitions can fragment the equivalence-set space, inflating this.
- The **copies physical analysis emits show up on Legion Prof channel rows**. Heavy channel-row activity = physical analysis decided current instances weren't valid and issued updates.
- **Tracing memoizes physical analysis** (specifically, "physical tracing" in `tracing.md`). With it, an iterative code reissues precomputed event chains instead of re-doing versioning analysis.
- **`-lg:filter <N>`** trims long instance user lists during physical analysis, reducing memory at the cost of additional dependence-checking work.
- The paper `visibility2023.pdf` describes scalable algorithms for the visibility queries underlying physical analysis at distributed scale.

## Debug signals
- **Legion Spy event graph** (`-lg:spy -e`): the post-physical-analysis structure. Nodes are operations + per-point fan-outs; edges are precise dependence events.
- **Heavy Legion Prof channel-row activity** despite stable mapping → physical analysis is issuing copies you didn't expect; investigate equivalence-set fragmentation or mapper-chosen instance mismatches.
- **`-level legion=2`** + filtering for "physical_perform_updates" lines shows per-op cost.
- **`-DLEGION_GC`** + `tools/legion_gc.py`: helps trace how equivalence sets and the instances they reference get collected.

## Failure modes
- [Excessive data movement](../pitfalls/excessive-data-movement.md) — physical analysis emits a copy on every entry because instances aren't being reused.
- [Instance fragmentation](../pitfalls/instance-fragmentation.md) — equivalence sets reference many short-lived instances.

## Source pointers
- **Runtime (legion_analysis.h, runtime.cc)**: https://github.com/StanfordLegion/legion/tree/master/runtime/legion
- **Lectures**: `raw/youtube_transcripts/runtime_school_2023/` Lessons 9–14
- **Paper (visibility algorithms)**: `raw/publications/pdfs/visibility2023.pdf`

## Related
- `wiki/concepts/operation-pipeline.md` — stage 5 is physical analysis.
- `wiki/concepts/dependence-analysis.md` — umbrella concept.
- `wiki/concepts/logical-analysis.md` — stage 2 counterpart.
- `wiki/concepts/physical-instance.md` — what physical analysis works on.
- `wiki/concepts/event.md` — the output of physical analysis.
- `wiki/concepts/tracing.md` — how this pass gets memoized.

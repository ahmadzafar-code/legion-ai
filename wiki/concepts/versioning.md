---
title: Versioning
slug: versioning
summary: The runtime's sub-pass of physical analysis that determines which equivalence sets cover an operation's region requirement, returned as a VersionInfo bundle; the asynchronous step that precedes physical updates.
tags: [dependence-analysis, instances, for-program-reasoning]
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
  - wiki/concepts/equivalence-set.md
  - wiki/concepts/visibility-algorithm.md
  - wiki/concepts/operation-pipeline.md
---

## TL;DR
Versioning is the first sub-step of `physical-analysis.md` (pipeline stage 5): given an operation's region requirement, the runtime walks the region tree to find the `equivalence-set.md`s that cover its points + fields, packages them into a `VersionInfo`, and returns it. The operation then uses this `VersionInfo` to drive the actual update + registration pass. Per Runtime School Lesson 9: it's the asynchronous "what equivalence sets do I touch?" lookup. The confusion: versioning is named like a version-control concept but it's about finding the *current most-recent valid views* for the operation, not about historical snapshots.

## Mental model
Versioning is the equivalence-set discovery step. Given a region requirement, the runtime determines which "buckets" of point-state the operation needs to read or update. The buckets are equivalence sets; the discovery walks the region tree. Once discovery is done, the operation has a precise per-field, per-equivalence-set picture of what it must check and what it must update.

## Mechanism & API
Per Runtime School Lesson 9:

The API is:
```cpp
op->perform_versioning_analysis(region_requirement, version_info, ...);
```

`VersionInfo` is the output — a per-field-mask table of equivalence-set pointers:
```text
{ field_mask₁ → [eq_set_a, eq_set_b, ...],
  field_mask₂ → [eq_set_c, ...] }
```

The call is **asynchronous**:
- The runtime may need to create or migrate equivalence sets to satisfy the request.
- The returned events are preconditions for `physical_perform_updates_and_registration`.
- Multiple operations can have their versioning analyses run in parallel — they're not blocked on each other unless they touch the same nodes.

**Inside the call** (from Lesson 9):
- The runtime walks the region tree downward from the requirement's region, collecting equivalence sets that cover the points + fields.
- If an equivalence set covers a strict subset of the points, the walk continues to find sibling sets.
- If a finer partition has been introduced since the last visit, equivalence sets are split here.

The `VersionInfo` is then consumed by `physical_perform_updates_and_registration` to:
- Test whether the mapper's chosen instances are valid for each equivalence set.
- Issue copies/fills/reductions to make them valid.
- Register the operation as a new user of the equivalence sets it touched.

## Invariants
- Versioning runs **once per operation per region requirement**, after the mapper has chosen instances.
- The returned `VersionInfo` is **valid only for this specific operation's analysis pass**.
- The walk is **asynchronous and parallelizable across operations** that don't share equivalence sets.
- Equivalence-set splits triggered by versioning are visible to all subsequent operations; the runtime amortizes the cost across users.
- A `VersionInfo` partitions the requirement's fields into groups; each group has its own equivalence-set covering.

## Performance implications
- **Versioning is one of the per-operation costs in physical analysis** — visible as utility-row activity in `legion-prof.md`.
- Cost scales with the depth and fragmentation of the equivalence-set forest. Aliased partitions and many-field workloads increase fragmentation.
- Tracing (`tracing.md`) memoizes the versioning result so subsequent traced replays skip it.
- For workloads where versioning is the bottleneck, the **visibility-algorithm.md** paper's techniques apply.

## Debug signals
- **Utility-row activity** in `legion-prof.md` proportional to operation count → versioning + adjacent physical-analysis steps. Mitigate with tracing.
- **`-level legion_analysis=2`** logs versioning steps per operation.
- **A `VersionInfo` with many small field masks** = field-fragmented equivalence sets; consolidate fields where possible.

## Failure modes
- Operation requesting fields that don't exist in the field space → caught earlier; not a versioning issue.
- Internal versioning errors → runtime bugs; report with `-level legion_analysis=2` logs.

## Source pointers
- **Runtime tree**: https://github.com/StanfordLegion/legion/tree/master/runtime/legion (`legion_analysis.cc`, `runtime.cc`)
- **Lecture**: `raw/youtube_transcripts/runtime_school_2023/transcripts/009_..._Physical_Analysis_Part_1.txt`
- **Paper (visibility algorithms)**: `raw/publications/pdfs/visibility2023.pdf`

## Related
- `wiki/concepts/physical-analysis.md` — host pass.
- `wiki/concepts/equivalence-set.md` — what versioning collects.
- `wiki/concepts/visibility-algorithm.md` — broader scalable-queries context.
- `wiki/concepts/operation-pipeline.md` — versioning is in stage 5.

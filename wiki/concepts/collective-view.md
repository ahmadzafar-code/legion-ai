---
title: Collective View
slug: collective-view
summary: A runtime data structure representing a set of physical instances all backing the same logical region across an index launch; deduplicates physical-analysis work for index-launch points that touch the same region.
tags: [data-model, dependence-analysis, instances, replication, for-perf-debug]
subsystem: legion
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/youtube_transcripts/runtime_school_2023/transcripts/015_Legion_Runtime_Internals_-_Lesson_16_-_Collective_Views_Part_1.txt
github:
  - https://github.com/StanfordLegion/legion/tree/master/runtime/legion
related:
  - wiki/concepts/equivalence-set.md
  - wiki/concepts/physical-analysis.md
  - wiki/concepts/physical-instance.md
  - wiki/concepts/control-replication.md
  - wiki/concepts/index-space-launch.md
---

## TL;DR
A collective view is a runtime data structure that groups together the physical instances backing the **same logical region across multiple points of an index launch**. Instead of every point task independently doing physical-analysis work for "its" instance, the runtime creates one collective view representing all the matching instances and analyzes once — deduplicating equivalence-set updates and enabling more efficient copies between collective views via topology-aware algorithms. Per Runtime School Lesson 16: this is the modern scalable approach. The confusion: collective views are an **optimization hint**. The application asks for it via the mapper; the runtime creates the rendezvous structure; if there's no actual collective behavior, you pay a small lookup cost but nothing breaks.

## Mental model
Collective views are the runtime's analog of MPI collective operations: instead of every rank issuing its own point-to-point exchange, one collective op represents the whole group. Where MPI's `MPI_Allreduce` is "every rank participates in one operation", Legion's collective view is "every point task that touches this region participates in one physical-analysis step".

## Mechanism & API
Per Runtime School Lesson 16:

**When collective views fire**:
- Multiple points of one `index-space-launch.md` touch the same logical region (a common case under control replication or aliased partitions).
- The **mapper hints** the runtime that collective behavior might be present (a tag or per-task flag).
- The runtime performs a **parallel rendezvous** across the points to find groups of points that match. Matching points get bundled into a collective view.

**Two kinds** (per Lesson 16):
- **Replicated views** — multiple points reading the same logical region but mapping to distinct physical instances (e.g., one per GPU); the collective view bundles all those replicas.
- **All-reduce views** — multiple points reducing into the same logical region; the runtime can use efficient tree-reduce algorithms across the collective view's members.

**What gets deduplicated**:
- Equivalence-set updates: one collective view writes the equivalence set instead of N points each writing.
- Copy-fill aggregation: copies between collective views use topology-aware algorithms (e.g., scatter from one source to N replicas) instead of N point-to-point copies.
- Discovery of preconditions: one rendezvous per collective view instead of N.

**What does not get deduplicated**:
- Per-point instance registration as a user. Each point task still registers itself as a user of its specific physical instance.

**Mapper hint**: the application's custom mapper can flag an index launch as "may have collective behavior". The runtime then performs the rendezvous step; if no collective behavior is found, it falls back to per-point analysis with a small overhead.

## Invariants
- Collective views are **internal to the runtime** — the application sees them only indirectly (faster analysis, better channel-row activity).
- Hinting at collective behavior **never affects correctness**, only performance. Wrong hints incur a small rendezvous cost but no semantic impact.
- A collective view's member instances may live in **different memories on different nodes**; the runtime handles cross-memory access through optimized DMA paths.
- Two kinds: replicated views (read-heavy / different physical instances of the same logical view) and all-reduce views (reduce-heavy).
- Used most heavily under `control-replication.md` where index launches naturally produce collective patterns.

## Performance implications
- **Significant scalability improvement** for index launches with many points touching shared regions — analysis cost drops from O(N) to O(1) for the deduplicated steps.
- The biggest wins are on **multi-node runs under control replication** where collective views replace point-to-point cross-shard exchanges with topology-aware collectives.
- See paper `visibility2023.pdf` for the algorithmic underpinnings.
- The runtime's rendezvous + collective-view-construction step itself has a cost; for very small index launches the overhead may exceed the savings.

## Debug signals
- **`-level legion_analysis=2`** logs collective-view creation and lookup events.
- **Legion Prof channel rows** under control replication: collective-view-aware copies appear as smaller bundled DMA bars instead of many small point-to-point bars.
- **`pitfalls/runtime-overhead-dominates.md`** at scale despite control replication → possibly collective views aren't firing; check the mapper's hints.

## Failure modes
- Index launches with no actual collective behavior under a "use collective views" hint → small rendezvous overhead, no correctness impact.
- Wrong rendezvous results (rare — runtime bug) → report with `-level legion_analysis=2` logs.

## Source pointers
- **Lecture**: `raw/youtube_transcripts/runtime_school_2023/transcripts/015_..._Collective_Views_Part_1.txt`.
- **Paper (visibility algorithms underlying collectives)**: `raw/publications/pdfs/visibility2023.pdf`.
- **Runtime tree**: https://github.com/StanfordLegion/legion/tree/master/runtime/legion

## Related
- `wiki/concepts/equivalence-set.md` — what collective views deduplicate updates to.
- `wiki/concepts/physical-analysis.md` — the host stage.
- `wiki/concepts/physical-instance.md` — what the view's members are.
- `wiki/concepts/control-replication.md` — the primary client of collective views.
- `wiki/concepts/index-space-launch.md` — the operation that produces collective patterns.

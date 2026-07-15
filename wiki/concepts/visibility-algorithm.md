---
title: Visibility Algorithm
slug: visibility-algorithm
summary: The scalable family of algorithms inside Legion's physical analysis (and distributed coherence) for computing which prior writers' effects are visible to a current operation's reads; reduced to the *visibility problem* from computer graphics, with Legion's production implementation using ray casting.
tags: [dependence-analysis, distributed, for-perf-debug, for-program-reasoning]
subsystem: legion
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/publications/pdfs/visibility2023.pdf
  - raw/publications/publications.md
github:
  - https://github.com/StanfordLegion/legion/tree/master/runtime/legion
related:
  - wiki/concepts/physical-analysis.md
  - wiki/concepts/dependence-analysis.md
  - wiki/concepts/equivalence-set.md
  - wiki/concepts/control-replication.md
  - wiki/concepts/coherence-mode.md
---

## TL;DR
Visibility algorithms are how Legion's physical analysis answers "for this operation's read, which prior writers' effects must I see?". The key insight of `visibility2023.pdf` (PPoPP 2023) is that this problem — combined with **content-based coherence** (regions may overlap, aliased subregions, reductions composed of fragments) — is mathematically identical to the **visibility problem in computer graphics** ("which surface is visible at this pixel?"). Legion implements three classic graphics algorithms: **painter's algorithm**, **Warnock's algorithm**, and **ray casting**. Ray casting wins on scaling and is the **current production implementation**. The confusion: this isn't analogy — it's a literal reduction. The paper proves the two problems are equivalent and ports algorithms across.

## Mental model
Content-based coherence is the harder cousin of name-based coherence. In **name-based** systems (most task-based runtimes), regions are pairwise disjoint, so a task reading region R only needs to consult writers of *region R*. In **content-based** (Legion's), regions can overlap — a task reading subregion R₁ might need to see writers of R₂ that intersect R₁. This is dependence analysis in the **temporal** dimension, and the paper shows it's the same problem graphics solves in the **spatial** dimension. Ray casting in graphics shoots rays through pixels and records what they hit; ray casting in Legion shoots queries through equivalence sets and records what they hit.

## Mechanism & API
Per `visibility2023.pdf` §3-7:

**Reduction to visibility**: a sequence of operations `⟨o₁, t₁⟩, ..., ⟨oₙ, tₙ⟩` over a data element `v` is processed by a blending function `B`:
- write `wₓ` → `B = x` (opaque, occludes prior).
- reduction `f_x` → `B = f(x, prior)` (partially transparent).
- read → returns current `B`.

This is alpha blending — writes are fully opaque, reductions are partially transparent, reads observe the current state. The "depth" axis in graphics becomes the "time" axis in Legion.

**The three algorithms** (`visibility2023.pdf` §5-7):

1. **Painter's algorithm** (§5): keep a *history* per region-tree node — a list of `(privilege, region)` pairs. To materialize a read of region R, walk the history from oldest to newest, blending operations whose region intersects R. **Optimization**: store histories on the region tree at the lowest node that covers them; traverse a "path history" from root to R. Painter's scales linearly with history length — fine for short histories, poor for long ones.

2. **Warnock's algorithm** (§6): recursive spatial decomposition. Maintain a set of **equivalence sets** — `(region, history)` pairs where every operation in `history` is relevant to every point of `region`. When a new task arrives, **refine**: split equivalence sets whose region partially overlaps the task's, producing two sub-equivalence-sets (one inside, one outside). Each refinement reduces history length. Warnock's improves on painter's but suffers when many overlapping aliased partitions force exponential refinement.

3. **Ray casting** (§7): like Warnock's but smarter about writes. Maintain equivalence sets organized as a **BVH (Bounding Volume Hierarchy)** over the region tree's partitions. When a write occurs, *coalesce* equivalence sets it dominates rather than refining further. The BVH gives `O(log)` lookup for region-overlap queries; coalescing prevents the exponential blowup Warnock's suffers. Ray casting also handles reductions efficiently via `dominating_write` — a special op that prunes occluded equivalence sets.

**Legion uses ray casting** (`visibility2023.pdf` §7-8): "as a result the ray casting algorithm is the one currently in use by the Legion project". The painter's and Warnock's implementations exist but are kept as reference / fallback. The BVH is a distributed data structure; immutable subtree nodes are replicated across nodes for scalability.

**Under control replication** (`control-replication.md` + `visibility2023.pdf` §8): each shard runs the analysis locally over its slice of the equivalence-set BVH. Cross-shard queries are necessary only when an equivalence set's region spans the sharding boundary.

**Reduction operators** ("partial transparency" in graphics terms): operators must have an identity (the "fully transparent" state). Two reductions with the same operator are non-interfering (they "blend" rather than occlude). A read followed by a reduction with the same operator can be coalesced.

## Invariants
- Visibility queries are **sound**: any writer whose effects are observable per Legion's sequential semantics is returned.
- Painter's and Warnock's produce identical results to ray casting — they differ only in algorithmic complexity.
- Ray casting's BVH is built from **disjoint, complete partitions** in the region tree. When the application uses aliased partitions, the runtime falls back to K-d trees (`visibility2023.pdf` §7).
- Equivalence sets are immutable after their history is sealed (a new write replaces them entirely rather than mutating). This is what enables distributed replication.
- Coherence weaker than `EXCLUSIVE` (see `coherence-mode.md`) widens the visibility predicate; `ATOMIC`/`SIMULTANEOUS` mean fewer writers are reported.

## Performance implications
- **Painter's**: O(history length) per query. Good for tight loops with few prior writes; poor at scale.
- **Warnock's**: better than painter's on programs with stable partitions; degrades on workloads with many overlapping aliased partitions (worst case: exponential equivalence sets, evaluation §8.1 shows it scales only to 256 nodes on stencil).
- **Ray casting**: scales to **512+ nodes** on stencil, circuit, and PENNANT benchmarks (`visibility2023.pdf` Fig. 12-17). The clear winner; near-constant per-node overhead with DCR.
- **DCR is critical**: without dynamic control replication, all three algorithms have a sequential bottleneck on the control node. With DCR, ray casting scales linearly.
- The cost of visibility queries shows up as **utility-processor activity** in `legion-prof.md`; for traced replay (`tracing.md`), the result is memoized.

## Debug signals
- **Heavy utility-row activity** scaling worse than linearly with node count → likely Warnock's or painter's behavior; verify the runtime is using ray casting (default in modern Legion).
- **`-level legion_analysis=2`** logs per-query stats — number of writers returned, equivalence-set traversal depth.
- **Legion Spy event graph** shows precise visibility output; sparser-than-expected = visibility correctly pruning false dependencies.

## Failure modes
- Many overlapping aliased partitions → BVH falls back to K-d trees, less cache-friendly.
- Without control replication → centralized analysis becomes the bottleneck regardless of algorithm.

## Source pointers
- **Paper**: `raw/publications/pdfs/visibility2023.pdf` — PPoPP 2023, *Visibility Algorithms for Dynamic Dependence Analysis and Distributed Coherence* (Bauer, Slaughter, Treichler, Lee, Garland, Aiken).
- **Companion paper (correctness)**: `raw/publications/pdfs/dep2018.pdf`.
- **Implementation tree**: https://github.com/StanfordLegion/legion/tree/master/runtime/legion (`legion_analysis.cc`, `region_tree.cc`).
- **Lectures**: `raw/youtube_transcripts/runtime_school_2023/` Lessons 9–14.

## Related
- `wiki/concepts/physical-analysis.md` — host of the visibility queries.
- `wiki/concepts/dependence-analysis.md` — umbrella that visibility serves.
- `wiki/concepts/equivalence-set.md` — the BVH leaves; the data structure visibility queries traverse.
- `wiki/concepts/control-replication.md` — essential for scalable visibility.
- `wiki/concepts/coherence-mode.md` — modulates what visibility must return.

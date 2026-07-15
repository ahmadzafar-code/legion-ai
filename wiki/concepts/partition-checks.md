---
title: Partition Checks
slug: partition-checks
summary: Runtime flag `-lg:partcheck` that verifies every "disjoint" partition is actually disjoint at creation time; catches silent data races caused by misdeclared disjointness.
tags: [debugging, partitioning, configuration, for-correctness-debug]
subsystem: legion
layer: tooling
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/website-pages/debugging.md
github:
  - https://github.com/StanfordLegion/legion/tree/master/runtime/legion
related:
  - wiki/concepts/partition.md
  - wiki/concepts/non-interference.md
  - wiki/concepts/legion-spy.md
  - wiki/concepts/debug-mode.md
---

## TL;DR
`-lg:partcheck` is a runtime flag that turns on dynamic verification of every partition the application declares as disjoint. Without it, the runtime trusts the declaration; with it, partition creation pays a one-time cost to actually check the coloring has no overlapping subregions. The confusion: a non-disjoint "disjoint" partition is a **correctness** bug, but the runtime's normal default is to trust the declaration so the bug manifests as a silent data race or wrong output — not a runtime error. `-lg:partcheck` turns it into a clean error.

## Mental model
`-lg:partcheck` is to partitions what `-fsanitize=address` is to memory: an opt-in runtime verifier that catches a class of bug the type system can't prove. The cost is partition-creation overhead; the benefit is replacing "wrong answer with no error" with "loud assert at the moment of the misdeclaration".

## Mechanism & API
Pass at runtime:
```bash
./app -lg:partcheck
```

What the runtime checks under this flag (per `raw/website-pages/debugging.md`):
- At every `create_partition_by_*` call whose `disjoint` argument is true, walk the coloring and confirm that no two sub-spaces share any points.
- If a violation is found, the runtime errors out at the partition-creation call site with a diagnostic.

The check applies to all partition-creation APIs that take a disjointness flag — `create_equal_partition` (provably disjoint, no check needed), `create_partition_by_field`, `create_partition_by_image`, `create_partition_by_preimage`, hand-built colorings via `create_partition_by_domain`/`create_partition_by_pieces`, etc.

**Why disjointness misdeclaration matters**:
- Disjoint partitions enable maximum non-interference (`non-interference.md`): operations on disjoint subregions are independent and the runtime parallelizes them aggressively.
- If the declaration is wrong, the runtime parallelizes operations that actually conflict — undefined behavior in release builds, often producing different wrong answers on different runs.
- Especially common with `partition_by_field`/`partition_by_image` when the source data has unexpected duplicates.

## Invariants
- `-lg:partcheck` adds **no false positives**: a partition declared disjoint that *is* disjoint will not trigger.
- The check runs at **partition creation time**, not at usage time — moves the failure to the actual buggy line.
- Adds **no semantic change** to correct programs.
- The check is **per-partition**, not per-operation; cost amortizes over the partition's lifetime.
- Combinable with `DEBUG=1`, `-DPRIVILEGE_CHECKS`, `-DBOUNDS_CHECKS`, and other debug aids.

## Performance implications
- The check costs **O(points × log points)** at partition creation; substantial for very large partitions.
- Recommend leaving on during development and turning off in `DEBUG=0` release builds.
- For `partition_by_image`/`partition_by_preimage` over fields whose disjointness depends on data, run `-lg:partcheck` **with representative inputs** to catch real-world coloring errors.

## Debug signals
- **Runtime error from a `create_partition_*` call** with `-lg:partcheck` on → declared `disjoint=true` but isn't. Either fix the coloring or change to `disjoint=false`.
- **Sibling point tasks of an `IndexLauncher` over the partition serializing** when `-lg:partcheck` is off → strong suspicion of misdeclared disjointness; rerun with the flag.
- **Legion Spy `-d` showing edges between supposedly-independent point tasks** → confirm partition disjointness with `-lg:partcheck`.

## Failure modes
- Caught by this very check: declaring `disjoint=true` on a partition whose coloring actually overlaps.
- Not caught: aliased partitions that the application correctly declares non-disjoint and intentionally uses for halo patterns.

## Source pointers
- **Reference**: `raw/website-pages/debugging.md`
- **Runtime tree**: https://github.com/StanfordLegion/legion/tree/master/runtime/legion (the check is implemented in `region_tree.cc`)

## Related
- `wiki/concepts/partition.md` — what gets checked.
- `wiki/concepts/non-interference.md` — disjointness is what makes parallel point-task execution legal.
- `wiki/concepts/legion-spy.md` — the analysis tool that shows what bad disjointness looks like in the operation graph.

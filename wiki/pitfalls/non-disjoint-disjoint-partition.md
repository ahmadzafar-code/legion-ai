---
title: Non-Disjoint "Disjoint" Partition
slug: non-disjoint-disjoint-partition
summary: A partition declared `disjoint=true` whose coloring actually overlaps; the runtime parallelizes point tasks that conflict, producing silent data races or wrong results.
tags: [for-correctness-debug, partitioning, parallelism]
status: draft
created: 2026-05-15
updated: 2026-05-15
related:
  - wiki/concepts/partition.md
  - wiki/concepts/disjoint-partition.md
  - wiki/concepts/aliased-partition.md
  - wiki/concepts/partition-checks.md
  - wiki/concepts/legion-spy.md
  - wiki/concepts/dependent-partitioning.md
  - wiki/workflows/debug-correctness-bug.md
---

## Symptom

- Point tasks of an `IndexLauncher` over the partition produce **non-deterministic or wrong results** across runs.
- Results are **correct under `-lg:inorder`** (which serializes everything) but wrong without it. Classic signal of a data race.
- Results are **wrong only at higher parallelism** — the bug needs enough concurrent point tasks to hit the overlap.
- `legion_spy.py -dez` shows **dependence edges between supposedly-independent sibling point tasks** — the runtime correctly detected the per-point conflict, even though the application declared the partition disjoint at the partition level.
- Adding `-lg:partcheck` triggers a runtime error at partition creation — **direct confirmation**.

## Cause

The application called a partition constructor with `disjoint=true` but the resulting coloring has overlapping subregions. The runtime **trusts the declaration** and (without `-lg:partcheck`) does not verify. When the user later launches point tasks expecting full parallelism, the runtime parallelizes them — and per-point physical analysis discovers conflicts that produce undefined behavior.

Three common ways this happens:

1. **Hand-built coloring with off-by-one errors**. The user constructs subregion bounds manually (e.g., `lo[i] = i*stride; hi[i] = (i+1)*stride;` instead of `(i+1)*stride - 1`) and accidentally overlaps adjacent chunks by one point.

2. **`partition_by_field` / `partition_by_image` with data the user thought was unique**. The application asserts `disjoint=true` but the coloring field has duplicates, or the image of pointers has duplicates. The partition is data-dependent and only sometimes aliased.

3. **Modified data invalidating a previously-disjoint partition**. The partition was disjoint at creation; subsequent updates to the coloring field made it aliased. The partition is stale.

## Fix

- **Run with `-lg:partcheck`** (`partition-checks.md`). This is the canonical fix: it makes the runtime verify disjointness at the `create_partition_*` call site. Either the assertion passes (and the bug is elsewhere) or it fires (and you've found the line).

- **Switch to a runtime-verified-disjoint constructor** when possible:
  - `create_equal_partition` — disjoint by construction.
  - `create_partition_by_restriction` — affine block; disjoint by construction.
  - `create_partition_by_field` over a field with provably-unique values; declare `disjoint=true` and run with `partition-checks` on.

- **For genuinely aliased data**, declare `disjoint=false` and use the proper aliased pattern. This is correct, not a bug — see `aliased-partition.md` and `ghost-region.md`. The pattern is to keep a disjoint partition for writes and an aliased one for reads.

- **For data-dependent partitions** built with `partition_by_image`/`preimage`, check the source field's uniqueness assumption explicitly before declaring the result disjoint.

- **Combine with `dataflow-graph.md` review**. Re-run `legion_spy.py -dez` after fixing — the spurious edges between sibling point tasks should be gone (or, if the partition is correctly aliased, expected and present at the right places).

## Underlying concepts

- `wiki/concepts/partition.md` — umbrella.
- `wiki/concepts/disjoint-partition.md` — what was declared.
- `wiki/concepts/aliased-partition.md` — what the data actually is, usually.
- `wiki/concepts/partition-checks.md` — runtime verifier; the direct diagnostic.
- `wiki/concepts/legion-spy.md` — for confirming the symptom in the operation graph.
- `wiki/concepts/dependent-partitioning.md` — where this bug most often originates.
- `wiki/workflows/debug-correctness-bug.md` — the workflow that surfaces this pitfall.

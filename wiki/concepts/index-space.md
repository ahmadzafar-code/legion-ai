---
title: Index Space
slug: index-space
summary: The "rows" of a logical region; an immutable set of points (1D or N-D, dense or sparse) that gets partitioned into subspaces; one half of every logical region's identity.
tags: [data-model, for-program-reasoning]
subsystem: legion
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/05_logical_regions.md
  - raw/tutorials/08_partitioning.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
related:
  - wiki/concepts/logical-region.md
  - wiki/concepts/field-space.md
  - wiki/concepts/partition.md
  - wiki/concepts/subregion.md
  - wiki/concepts/region-tree.md
---

## TL;DR
An `IndexSpace` is the set of points (1D or N-D) over which a `logical-region.md` is defined. It's **immutable** once created — you can't add or remove points after the fact, but you can **partition** it into subspaces. Supports dense rectangles via `Rect`/`Domain` and sparse domains too. Every logical region is the cross-product of one index space (rows) and one `field-space.md` (columns). The confusion: an `IndexSpace` does not hold *data* — it holds *point identities*. The actual values live in physical instances, addressed by the index space's points.

## Mental model
An `IndexSpace` is a database table's primary key set, lifted to be a first-class type. The set is fixed at table-creation time; you index into the table by point. Where SQL has implicit primary keys, Legion makes them explicit so the runtime can partition them, project them, and reason about which points each task touches.

## Mechanism & API
**Dense N-D**:
```cpp
const Rect<1> rect(0, 1023);
IndexSpaceT<1> typed_is = runtime->create_index_space(ctx, rect);

const Domain domain(DomainPoint(0), DomainPoint(1023));
IndexSpace untyped_is = runtime->create_index_space(ctx, domain);
```

`Rect<DIM, COORD_T>` is the typed compile-time form (preferred); `Domain` carries dimensionality at runtime (useful when DIM is dynamic).

**Sparse** index spaces are created directly with point sets or as the result of `dependent-partitioning.md` operations.

**Use** in a region:
```cpp
LogicalRegion lr = runtime->create_logical_region(ctx, typed_is, fs);
```

**Partition**:
```cpp
IndexPartition ip = runtime->create_equal_partition(ctx, typed_is, color_space);
```
See `partition.md`, `disjoint-partition.md`, `aliased-partition.md`.

**Query the domain**:
```cpp
Domain d = runtime->get_index_space_domain(ctx, is);
```

**Destroy** when the application is done with it:
```cpp
runtime->destroy_index_space(ctx, is);
```
The runtime defers actual reclamation until all in-flight uses complete.

## Invariants
- An `IndexSpace` is **immutable** after creation — you cannot add or remove points. Partition it instead.
- Every `IndexSpace` has a fixed **dimensionality** (1, 2, 3, ...) and a **coordinate type** (default `coord_t` = 64-bit signed integer).
- The triple `(index_space, field_space, tree_id)` is the unique identity of a `logical-region.md`. Distinct calls to `create_logical_region` with the same `(is, fs)` produce distinct regions.
- Sparse index spaces are supported; their points need not be in a single bounding rectangle.
- Children of an index space partition are themselves `IndexSpace`s in their own right; the lifting from `IndexPartition` → per-color `IndexSpace` is automatic.

## Performance implications
- Index space creation is **cheap** (a runtime data-structure update; no buffer allocation).
- Partitioning is **lazy** — the runtime materializes subspace data structures only when used.
- Sparse index spaces and dependent-partitioning operations can be expensive at creation time; cache results in iterative codes.
- Errors related to index-space handles (`error-message-catalog.md` codes 451-500) typically point at handle misuse, dimension mismatch, or operations exceeding bounds.

## Debug signals
- **Error 451** ("Invalid index space"): a handle does not refer to a valid `IndexSpace`. Usually a destroyed handle still being used, or a handle from a different context.
- **Error "Index space bounds error"**: an operation references points outside the index space's domain. Often paired with `bounds-checks.md` accessor failures.
- **`legion-spy.md`** region-tree output renders index-space partitioning; useful for visualizing what coloring you actually have.

## Failure modes
- Using a destroyed `IndexSpace` handle → undefined behavior (or error in debug builds).
- Assuming a partitioned index space is somehow "modified" by partitioning — it isn't; the parent space's point set is unchanged.

## Source pointers
- **Legion API**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Tutorial**: https://legion.stanford.edu/tutorial/logical_regions.html

## Related
- `wiki/concepts/logical-region.md` — what this is one half of.
- `wiki/concepts/field-space.md` — the other half.
- `wiki/concepts/partition.md` — how to subdivide.
- `wiki/concepts/subregion.md` — what partitioning produces.
- `wiki/concepts/region-tree.md` — the runtime structure index spaces live in.

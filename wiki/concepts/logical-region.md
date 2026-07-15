---
title: Logical Region
slug: logical-region
summary: Legion's core data abstraction; the cross-product of an index space (rows) and a field space (columns), addressed independently of any physical layout.
tags: [data-model, for-program-reasoning]
subsystem: legion
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/website-pages/overview.md
  - raw/tutorials/05_logical_regions.md
  - raw/publications/publications.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/region_tree.h
related:
  - wiki/concepts/task.md
  - wiki/concepts/privilege.md
  - wiki/concepts/partition.md
  - wiki/concepts/physical-instance.md
  - wiki/concepts/region-tree.md
  - wiki/concepts/regent-type-system.md
  - wiki/concepts/index-space.md
  - wiki/concepts/field-space.md
  - wiki/concepts/subregion.md
---

## TL;DR
A logical region is the unit of data that Legion reasons about. It is the cross-product of an **index space** (rows / element identities) and a **field space** (columns / typed attributes), with no commitment to physical storage. Two logical regions can share an index space and a field space and still be distinct — region identity is `(index_space, field_space, tree_id)`. The confusion: a "region" is a *name* for data, not the data itself; the data lives in a physical instance picked by the mapper.

## Mental model
Think of a logical region like a relational table: index space is the primary key set, field space is the schema. Legion programs manipulate these table *names*; the runtime decides where copies of the table live in memory (system RAM, GPU framebuffer, zero-copy memory) based on what mappers say. Two declarations of the "same" table produce two distinct regions on purpose — that's what lets the runtime overlap their lifetimes without aliasing concerns.

## Mechanism & API
- `runtime->create_index_space(ctx, domain)` — `IndexSpace` of points. 1D or N-D; sparse domains supported.
- `runtime->create_field_space(ctx)` — empty `FieldSpace`; populate via `FieldAllocator::allocate_field(size, FieldID)`. Hard cap of `MAX_FIELDS` per field space (default 512); make a new field space to exceed that.
- `runtime->create_logical_region(ctx, is, fs)` — combines them; returns `LogicalRegion`. Each call yields a fresh region, even with identical inputs. Identity is `(index_space_id, field_space_id, tree_id)`.
- `runtime->destroy_logical_region(ctx, lr)`, `destroy_field_space`, `destroy_index_space` — deferred destruction; the runtime waits for in-flight users.

Fields are assumed trivially copyable. Non-POD types need a custom serializer.

## Invariants
- An index space is **immutable** after creation; points cannot be added or removed (you partition it instead).
- A logical region exposes data **only through region requirements** on tasks (see `privilege.md`) or `InlineMapping`.
- The triple `(index_space, field_space, tree_id)` uniquely identifies a logical region; two regions with identical `is`/`fs` but different `tree_id` are non-aliasing for dependence analysis.
- A logical region has **no physical storage of its own** — physical instances are created lazily by the mapper at task-mapping time.
- A subregion is always a subset of its parent's points and shares the same field space.

## Performance implications
- Region creation is cheap; partitioning is cheap-ish but lazy.
- The runtime tracks region trees in its operation pipeline (see `operation-pipeline.md`); excessive numbers of distinct regions inflate the dependence-analysis cost.
- Naming distinct regions for distinct logical data (rather than overloading one big region) makes finer non-interference possible — see `privilege.md`.

## Debug signals
- **Legion Spy**: dataflow and event graphs label nodes by logical region triple; use `runtime->attach_name(lr, "input_lr")` so the graphs are readable.
- **Bounds checks** (`-DBOUNDS_CHECKS`): triggers if a task accesses a point not in its requested region.
- **Privilege checks** (`-DPRIVILEGE_CHECKS`): triggers if a task reads/writes a field it didn't request on this region.

## Failure modes
- [Over-broad privileges create false dependencies](../pitfalls/false-dependencies-overbroad-privileges.md) — requesting a whole region when only a subregion is touched.

## Source pointers
- **Header**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Region tree implementation**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/region_tree.h
- **Tutorial**: https://legion.stanford.edu/tutorial/logical_regions.html (mirrored at `raw/tutorials/05_logical_regions.md`)

## Related
- `wiki/concepts/task.md` — tasks declare access to logical regions via region requirements.
- `wiki/concepts/privilege.md` — what kind of access a task takes on a region.
- `wiki/concepts/partition.md` — how to break a region into subregions for parallel work.
- `wiki/concepts/physical-instance.md` — where the data named by a logical region actually lives.

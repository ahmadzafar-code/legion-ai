---
title: Field Space
slug: field-space
summary: The "columns" of a logical region; a typed namespace of fields (each with its own size) that gets shared across all regions defined over it; one half of every logical region's identity.
tags: [data-model, for-program-reasoning]
subsystem: legion
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/05_logical_regions.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
related:
  - wiki/concepts/logical-region.md
  - wiki/concepts/index-space.md
  - wiki/concepts/region-requirement.md
  - wiki/concepts/field-level-non-interference.md
  - wiki/concepts/region-tree.md
---

## TL;DR
A `FieldSpace` is a typed namespace of fields — each field has a `FieldID` and a byte size. Multiple logical regions can share a field space (they have the same columns), and a region can be defined over an `index-space.md` × `field-space.md` cross product. Built dynamically via `FieldAllocator`. Hard cap of `MAX_FIELDS` per field space (default 512) — for more fields, use multiple field spaces. The confusion: a `FieldSpace` is not a struct definition; fields are bytes-sized cells, not C++ types. Same field space, different regions = same columns, different rows.

## Mental model
A `FieldSpace` is the schema of a database table — the columns and their widths. Where SQL writes `CREATE TABLE T (x INT, y FLOAT, z DOUBLE)`, Legion writes `create_field_space() + allocate_field(sizeof(int), FID_X)` etc. Sharing a field space across multiple regions is like having two tables with the same schema.

## Mechanism & API
**Create + populate**:
```cpp
FieldSpace fs = runtime->create_field_space(ctx);
{
  FieldAllocator allocator = runtime->create_field_allocator(ctx, fs);
  FieldID fida = allocator.allocate_field(sizeof(double), FID_X);  // returns FID_X
  FieldID fidb = allocator.allocate_field(sizeof(int),    FID_Y);
}
```

`FieldAllocator` is a transient handle (scoped to the function). `allocate_field` takes the field size in bytes and an optional `FieldID`; if the ID is unspecified, the runtime picks one and returns it.

**Use** with an index space to make a logical region:
```cpp
LogicalRegion lr1 = runtime->create_logical_region(ctx, is, fs);
LogicalRegion lr2 = runtime->create_logical_region(ctx, is, fs);
// lr1 and lr2 are distinct regions sharing (is, fs); their tree_ids differ.
```

**Attach human-readable names** for debugging (improves `legion-spy.md` graph labels):
```cpp
runtime->attach_name(fs, "input_fs");
runtime->attach_name(fs, FID_X, "X");
```

**Destroy** when done:
```cpp
runtime->destroy_field_space(ctx, fs);
```

## Invariants
- A `FieldSpace` can hold up to `MAX_FIELDS` fields (default 512; compile-time configurable).
- Field IDs are **per-field-space**; FID_X in field space A and FID_X in field space B are distinct fields.
- Fields are assumed **trivially copyable**. Non-trivial C++ types need a custom serializer registered with the runtime.
- A field's size is fixed at allocation; you can't grow a field after the fact.
- Multiple regions sharing a `FieldSpace` have the **same set of fields**; they can be used with `field-level-non-interference.md` to parallelize across fields independent of region overlap.

## Performance implications
- Field-space creation is **cheap** (a runtime data-structure update; no buffers).
- Field allocation is cheap; the actual storage is materialized at `physical-instance.md` time.
- The `MAX_FIELDS` cap is per-field-space — create more field spaces if you need many fields. Sharing a field space across regions enables field-level non-interference across them.
- Fields not in a task's `RegionRequirement::privilege_fields` cost nothing for that task — the runtime doesn't materialize them.

## Debug signals
- **`error-message-catalog.md`** codes around 101-200 — Region-Related Errors — frequently involve field spaces:
  - "Invalid field ID": the FID isn't allocated in this field space.
  - "Field space mismatch": two regions used together must have the same field space.
  - "Duplicate field allocation": tried to allocate an FID that already exists.
- **`legion-spy.md`** dataflow graph labels show field IDs on edges; use `attach_name` for readability.
- **`-DPRIVILEGE_CHECKS`** catches access to fields not in the task's `privilege_fields`.

## Failure modes
- Allocating a field, destroying the field space, then accessing a region built over it → use-after-destroy; UB in release builds.
- Hitting `MAX_FIELDS` → allocate a new field space instead.
- Confusing field IDs across field spaces — they're not interchangeable.

## Source pointers
- **Legion API**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Tutorial**: https://legion.stanford.edu/tutorial/logical_regions.html

## Related
- `wiki/concepts/logical-region.md` — what this is half of.
- `wiki/concepts/index-space.md` — the other half.
- `wiki/concepts/region-requirement.md` — where `privilege_fields` is set.
- `wiki/concepts/field-level-non-interference.md` — parallelism across disjoint fields.
- `wiki/concepts/region-tree.md` — runtime structure.

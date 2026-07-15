---
title: Field Accessor
slug: field-accessor
summary: A typed C++ wrapper around a PhysicalRegion that turns a logical point into a typed memory dereference at the correct address for the chosen instance layout.
tags: [data-model, instances]
subsystem: legion
layer: programming-model
status: stub
created: 2026-05-15
updated: 2026-05-15
related:
  - wiki/concepts/physical-instance.md
  - wiki/concepts/privilege.md
---

```cpp
const FieldAccessor<READ_ONLY, double, 1> acc(regions[0], FID_X);
double v = acc[point];          // typed load
```

Compile-time privilege check (with `-DPRIVILEGE_CHECKS`) and compile-time bounds check (with `-DBOUNDS_CHECKS`). Full page pending; see `raw/tutorials/06_physical_regions.md`.

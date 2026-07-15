---
title: Virtual Mapping
slug: virtual-mapping
summary: A mapper option to satisfy a task's region requirement without creating a physical instance; the privilege transfers to the task but no buffer is allocated, deferring materialization to the task's subtasks.
tags: [mapping, instances, memory, for-perf-debug]
subsystem: legion
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/website-pages/mapper.md
  - raw/tutorials/07_privileges.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion_mapping.h
related:
  - wiki/concepts/mapper.md
  - wiki/concepts/physical-instance.md
  - wiki/concepts/inner-task.md
  - wiki/concepts/region-requirement.md
  - wiki/concepts/map-task.md
---

## TL;DR
Virtual mapping is a `map_task` option that says "give this task the *privilege* to access the region, but don't allocate a physical instance for it — its subtasks will materialize when they actually need to read or write the data". The privilege chain stays intact (subtasks still take a subset of the parent's privileges) but no buffer exists at the virtually-mapped task's level. The confusion: virtual mapping isn't a privilege type — it's a mapper-level decision. The task body still receives a `PhysicalRegion` in `regions[i]`, but `region.is_mapped()` returns `false` and constructing a `FieldAccessor` on it is undefined.

## Mental model
Virtual mapping is `mmap(MAP_NORESERVE)` for Legion: the address space (privilege + region identity) is reserved but no pages back it yet. The "pages" — physical instances — get allocated when subtasks request real access. For `inner-task.md`s that only orchestrate work, this saves allocating instances the task never touches.

## Mechanism & API
**From the mapper side** (in `map_task` for a requirement):
```cpp
void map_task(const MapperContext ctx, const Task &task,
              const MapTaskInput &in, MapTaskOutput &out) override {
  for (size_t i = 0; i < task.regions.size(); i++) {
    if (task.tag == VIRTUAL_MAP_TAG) {
      out.chosen_instances[i].push_back(PhysicalInstance::get_virtual_instance());
    } else {
      // normal instance creation
    }
  }
}
```

**Or directly on the region requirement** at launcher build time:
```cpp
launcher.region_requirements[0].flags |= NO_ACCESS_FLAG;  // or VIRTUAL_MAP request
launcher.region_requirements[0].virtual_map = true;       // depending on Legion version
```

**Inside the task body**:
```cpp
void inner_task(const Task *task, const std::vector<PhysicalRegion> &regions,
                Context ctx, Runtime *runtime) {
  // regions[0].is_mapped() == false for the virtually-mapped requirement
  // Do NOT construct FieldAccessor on regions[0].
  // OK to launch subtasks using the same region/privilege:
  TaskLauncher sub(SUB_TASK_ID, TaskArgument(NULL, 0));
  sub.add_region_requirement(RegionRequirement(task->regions[0].region, RW, EXCLUSIVE,
                                               task->regions[0].region));
  sub.add_field(0, FID_X);
  runtime->execute_task(ctx, sub);
}
```

## Invariants
- A virtually-mapped requirement has **no physical instance**; constructing a `FieldAccessor` on the corresponding `PhysicalRegion` is undefined behavior.
- The **privilege** still transfers normally — subtasks may request a subset of the inner task's privilege via region requirements.
- Subtasks may independently choose to map (not virtual) the same region; the subtask's mapper makes its own placement choice.
- Combining virtual mapping with `inner-task.md` is the standard pattern; an `inner` task with virtually-mapped regions costs essentially zero allocation.
- The runtime's dependence analysis treats a virtually-mapped requirement the same as a real one for the purpose of computing operation order — the *fact* of the requirement matters, not whether it's backed by storage.

## Performance implications
- **Saves the allocation + copy** that would otherwise be needed to give the inner task an instance it doesn't use.
- For deep call trees (top-level → orchestrator → kernel), virtual mapping at the upper levels keeps instances colocated with the leaf kernels rather than mirrored at every level.
- The win is most visible when the inner task accepts large regions and its subtasks only touch small subregions — the upper levels would have had to allocate large instances unnecessarily.
- Wrong use (virtually mapping a region the body actually reads) → runtime error or crash. There's no silent fallback.

## Debug signals
- **`region.is_mapped() == false`** is the explicit check inside a task body.
- **`LoggingWrapper`**: `map_task` logs show `chosen_instances` containing `PhysicalInstance::get_virtual_instance()` (or the equivalent).
- **Crash when constructing an accessor**: the virtually-mapped requirement was treated as real; either remove the virtual flag or move the access into a subtask.

## Failure modes
- Accessing a virtually-mapped region in the task body → crash or undefined behavior.
- Virtually mapping a leaf task's region → the task has no way to access its data; runtime error or crash.

## Source pointers
- **Mapper API**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion_mapping.h
- **Reference**: `raw/website-pages/mapper.md` (virtual mapping section)
- **Tutorial mention**: `raw/tutorials/07_privileges.md` (`is_mapped` usage)

## Related
- `wiki/concepts/mapper.md` — where virtual mapping is decided.
- `wiki/concepts/physical-instance.md` — what virtual mapping does NOT create.
- `wiki/concepts/inner-task.md` — the natural client of virtual mapping.
- `wiki/concepts/region-requirement.md` — the `flags` / `virtual_map` field.
- `wiki/concepts/map-task.md` — the callback where virtual mapping is set.

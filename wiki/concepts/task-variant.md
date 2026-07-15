---
title: Task Variant
slug: task-variant
summary: One of potentially many implementations of a Legion task ID, registered with a `TaskVariantRegistrar` carrying execution constraints (processor kind, leaf/inner property, layout requirements) the mapper uses to pick a runnable variant.
tags: [execution, mapping, for-program-reasoning, for-perf-debug]
subsystem: legion
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/02_tasks_and_futures.md
  - raw/tutorials/04_hybrid_model.md
  - raw/website-pages/mapper.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
related:
  - wiki/concepts/task.md
  - wiki/concepts/task-launcher.md
  - wiki/concepts/mapper.md
  - wiki/concepts/leaf-task.md
  - wiki/concepts/map-task.md
---

## TL;DR
A task variant is a *specific implementation* of a Legion task ID — for example, a CPU implementation of `STENCIL_TASK_ID` and a GPU implementation of the same task ID are two distinct variants of the one task. You register each via a `TaskVariantRegistrar` plus a function pointer. The registrar carries **execution constraints** (processor kind, leaf/inner property, layout requirements) which the mapper queries during `map_task` to pick a runnable variant for the chosen `target_proc`. The confusion: a task ID is a *contract* (signature + privileges + behavior); a variant is a *backend* for that contract. The mapper's job is to pick a variant whose constraints match its placement decision.

## Mental model
Task variants are to a Legion task ID what CPU instruction encodings are to an ISA opcode: multiple ways of implementing the same logical operation, with the runtime/mapper picking the appropriate one for the available hardware. Where x86 has SSE/AVX/AVX-512 variants of a vector add, Legion has CPU/GPU/leaf/inner variants of a task.

## Mechanism & API
**Registration**:
```cpp
{
  TaskVariantRegistrar reg(STENCIL_TASK_ID, "stencil_cpu");
  reg.add_constraint(ProcessorConstraint(Processor::LOC_PROC));
  reg.set_leaf(true);
  Runtime::preregister_task_variant<stencil_cpu_impl>(reg, "stencil_cpu");
}
{
  TaskVariantRegistrar reg(STENCIL_TASK_ID, "stencil_gpu");
  reg.add_constraint(ProcessorConstraint(Processor::TOC_PROC));
  reg.set_leaf(true);
  Runtime::preregister_task_variant<stencil_gpu_impl>(reg, "stencil_gpu");
}
```

Each call adds one variant. Both share the same `TaskID`; the runtime stores them in a per-task table keyed by `VariantID` (auto-generated unless you pass an explicit ID).

**Constraint kinds** carried on the registrar:
- `ProcessorConstraint(Processor::LOC_PROC|TOC_PROC|IO_PROC|UTIL_PROC|OMP_PROC|PY_PROC)` — which processor kinds may run this variant.
- `set_leaf(true)` — declares this is a [leaf task](leaf-task.md); enables the leaf-context fast path.
- `set_inner(true)` — declares this is an inner task (launches subtasks but does not access region instances directly).
- `ExecutionConstraintSet` — finer details like ISA, OS, FP-precision requirements.
- `TaskLayoutConstraintSet` — required layouts for region requirements (AOS/SOA, dimension order, alignment).

**Selection at runtime**: the mapper's `map_task` callback queries `runtime->find_valid_variants(ctx, task_id, variants)`, then `find_execution_constraints` to filter by what the chosen `target_proc` can run, then sets `output.chosen_variant`.

**Pre-registration vs registration**: `Runtime::preregister_task_variant<...>` runs *before* `Runtime::start` — variants are baked into the binary at startup. `Runtime::register_task_variant` registers at runtime (used by Pygion/Regent compilers and for dynamically-loaded code).

## Invariants
- Multiple variants per task ID is normal; one is the minimum.
- A variant's **constraints are immutable** after registration.
- The mapper must select a variant whose constraints match the chosen processor; if none does, `map_task` is an error.
- A `set_leaf(true)` variant **must not** launch subtasks at runtime; doing so is a runtime error (the leaf-context check fires).
- All variants of a task ID share the **same signature** and **same logical behavior** — only the implementation differs.
- A variant's function pointer must match the expected Legion task signature: `void(*)(const Task*, const std::vector<PhysicalRegion>&, Context, Runtime*)` (or the templated typed-return form).

## Performance implications
- **A missing variant for the desired processor kind forces fallback** — the most common cause of [GPU underutilization](../pitfalls/gpu-underutilization.md) is "no `TOC_PROC` variant registered, mapper had no choice but CPU".
- `set_leaf(true)` enables an optimized context that skips bookkeeping for subops — significant win for fine-grained leaf tasks.
- `set_inner(true)` unlocks **virtual mapping**: the inner task's regions need not have physical instances at task entry, allowing instance creation to be deferred to the subtasks.
- Layout constraints on variants let the mapper pre-create instances of the right shape, avoiding mid-execution layout mismatches.

## Debug signals
- **Mapper logs (`LoggingWrapper`)** show the `chosen_variant` for every `map_task`. Mismatches between expected and actual variant are usually constraint misconfigurations.
- **Runtime error "no valid variant"** during dependence analysis or mapping = `ProcessorConstraint` does not match any registered variant for the mapper's `target_proc`. Register a matching variant.
- **Leaf-context assertion failure** = a leaf variant called a subtask; remove `set_leaf` or refactor the body.

## Failure modes
- [GPU underutilization](../pitfalls/gpu-underutilization.md) when only a CPU variant exists.
- Mapper crashes when no variant satisfies the chosen processor's constraint.

## Source pointers
- **Legion API**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Tutorial (registration patterns)**: https://legion.stanford.edu/tutorial/tasks_and_futures.html
- **Mapper reference**: `raw/website-pages/mapper.md`

## Related
- `wiki/concepts/task.md` — task IDs that variants implement.
- `wiki/concepts/task-launcher.md` — launches that the mapper resolves to a variant.
- `wiki/concepts/mapper.md` — picks the variant.
- `wiki/concepts/leaf-task.md` — `set_leaf(true)` variants.

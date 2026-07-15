---
title: select_instance (mapper callback family)
slug: select-instance
summary: A family of mapper callbacks (`select_task_sources`, `select_copy_sources`, `select_inline_sources`, `select_release_sources`, etc.) that pick among multiple valid physical instances when more than one exists for a region requirement.
tags: [mapping, instances, for-perf-debug]
subsystem: legion
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/website-pages/mapper.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion_mapping.h
related:
  - wiki/concepts/mapper.md
  - wiki/concepts/mapper-callback.md
  - wiki/concepts/physical-instance.md
  - wiki/concepts/map-task.md
  - wiki/concepts/proc-mem-affinity.md
---

## TL;DR
`select_instance` is the family of mapper callbacks the runtime invokes when it needs to **pick among multiple valid physical instances** for a region requirement — `select_task_sources` (for an incoming task), `select_copy_sources` (for an explicit copy), `select_inline_sources` (for an inline mapping), `select_release_sources` (for a release op). The mapper returns an ordered preference list; the runtime uses the first valid one. The confusion: these are **separate callbacks from `map-task.md`** — they fire after `map_task` has chosen the destination instance, when the runtime needs to pick a *source* to copy from.

## Mental model
`select_instance` is "which existing copy should I read from?" — like a CPU cache choosing among multiple valid lines in different ways, or a CDN router picking among edge servers. The mapper expresses preference (typically "closest to the consumer"); the runtime picks the first viable one from the preference list.

## Mechanism & API
The callbacks (per `raw/website-pages/mapper.md` and `legion_mapping.h`):

**`select_task_sources`** — pick sources for the copies the runtime will issue to populate the chosen instance(s) for a task:
```cpp
void select_task_sources(const MapperContext ctx, const Task &task,
                         const SelectTaskSrcInput &input,
                         SelectTaskSrcOutput &output);
```
- `input.target` — the destination instance the runtime is filling.
- `input.source_instances` — candidate sources (valid instances elsewhere).
- `output.chosen_ranking` — your preferred order.

**`select_copy_sources`** — same callback but for explicit `IssueCopy` operations.

**`select_inline_sources`** — for `InlineMapping`.

**`select_release_sources`** — for release operations bracketing simultaneous-coherence regions.

**Typical pattern**:
```cpp
void MyMapper::select_task_sources(const MapperContext ctx, const Task &task,
                                    const SelectTaskSrcInput &in,
                                    SelectTaskSrcOutput &out) override {
  // Score each candidate by affinity to the target memory.
  auto target_mem = in.target.get_location();
  std::vector<std::pair<unsigned, PhysicalInstance>> scored;
  for (auto &cand : in.source_instances) {
    unsigned bw = bandwidth_between(cand.get_location(), target_mem);
    scored.push_back({bw, cand});
  }
  std::sort(scored.rbegin(), scored.rend());  // highest BW first
  for (auto &p : scored) out.chosen_ranking.push_back(p.second);
}
```

## Invariants
- `select_instance` callbacks **never affect correctness** — only which valid copy is read from. Multiple valid copies hold identical data.
- The runtime walks the ranking in order; if the first preference is unavailable (in transit, locked), it tries the next.
- Empty `chosen_ranking` → the runtime falls back to its default selection.
- These callbacks are subject to the standard `mapper-callback.md` rules: non-blocking, non-reentrant by default, etc.
- `MapperContext` is per-callback (`mapper-context.md`); cache machine-model queries elsewhere.

## Performance implications
- **The right source pick saves DMA bandwidth**. A copy from a nearby memory (high `proc-mem-affinity.md`) completes faster than one from a distant memory.
- For multi-node runs, picking a same-node source instead of cross-node is a major win — visible in Legion Prof channel rows.
- The default behavior (no override or empty ranking) usually picks reasonably; custom rankings shine when the mapper has application-specific knowledge.
- Like other callbacks, keep these fast — `pitfalls/mapper-stalls.md` applies.

## Debug signals
- **`LoggingWrapper`** logs each `select_*_sources` callback's `target` and `chosen_ranking`.
- **Heavy channel-row activity** between two memories despite a closer source being available → check the relevant `select_*_sources` callback's logic.
- **Slow `select_*_sources` callbacks** → cache `Machine` queries.

## Failure modes
- Empty `chosen_ranking` → runtime falls back to default. Not a bug, but a missed optimization opportunity.
- Ranking that includes invalid instances → the runtime skips them; no error, but wasted work.

## Source pointers
- **Mapper API**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion_mapping.h
- **Reference**: `raw/website-pages/mapper.md`

## Related
- `wiki/concepts/mapper.md` — host.
- `wiki/concepts/mapper-callback.md` — callback model.
- `wiki/concepts/physical-instance.md` — what's being chosen among.
- `wiki/concepts/map-task.md` — the prior callback that chose the destination.
- `wiki/concepts/proc-mem-affinity.md` — the input the ranking typically derives from.

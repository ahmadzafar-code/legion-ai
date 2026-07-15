---
title: EXCLUSIVE Coherence
slug: exclusive-coherence
summary: The default coherence mode; demands strict sequential-program-order semantics between any two conflicting accesses, giving Legion programs their "as-if-serial" illusion.
tags: [data-model, coherence, for-program-reasoning]
subsystem: legion
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/tutorials/07_privileges.md
github:
  - https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
related:
  - wiki/concepts/coherence-mode.md
  - wiki/concepts/privilege.md
  - wiki/concepts/non-interference.md
  - wiki/concepts/region-requirement.md
---

## TL;DR
`EXCLUSIVE` is Legion's **default coherence mode**: any two operations whose region requirements conflict (overlapping points + fields + incompatible privileges) execute in **program order**. This is what makes a Legion program look sequential to its author — the runtime preserves the illusion regardless of how many processors it actually runs on. The confusion: `EXCLUSIVE` is not pessimistic locking — it's just the strictest non-interference predicate. The runtime still parallelizes everything that *doesn't* conflict; `EXCLUSIVE` only kicks in when two requirements *do* conflict.

## Mental model
`EXCLUSIVE` is the sequential-program contract: "as if I issued these operations one at a time on a single thread". The runtime works very hard to keep this illusion while exploiting all the parallelism the non-interference predicate (`non-interference.md`) allows. Where MPI codes write explicit synchronization, Legion gives you `EXCLUSIVE` for free as the default.

## Mechanism & API
`EXCLUSIVE` is the third argument of a `RegionRequirement` (the `prop` field):
```cpp
RegionRequirement(lr, READ_WRITE, EXCLUSIVE, lr);
```

It's the default — if you omit the coherence argument in convenience constructors, `EXCLUSIVE` is what you get.

**Behavior with each privilege**:
- `READ_ONLY` + `EXCLUSIVE`: multiple `READ_ONLY` readers run concurrently (they're non-interfering). Any writer waits for all readers to finish.
- `READ_WRITE` + `EXCLUSIVE`: full mutator, conflicts with everything else on overlapping data. The standard "this task updates the region" pattern.
- `WRITE_DISCARD` + `EXCLUSIVE`: same conflict semantics as `READ_WRITE` but skips the init-copy from prior writers.
- `REDUCE` + `EXCLUSIVE`: multiple same-op reducers run concurrently; different-op or non-reduce accesses serialize.

## Invariants
- `EXCLUSIVE` is **the runtime's default**. Application has no reason to specify it explicitly except for clarity.
- Any pair of requirements with `EXCLUSIVE` coherence on overlapping data is **fully ordered** by program-order issue.
- `EXCLUSIVE` does **not** prevent parallelism among non-conflicting operations — only among conflicting ones.
- Mixing `EXCLUSIVE` and a weaker coherence (`ATOMIC`/`SIMULTANEOUS`/`RELAXED`) on the same data: the runtime uses the **stronger** of the two (i.e., `EXCLUSIVE` wins) for the pair.
- A program correct under `EXCLUSIVE` is correct under any weaker coherence (assuming the application supplies the required synchronization for weaker modes). The reverse is not true.

## Performance implications
- **Use it as the default** — there's almost never a reason to weaken coherence unless a specific pattern (atomic counters, message buffers, hand-rolled shared state) requires it.
- Performance comes from the **other** non-interference axes (region disjointness, field disjointness, RO/RO or REDUCE/REDUCE). Coherence weakening is the last resort.
- Weakening coherence trades runtime synchronization for application synchronization — the application becomes responsible for what `EXCLUSIVE` would have handled automatically.

## Debug signals
- **`dataflow-graph.md`** edges between operations under `EXCLUSIVE` coherence are the runtime's expected serialization; check the privilege/coherence/field set on the edge label.
- **`-DPRIVILEGE_CHECKS`** confirms the application isn't reading/writing fields outside what the privilege allows; it doesn't directly check coherence.
- **Runs that work correctly with `EXCLUSIVE` and break under weaker coherence** = the application's synchronization for the weaker mode is broken.

## Failure modes
- Specifying `EXCLUSIVE` when weaker coherence would suffice rarely matters; the default is correct. The reverse — weakening unnecessarily — is the actual risk.

## Source pointers
- **Legion API**: https://github.com/StanfordLegion/legion/blob/master/runtime/legion/legion.h
- **Tutorial**: https://legion.stanford.edu/tutorial/privileges.html

## Related
- `wiki/concepts/coherence-mode.md` — umbrella for the four modes.
- `wiki/concepts/privilege.md` — the other half of every region requirement.
- `wiki/concepts/non-interference.md` — `EXCLUSIVE`'s contribution is the strictest version of the predicate.
- `wiki/concepts/region-requirement.md` — where coherence is set.

---
title: Regent Type System
slug: regent-type-system
summary: Regent's type system encodes Legion's contracts (privileges, region containment, partition structure) as static types; programs that the C++ API would catch at runtime are rejected at compile time.
tags: [data-model, dependence-analysis, for-program-reasoning, for-correctness-debug]
subsystem: regent
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/publications/publications.md
  - raw/youtube_transcripts/retreat_2024/transcripts/003_Legion_Retreat_2024_-_Regent_and_Pygion_-_Elliott_Slaughter.txt
  - raw/youtube_transcripts/bootcamp_2017/transcripts/010_Regent_Update_-_Legion_Bootcamp_2017_10_of_10.txt
github:
  - https://github.com/StanfordLegion/legion/tree/master/language
related:
  - wiki/concepts/regent-language.md
  - wiki/concepts/privilege.md
  - wiki/concepts/logical-region.md
  - wiki/concepts/partition.md
---

## TL;DR
Regent's type system makes Legion's runtime contracts into static types. A `region(ispace(int1d), float)` is a region type; `partition(disjoint, r, color_space)` is a partition type; `reads`/`writes`/`reduces` clauses on task signatures declare privileges as part of the function type. The compiler then **proves at compile time** that subtasks have a subset of their parent's privileges, that partitions are used disjointly, that region accesses stay in-bounds — guarantees that the C++ API can only check dynamically under `-DPRIVILEGE_CHECKS`. The confusion: types are checked, not enforced at runtime; you do still need `-lg:partcheck` when a partition declared disjoint actually depends on runtime data.

## Mental model
Regent's types are to Legion what Rust's lifetimes are to C++ pointers: the compiler tracks ownership and access through the type system, so misuse becomes a type error rather than runtime UB. You're not writing assertions; you're writing signatures that the compiler verifies.

## Mechanism & API
Key type forms (from the Regent paper `regent2015.pdf` + tutorial materials):

- **Index space types**: `ispace(int1d)`, `ispace(int3d)`, sparse variants. Parameterize on dimensionality and coordinate type.
- **Region types**: `region(ispace_t, element_t)` — the cross-product of an index space type and a field/element type.
- **Partition types**: `partition(kind, r, color_space)` where `kind` is `disjoint` / `aliased` / `complete`. Subregions inherit the type via `r[c]`.
- **Privilege clauses** in task signatures: `where reads(r), writes(r.x), reduces +(r.y) do ... end`. The compiler verifies that the task body only accesses what's declared.
- **Field paths**: `r.x` selects a specific field; `r.{x, y}` selects multiple. Privileges apply per field, mirroring `privilege.md`.
- **Coherence**: `where reads(r), atomic writes(r)` declares non-default coherence (`coherence-mode.md`).
- **Generics / metaprogramming** via Lua: type-level computations are arbitrary Lua code that runs at compile time.

Compile-time checks the type system performs:
- **Privilege subset**: any subtask call must request a subset of the caller's privileges; violations are type errors.
- **Region containment**: in-bounds access on a region whose bounds are known statically (e.g., a subregion of a disjoint partition).
- **Partition disjointness use**: the compiler can prove a `for` loop over a disjoint partition is non-interfering and emit an index launch.
- **Privilege escape**: returning a privilege from a task is rejected.

Dynamic checks remain for things the type system can't prove (per retreat 2024 transcript):
- Disjointness of a partition whose coloring is data-dependent (`-lg:partcheck`).
- Some accessor-bounds cases when the index space is dynamic.

## Invariants
- **All privilege checks the C++ API performs only at runtime** are subsumed by the Regent compiler when types are precise enough.
- A Regent program that **type-checks** may still have runtime bugs that the type system can't prove away — dynamic partitioning, aliased data accessed via raw pointers from foreign function calls, mapper-introduced inconsistencies. The C++ Legion runtime's correctness invariants apply.
- The compiler **emits a runnable Legion binary** that uses the same runtime as a C++ program — types don't change runtime semantics; they just gate what programs are accepted.
- Compiler errors **always point at the offending source line** with the failed constraint.

## Performance implications
- The type system is a **prerequisite for many Regent optimizations**: automatic index-launch detection, automatic predication, static control replication, all rely on statically known privilege and partition structure.
- Type-checked programs need fewer runtime checks (no `-DPRIVILEGE_CHECKS`, etc.); compile-time errors replace runtime asserts.
- **No runtime cost** from the type system itself — it's an elaboration step in the compiler.

## Debug signals
- **Compile error**: the type system tells you which privilege rule, region containment, or partition usage failed. First place to look.
- **Runtime crash on a compile-checked invariant** = compiler bug or escape via foreign function call.
- **Surprising `partcheck` failures** = the partition's disjointness was claimed but is data-dependent; the type checker took your word for it.

## Failure modes
- Returning a region/partition handle from a task: type error.
- Calling a subtask with privileges the parent doesn't hold: type error.
- Asserting `disjoint` on a partition that isn't: type-checks, fails at runtime under `-lg:partcheck` ([non-disjoint-disjoint-partition](../pitfalls/non-disjoint-disjoint-partition.md)).

## Source pointers
- **Compiler tree**: https://github.com/StanfordLegion/legion/tree/master/language
- **Paper (Regent design)**: `raw/publications/pdfs/regent2015.pdf` (SC 2015)
- **Paper (compiler-driven CR)**: `raw/publications/pdfs/cr2017.pdf` (SC 2017)
- **Theses**: `raw/publications/pdfs/slaughter_thesis.pdf`, `raw/publications/pdfs/lee_thesis.pdf`

## Related
- `wiki/concepts/regent-language.md` — host concept.
- `wiki/concepts/privilege.md` — what the type system encodes statically.
- `wiki/concepts/logical-region.md` — surface a `region(...)` type wraps.
- `wiki/concepts/partition.md` — and what `partition(disjoint, ...)` wraps.

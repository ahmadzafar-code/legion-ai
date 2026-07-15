---
title: Regent
slug: regent-language
summary: Stanford's compiled, statically-typed programming language for Legion; embeds in Lua/Terra, lowers to LLVM, and gives you the Legion programming model with native syntax plus compiler-driven performance optimizations.
tags: [execution, data-model, configuration, for-program-reasoning, for-perf-debug]
subsystem: regent
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/youtube_transcripts/retreat_2024/transcripts/003_Legion_Retreat_2024_-_Regent_and_Pygion_-_Elliott_Slaughter.txt
  - raw/youtube_transcripts/bootcamp_2017/transcripts/010_Regent_Update_-_Legion_Bootcamp_2017_10_of_10.txt
  - raw/publications/publications.md
github:
  - https://github.com/StanfordLegion/legion/tree/master/language
related:
  - wiki/concepts/task.md
  - wiki/concepts/regent-demand-directive.md
  - wiki/concepts/regent-type-system.md
  - wiki/concepts/control-replication.md
  - wiki/concepts/pygion.md
  - wiki/concepts/regent-mapper-dsl.md
---

## TL;DR
Regent is a compiled programming language whose semantics are the Legion programming model: tasks, regions, partitions, privileges. You write a function annotated as a task with typed region parameters, and the compiler emits the corresponding Legion `TaskVariantRegistrar` + `TaskLauncher` boilerplate, plus a static dependence analysis that can sometimes skip Legion's runtime analysis altogether. The compiler stack is **Regent → Terra → LLVM**, with `__demand` directives (`regent-demand-directive.md`) as the main perf knob. The confusion: Regent is not "Legion with sugar" — its type system encodes privileges and partition structure as types, so the compiler statically rejects programs that the C++ API would only catch at runtime under `-DPRIVILEGE_CHECKS`.

## Mental model
Think of Regent as Rust to Legion's C++: a language with a stronger type system that turns Legion's *contracts* (privileges, non-interference, region containment) into *types*. The compiler can then prove things statically — for example that an `__index_launch` is provably non-interfering — and emit code that doesn't pay runtime dependence-analysis cost. Same execution model as C++ Legion; same runtime; very different programmer ergonomics.

## Mechanism & API
**Stack** (per retreat 2024 transcript):
- Source: Regent (embedded in **Lua**; metaprogramming is just Lua code).
- Compiler target: **Terra** (a low-level, statically-typed metaprogramming layer).
- Lowered to: **LLVM** (yields native CPU code, NVIDIA PTX, or AMD GPU bitcode).
- Runtime: standard Legion + Realm.

**Hello-world pattern** (illustrative; not verbatim Regent syntax):
```
task hello(r : region(ispace(int1d), float))
where reads writes(r) do
  for p in r do r[p] = drand48() end
end
```
The `where` clause declares privileges; the type `region(ispace(int1d), float)` carries index space + field type.

**Key compiler optimizations** (transcripts above):
- **Static control replication** historically — compiled SPMD-style programs from a sequential top-level task (SC 2017, `cr2017.pdf`). Now superseded by **dynamic control replication** in the runtime, but Regent still triggers it automatically (`__demand(__replicable)`).
- **Automatic `__index_launch` detection**: a `for` loop launching identical tasks gets compiled into a single `IndexLauncher`.
- **Automatic predication**: assignments inside `if` blocks become predicated task launches with chained predicates.
- **GPU code generation**: `__demand(__cuda)` on a task triggers automatic CUDA emission via LLVM's NVPTX/AMDGPU backends. AMD support requires LLVM 18 specifically (per retreat 2024).
- **Auto-fusion** and **constraint inference** (paper `parallelizer2019.pdf` covers related work).

The Regent compiler lives in `language/` in the Legion repo. Build and use it as a standalone tool that produces a runnable Legion binary.

## Invariants
- Regent programs **must** follow Legion semantics — privileges, region containment, partition disjointness — at compile time via the type system, plus dynamic checks for things the type system can't prove.
- The compiled output **uses the same Legion + Realm runtime** as a C++ Legion program; performance characteristics, debugging tools (Legion Prof, Legion Spy), and configuration flags all apply.
- **There is a lag** between Legion C++ runtime features landing and Regent supporting them; bleeding-edge users may need C++.
- Regent compiles via Lua/Terra/LLVM; debugging the compiler itself requires Lua and Terra knowledge.
- A Regent program **is** a Legion program at the ABI level — the two interoperate via Pygion or direct linking.

## Performance implications
- **Auto-index-launch and auto-predication** mean a naive Regent program often outperforms an idiomatic-but-naive C++ Legion program — the compiler does what a careful C++ developer would do manually.
- **Static analysis caps runtime overhead** — for highly-distributed runs the cost of dependence analysis stays roughly constant in node count (bootcamp 2017 transcript), enabling weak scaling to thousands of nodes (S3D ran to 8,000 Frontier nodes per retreat 2024).
- For **fine-grained tasks** Regent still hits the runtime-analysis floor; this is the next frontier.
- **`__demand` directives** (`regent-demand-directive.md`) are the primary perf knobs; an unmarked Regent program is correct but may not be optimized.

## Debug signals
- The Regent compiler emits **type errors at compile time** for privilege misuse, region escape, and similar — earlier than C++ Legion's runtime checks.
- The compiled binary's runtime behavior is debugged exactly like a C++ Legion binary: Legion Prof, Legion Spy, mapper logs, `LEGION_BACKTRACE=1`.
- **`__demand` directive failures** appear as compiler diagnostics naming the directive that couldn't be satisfied (e.g., a task body that doesn't qualify for `__leaf`).

## Failure modes
- Same as C++ Legion — false dependencies, GPU underutilization, etc. — see `wiki/pitfalls/`. The major difference is Regent's compiler can prevent some of them before runtime.

## Source pointers
- **Compiler tree**: https://github.com/StanfordLegion/legion/tree/master/language
- **Paper (Regent)**: `raw/publications/pdfs/regent2015.pdf` (SC 2015)
- **Paper (Static Control Replication, Regent backend)**: `raw/publications/pdfs/cr2017.pdf` (SC 2017)
- **Paper (Constraint-Based Auto Partitioning)**: `raw/publications/pdfs/parallelizer2019.pdf` (SC 2019)
- **Talks**: `raw/youtube_transcripts/retreat_2024/transcripts/003_..._Regent_and_Pygion_-_Elliott_Slaughter.txt`; `raw/youtube_transcripts/bootcamp_2017/transcripts/010_Regent_Update_...txt`

## Related
- `wiki/concepts/task.md` — what Regent functions compile to.
- `wiki/concepts/regent-demand-directive.md` — the perf-control surface.
- `wiki/concepts/regent-type-system.md` — how Regent enforces Legion's contracts at compile time.
- `wiki/concepts/control-replication.md` — Regent triggers replication automatically.
- `wiki/concepts/pygion.md` — Python sibling for the same programming model.

---
title: Regent Mapper DSL
slug: regent-mapper-dsl
summary: Regent's in-language mapping interface; per-task `__demand` directives plus a declarative mapping section that compiles to standard Legion mapper callbacks, eliminating the C++ subclassing boilerplate.
tags: [mapping, configuration, for-perf-debug]
subsystem: regent
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/youtube_transcripts/retreat_2024/transcripts/003_Legion_Retreat_2024_-_Regent_and_Pygion_-_Elliott_Slaughter.txt
  - raw/youtube_transcripts/bootcamp_2017/transcripts/006_Performance_Tuning_via_Mapping_-_Legion_Bootcamp_2017_6_of_10.txt
  - raw/publications/publications.md
github:
  - https://github.com/StanfordLegion/legion/tree/master/language
related:
  - wiki/concepts/regent-language.md
  - wiki/concepts/regent-demand-directive.md
  - wiki/concepts/mapper.md
  - wiki/concepts/default-mapper.md
  - wiki/concepts/automated-mapping.md
---

## TL;DR
Regent's mapper DSL is the in-language replacement for writing a C++ mapper subclass. Two complementary mechanisms: per-task `__demand` directives (`regent-demand-directive.md`) that constrain placement at compile time, and a declarative mapping section in the program that names processor kinds and instance memories — Regent's compiler generates the corresponding `select_task_options`/`map_task`/`slice_task` callbacks. The confusion: there's no single "Regent mapper file"; mapping is **declarative metadata attached to the program**, lowered to standard Legion mapper callbacks the runtime calls. For workloads where `default-mapper.md` is wrong but a hand-written C++ mapper is overkill, the Regent DSL is the middle ground.

## Mental model
Regent's mapper DSL is Pragma-as-Mapping: place a few annotations on tasks and partitions, the compiler emits the mapper that interprets them. Where C++ Legion users write a `MyMapper : DefaultMapper` class with overridden callbacks, Regent users add `__demand(__cuda)` to a task and a mapping clause that says "this partition goes on the GPU framebuffer". The compiler does the rest.

## Mechanism & API

**Per-task constraints** (the most-used surface) come from `regent-demand-directive.md`:
- `__demand(__cuda)` — task must run on `TOC_PROC`; constraints applied to chosen processor.
- `__demand(__leaf)` / `__demand(__inner)` / `__demand(__replicable)` — task-property constraints (mirror `leaf-task.md`, `inner-task.md`, `replicable-task.md`).
- `__demand(__index_launch)` — the enclosing loop must compile to a single `IndexLauncher` (mirror `index-space-launch.md`).
- `__demand(__vectorize)` — emit vectorized inner-loop code.

**Declarative mapping** (the surface for instance placement and processor selection): Regent programs include a mapping block — typically a small DSL of "for this task, on this processor kind, allocate these regions in this memory kind". The compiler emits the corresponding `select_task_options.md` (initial proc) and `map-task.md` (chosen instances + variant) callbacks.

**Automated mapping** (paper `automap2023.pdf`, see `automated-mapping.md`) integrates with Regent: where the DSL author would write the mapping clause by hand, the automated mapper infers it from the task graph + machine model. Increasingly the default path for Regent users who don't want to hand-tune.

**Fallback to C++**: Regent programs can register a custom C++ mapper alongside the DSL-generated one — useful when the DSL doesn't express what the application needs. The C++ mapper takes precedence for tasks routed to its `MapperID`.

## Invariants
- The DSL **lowers to the same Legion mapper interface** every C++ mapper uses; performance characteristics, debugging tools, and constraints are identical to a hand-written mapper.
- `__demand` directives are **contractual** — the compiler must satisfy them or refuse to compile.
- Mapping decisions made by the DSL are **deterministic** across runs (assuming deterministic input), avoiding `pitfalls/mapper-bouncing.md`.
- The DSL participates in **control replication** (`control-replication.md`) naturally — `__demand(__replicable)` opts the top-level task in.
- Debugging is via the same tools: `mapper-logging.md`, `-level mapper=2`, `legion-prof.md`.

## Performance implications
- **Lower friction than a custom C++ mapper**, with comparable performance for the workloads the DSL covers.
- The Regent compiler can fold mapping decisions into trace-cached templates (`tracing.md`), giving an additional win.
- For workloads with irregular load balance or application-specific placement logic, hand-written C++ mappers still win. The DSL is the middle of the curve.
- Combined with **automated mapping** (`automated-mapping.md`), the DSL becomes nearly maintenance-free for typical scientific workloads — you don't even write the mapping clause.

## Debug signals
- **Standard mapper-debug toolchain applies**: wrap the generated mapper with `LoggingWrapper` (`logging-wrapper.md`) and run with `-level mapper=2` to see decisions.
- **Compile-time errors from `__demand`** point at directives the compiler can't satisfy — the message names the directive.
- **Suboptimal placement** despite DSL annotations → check whether `default-mapper.md` is shadowing the DSL-generated mapper. Each task launch's `MapperID` decides which mapper handles it.

## Failure modes
- DSL doesn't express what the application needs → fall back to a hand-written C++ mapper for the affected task IDs.
- Conflicting `__demand` directives → compile error; fix one.

## Source pointers
- **Regent compiler tree**: https://github.com/StanfordLegion/legion/tree/master/language
- **Transcript (Regent overview)**: `raw/youtube_transcripts/retreat_2024/transcripts/003_..._Regent_and_Pygion_-_Elliott_Slaughter.txt`
- **Transcript (perf tuning via mapping)**: `raw/youtube_transcripts/bootcamp_2017/transcripts/006_Performance_Tuning_via_Mapping_-_Legion_Bootcamp_2017_6_of_10.txt`
- **Paper (automated mapping)**: `raw/publications/pdfs/automap2023.pdf`

## Related
- `wiki/concepts/regent-language.md` — host language.
- `wiki/concepts/regent-demand-directive.md` — the per-task constraint surface.
- `wiki/concepts/mapper.md` — what the DSL lowers to.
- `wiki/concepts/default-mapper.md` — the fallback if the DSL is silent.
- `wiki/concepts/automated-mapping.md` — the inference-based alternative.

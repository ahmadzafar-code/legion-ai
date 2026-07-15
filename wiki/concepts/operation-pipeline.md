---
title: Operation Pipeline
slug: operation-pipeline
summary: The seven-stage out-of-order pipeline every Legion operation flows through, from API call to completion event; the spine that ties tasks, dependence analysis, mapping, and execution together.
tags: [execution, dependence-analysis, mapping, for-program-reasoning, for-perf-debug]
subsystem: legion
layer: runtime-internals
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/youtube_transcripts/runtime_school_2023/transcripts/001_Legion_Runtime_Internals_-_Lesson_1_-_The_Operation_Pipeline.txt
  - raw/youtube_transcripts/runtime_school_2023/transcripts/002_Legion_Runtime_Internals_-_Lesson_2_-_Tasks_Context_and_Forward_Progress.txt
  - raw/youtube_transcripts/runtime_school_2023/transcripts/003_Legion_Runtime_Internals_-_Lesson_3_-_Scheduling_and_Mapper_Calls.txt
github:
  - https://github.com/StanfordLegion/legion/tree/master/runtime/legion
related:
  - wiki/concepts/task.md
  - wiki/concepts/privilege.md
  - wiki/concepts/mapper.md
  - wiki/concepts/tracing.md
  - wiki/concepts/event.md
  - wiki/concepts/dependence-analysis.md
  - wiki/concepts/logical-analysis.md
  - wiki/concepts/physical-analysis.md
  - wiki/concepts/index-space-launch.md
  - wiki/concepts/leaf-task.md
---

## TL;DR
Every operation a Legion application creates — a task launch, a copy, a fill, a partition, a fence — enters a 7-stage pipeline inside the runtime and flows through it asynchronously. The stages are *dispatch → dependence analysis → ready → mapping → physical analysis → execution → completion*. Understanding which stage your bottleneck lives in is the foundation of all Legion perf debugging. The confusion: the pipeline is asynchronous, so an operation's API call returns immediately even though it may take milliseconds to mapping or seconds to execution.

## Mental model
> "Legion operates like an out-of-order processor. It has a seven-stage pipeline. … For anyone that's ever implemented an out-of-order multi-stage pipeline processor in Verilog or something, this'll look incredibly familiar." — Runtime School 2023, Lesson 1.

Region requirements are operands. Privileges define hazard types (RAW/WAR/WAW). The pipeline is the issue/dispatch logic. Mapping is register renaming + placement. Execution is the functional units. Each stage has its own queues and back-pressure (controlled by `-lg:window`, `-lg:sched`, `-lg:width`).

## Mechanism & API
The seven stages (names from the Runtime School transcript):

1. **API dispatch.** Application calls `execute_task` / `execute_index_space` / `issue_copy` / etc. The call is a trampoline through `legion.cc` into the parent task's `TaskContext`. The op is constructed and assigned a unique ID; the call returns a `Future`.
2. **Dependence analysis** (logical analysis). The op is walked through the **region tree** and compared against all currently-outstanding ops. Pairwise non-interference produces an *operation DAG*. This stage is what the Logging level `tasks` and Legion Spy logical-analysis output reveal.
3. **Ready queue**. Once all its logical predecessors have themselves been mapped (and physical preconditions are computable), the op moves to ready.
4. **Mapping.** Mapper callbacks fire (`select_task_options`, `slice_task`, `map_task`, …). The mapper picks processors, instances, and variants. See `mapper.md`.
5. **Physical analysis.** The runtime computes the actual copies/fills/reductions needed to make the chosen physical instances valid for this op. The result is a set of Realm `Event`s.
6. **Execution.** The Realm task runs on the chosen processor; the task body executes; for index launches, point tasks fan out.
7. **Completion.** Postconditions trigger; the `Future` is filled; the op's effects become visible to logical successors.

Tracing (`tracing.md`) memoizes stages 2–5 across repeated trace bodies; control replication (`control-replication.md`) splits stages 1–5 across shards.

## Invariants
- Stages run **asynchronously** and **pipelined**: an op can be in execution while the next op behind it is still in dependence analysis.
- Logical analysis runs in **program order** within a context (no reordering across the API boundary), but physical analysis and execution are out-of-order.
- The **utility processors** (`-ll:util`) run stages 2, 4, 5, and parts of 7. Saturating them stalls the whole pipeline.
- `-lg:window` caps how many ops can be in flight at stages 1–3 (the "instruction window"). When full, the issuing task blocks.
- Forward-progress invariant: an op cannot proceed to stage N+1 until all predecessors have reached stage N+1 in some form (Runtime School L2). This is what makes Legion legal as an out-of-order machine.

## Performance implications
- **Stage 2 (dep analysis)** scales with #ops × #region requirements × tree depth. Over-launching tiny tasks here costs more than executing them. Use index launches.
- **Stage 4 (mapping)** runs the mapper callbacks. Slow mappers stall here; the symptom in Legion Prof is busy utility-processor rows while application rows idle.
- **Stage 5 (physical analysis)** dominates when there's much instance copying; the cost is mostly visible as channel-row activity in Legion Prof.
- **Tracing** is how you collapse stages 2 and 5 to near-zero on repeated patterns — see `tracing.md`.
- Tuning flags: `-lg:window`, `-lg:sched`, `-lg:width`, `-lg:filter`, `-ll:util`.

## Debug signals
- **Legion Prof**: separate rows for application processors and utility processors. Idle app rows + busy util rows = stuck in stages 2/4/5.
- **Critical path view** (press `a` in Legion Prof): the longest chain through the pipeline. The stage where time is spent is the bottleneck.
- **`-level legion=2`**: per-op transitions through stages get logged.
- **Legion Spy event graph** (`-lg:spy -e`): shows the post-mapping event structure (stages 5–7).

## Failure modes
- [Long dependence chains](../pitfalls/long-dependence-chains.md) — Stage 2 produces a chain instead of a fan-out.
- [Mapper stalls](../pitfalls/mapper-stalls.md) — Stage 4 saturated.
- [Runtime overhead dominates](../pitfalls/runtime-overhead-dominates.md) — Stages 1–3 cost more than execution.

## Source pointers
- **Runtime tree** (operation classes): https://github.com/StanfordLegion/legion/tree/master/runtime/legion
- **Tutorial**: https://legion.stanford.edu/tutorial/index_tasks.html
- **Lectures (deep dive)**: `raw/youtube_transcripts/runtime_school_2023/` (Lessons 1–14)

## Related
- `wiki/concepts/task.md` — what flows through the pipeline.
- `wiki/concepts/privilege.md` — what stage 2 compares.
- `wiki/concepts/mapper.md` — what fires at stage 4.
- `wiki/concepts/event.md` — what stages 5–7 produce.
- `wiki/concepts/tracing.md` — how to skip stages 2 and 5.
- `wiki/concepts/control-replication.md` — how to scale stages 1–5 across nodes.

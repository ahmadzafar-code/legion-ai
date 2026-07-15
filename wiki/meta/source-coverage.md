# Source Coverage

Inverse index: for each `raw/` file, which wiki pages cite it. Drives the "what haven't we ingested yet?" question. Updated on every ingest.

## website-pages/

- `raw/website-pages/overview.md` → `wiki/concepts/logical-region.md`, `wiki/concepts/task.md`, `wiki/concepts/privilege.md`
- `raw/website-pages/mapper.md` → `wiki/concepts/mapper.md`, `wiki/concepts/physical-instance.md`, `wiki/concepts/mapper-context.md`, `wiki/concepts/select-task-options.md`, `wiki/concepts/slice-task.md`, `wiki/concepts/map-task.md`, `wiki/concepts/mapper-callback.md`
- `raw/website-pages/debugging.md` → `wiki/concepts/privilege.md`, `wiki/concepts/legion-spy.md`, `wiki/concepts/event.md`, `wiki/concepts/partition.md`
- `raw/website-pages/profiling.md` → `wiki/concepts/legion-prof.md`, `wiki/concepts/physical-instance.md`
- `raw/website-pages/error_messages.md` → `wiki/concepts/error-message-catalog.md`
- `raw/website-pages/gasnet.md` → **not yet ingested** (pending: `gasnet`, `gasnetex`)
- `raw/website-pages/getting_started.md` → **not yet ingested**
- `raw/website-pages/community.md` → **not yet ingested**
- `raw/website-pages/documentation.md` → **not yet ingested**

## tutorials/

- `raw/tutorials/02_tasks_and_futures.md` → `wiki/concepts/task.md`, `wiki/concepts/future.md`, `wiki/concepts/future-map.md`, `wiki/concepts/task-variant.md`, `wiki/concepts/task-launcher.md`, `wiki/concepts/leaf-task.md`
- `raw/tutorials/03_index_tasks.md` → `wiki/concepts/index-space-launch.md`, `wiki/concepts/future-map.md`, `wiki/concepts/argument-map.md`, `wiki/concepts/task-launcher.md`
- `raw/tutorials/04_hybrid_model.md` → `wiki/concepts/leaf-task.md`, `wiki/concepts/task-variant.md`
- `raw/tutorials/realm_02_machine_model.md` → `wiki/concepts/realm-machine-model.md`
- `raw/tutorials/realm_04_region_instances.md` → `wiki/concepts/region-instance.md`
- `raw/tutorials/realm_07_copies_and_fills.md` → `wiki/concepts/dma-system.md`
- `raw/tutorials/realm_11_reservations.md` → `wiki/concepts/reservation.md`
- `raw/tutorials/realm_03_events.md` → `wiki/concepts/user-event.md`, `wiki/concepts/event-poisoning.md`
- `raw/tutorials/realm_10_completion_queue.md` → `wiki/concepts/completion-queue.md`
- `raw/tutorials/realm_12_barriers.md` → `wiki/concepts/realm-barrier.md`
- `raw/tutorials/realm_13_profiling.md` → `wiki/concepts/realm-profiling.md`
- `raw/tutorials/realm_14_cuda_interop.md` → `wiki/concepts/cuda-interop.md`
- `raw/tutorials/05_logical_regions.md` → `wiki/concepts/logical-region.md`
- `raw/tutorials/06_physical_regions.md` → `wiki/concepts/physical-instance.md`
- `raw/tutorials/07_privileges.md` → `wiki/concepts/privilege.md`, `wiki/concepts/coherence-mode.md`, `wiki/concepts/region-requirement.md`
- `raw/tutorials/08_partitioning.md` → `wiki/concepts/partition.md`, `wiki/concepts/index-space-launch.md`, `wiki/concepts/region-requirement.md`
- `raw/tutorials/10_custom_mappers.md` → `wiki/concepts/mapper.md`, `wiki/concepts/default-mapper.md`, `wiki/concepts/tunable-variable.md`, `wiki/concepts/mapper-context.md`, `wiki/concepts/select-task-options.md`, `wiki/concepts/slice-task.md`, `wiki/concepts/map-task.md`, `wiki/concepts/mapper-callback.md`
- `raw/tutorials/00_tutorial_index.md` → `wiki/concepts/event.md`
- Other tutorials (01, 04, 09) → **not yet ingested**
- `raw/tutorials/11_circuit_simulation.md` → `wiki/applications/circuit.md`
- Most Realm tutorials covered after batch 23; remaining pending: `realm_01`, `realm_05_06_08_09_15+` and subgraph/indirect-copy specifics → **not yet ingested**

## publications/

- `raw/publications/publications.md` → `wiki/concepts/logical-region.md`, `wiki/concepts/task.md`, `wiki/concepts/partition.md`, `wiki/concepts/control-replication.md`, `wiki/concepts/tracing.md`, `wiki/concepts/event.md`, `wiki/concepts/physical-instance.md`, `wiki/concepts/coherence-mode.md`, `wiki/concepts/dependence-analysis.md`, `wiki/concepts/index-space-launch.md`
- `raw/publications/pdfs/slaughter_thesis.pdf` → `wiki/applications/miniaero.md`, `wiki/applications/pennant.md` (§8.1.3, §8.3.3, Fig 8.7 — MiniAero weak-scaling to 1024 nodes; Pennant CR scaling)
- `raw/publications/pdfs/regent2015.pdf` → `wiki/applications/miniaero.md` (§6.3 — hybrid SOA-AOS 2.8× single-node speedup)
- `raw/publications/pdfs/cr2017.pdf` → `wiki/applications/pennant.md`, `wiki/applications/miniaero.md` (§5.2-5.3 — CR scaling benchmarks)
- `raw/publications/pdfs/dpl2016.pdf` → `wiki/applications/pennant.md` (case study: 96% LOC reduction)
- `raw/publications/pdfs/trace2018.pdf` (referenced via `publications.md`) → `wiki/applications/pennant.md`, `wiki/applications/circuit.md`, `wiki/applications/miniaero.md` (5-app tracing benchmark: Pennant 2.8×, Circuit/MiniAero 4.2×+)
- **Gap**: `publications.md` has no numbered entry for MiniAero specifically; canon lives in Slaughter thesis + Regent SC2015 + CR SC2017.
- Individual PDFs partially ingested via batch 22 + batch 24; remaining PDFs not yet ingested per page.

## youtube_transcripts/

- `raw/youtube_transcripts/runtime_school_2023/transcripts/001_..._Operation_Pipeline.txt` → `wiki/concepts/operation-pipeline.md`, `wiki/concepts/task.md`
- `raw/youtube_transcripts/runtime_school_2023/transcripts/002_..._Tasks_Context_and_Forward_Progress.txt` → `wiki/concepts/operation-pipeline.md`, `wiki/concepts/leaf-task.md`
- `raw/youtube_transcripts/runtime_school_2023/transcripts/003_..._Scheduling_and_Mapper_Calls.txt` → `wiki/concepts/operation-pipeline.md`, `wiki/concepts/mapper-callback.md`
- `raw/youtube_transcripts/runtime_school_2023/transcripts/005_..._Distributed_Collectable_Objects.txt` → `wiki/concepts/region-tree.md`
- `raw/youtube_transcripts/runtime_school_2023/transcripts/006_..._Region_Tree_Nodes_and_Reference_Counting_Invariants.txt` → `wiki/concepts/region-tree.md`
- `raw/youtube_transcripts/runtime_school_2023/transcripts/007_..._Logical_Dependence_Analysis.txt` → `wiki/concepts/dependence-analysis.md`, `wiki/concepts/logical-analysis.md`
- `raw/youtube_transcripts/runtime_school_2023/transcripts/008_..._Logical_Dependence_Analysis_Part_2.txt` → `wiki/concepts/logical-analysis.md`
- `raw/youtube_transcripts/runtime_school_2023/transcripts/009_..._Physical_Analysis_Part_1.txt` → `wiki/concepts/dependence-analysis.md`, `wiki/concepts/physical-analysis.md`, `wiki/concepts/equivalence-set.md`
- `raw/youtube_transcripts/runtime_school_2023/transcripts/010_..._Physical_Analysis_Part_2.txt` → `wiki/concepts/physical-analysis.md`
- `raw/youtube_transcripts/runtime_school_2023/transcripts/016_..._Control_Replication_Part_1.txt` → `wiki/concepts/control-replication.md`
- `raw/youtube_transcripts/runtime_school_2023/transcripts/019_..._Control_Replication_Part_4.txt` → `wiki/concepts/sharding-functor.md`
- `raw/youtube_transcripts/runtime_school_2023/transcripts/021_..._Tracing_Part_1.txt` → `wiki/concepts/tracing.md`, `wiki/concepts/dynamic-tracing.md`, `wiki/concepts/static-tracing.md`
- `raw/youtube_transcripts/runtime_school_2023/transcripts/022_..._Tracing_Part_2.txt` → `wiki/concepts/trace-recording.md`, `wiki/concepts/trace-replay.md`, `wiki/concepts/dynamic-tracing.md`
- `raw/youtube_transcripts/runtime_school_2023/transcripts/023_..._Tracing_Part_3.txt` → `wiki/concepts/trace-recording.md`, `wiki/concepts/trace-replay.md`
- `raw/youtube_transcripts/retreat_2024/transcripts/003_..._Regent_and_Pygion_-_Elliott_Slaughter.txt` → `wiki/concepts/regent-language.md`, `wiki/concepts/regent-demand-directive.md`, `wiki/concepts/regent-type-system.md`, `wiki/concepts/pygion.md`
- `raw/youtube_transcripts/retreat_2024/transcripts/017_..._Debugging_Legion_Applications_-_Michael_Bauer.txt` → `wiki/applications/pennant.md` (live Legion Prof debug demo using Pennant; mapper-stalls + multi-hop-copy anomalies)
- `raw/youtube_transcripts/bootcamp_2017/transcripts/010_Regent_Update_-_Legion_Bootcamp_2017_10_of_10.txt` → `wiki/concepts/regent-language.md`, `wiki/concepts/regent-demand-directive.md`, `wiki/concepts/regent-type-system.md`
- `raw/youtube_transcripts/bootcamp_2017/transcripts/` → `wiki/concepts/legion-prof.md` (Lesson 8 referenced)
- `raw/youtube_transcripts/retreat_2024/transcripts/` → `wiki/concepts/legion-prof.md` (debugging talk referenced)
- All other transcripts → **not yet ingested**

## retreats/

- `raw/retreats/retreat_2024.md`, `raw/retreats/retreat_2022.md`, `raw/retreats/retreat_2021.md`, `raw/retreats/bootcamp_*.md` → **not yet ingested**

## resources/

- `raw/resources/resources.md` → **not yet ingested**

## legion_applications/

- `raw/legion_applications/` → **not yet ingested**

## Pending high-priority ingest queue

Order: any `for-perf-debug`-tagged concept first; then `for-program-reasoning`; then `for-correctness-debug`.

Completed in batch 1 (2026-05-15): coherence-mode, dependence-analysis, logical-analysis, physical-analysis, index-space-launch.
Completed in batch 2 (2026-05-15): default-mapper, sharding-functor, automatic-tracing, equivalence-set, region-tree.
Completed in batch 3 (2026-05-15): future, future-map, region-instance, dma-system, reservation, realm-machine-model.
Completed in batch 4 (2026-05-15): regent-language, regent-demand-directive, regent-type-system, pygion, tunable-variable. User asked to skip-Realm for this batch; Regent+Pygion subsystem expansion landed instead.
Completed in batch 5 (2026-05-15): task-variant, task-launcher, region-requirement (promoted from glossary stub), argument-map, leaf-task.
Completed in batch 6 (2026-05-15): dynamic-tracing, static-tracing, trace-recording, trace-replay, visibility-algorithm.
Completed in batch 7 (2026-05-15): mapper-context, select-task-options, slice-task, map-task, mapper-callback.

Next up (skipping Realm per user direction):
1. `inner-task` + `replicable-task` — finish the leaf/inner/replicable triumvirate.
3. `non-interference` + `field-level-non-interference` — promote from inline definitions in `privilege.md` and `partition.md`.
4. `read-only-privilege` / `read-write-privilege` / `write-discard-privilege` / `reduce-privilege` — split out specific privilege pages.
5. `regent-mapper-dsl` + `pygion-decorators` — round out the high-level-language subsystem.
6. `instance-layout` + `virtual-mapping` + `reduction-instance` — Legion physical-instance variants from the pillar's "Other instance kinds" subsection.
7. `gasnet` + `gasnetex` + `active-message` — the system/distributed subsystem (currently uncovered).
(Realm pending: barriers, user-events, completion-queue, subgraphs, cuda-interop — held back per user direction.)

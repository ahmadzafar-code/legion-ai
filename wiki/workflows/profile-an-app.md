---
title: Profile a Legion App
slug: profile-an-app
summary: End-to-end recipe for capturing and viewing a Legion Prof profile, configured for accurate measurement rather than debug introspection.
tags: [for-perf-debug, profiling, tooling]
status: draft
created: 2026-05-15
updated: 2026-05-15
related:
  - wiki/concepts/legion-prof.md
  - wiki/concepts/legion-spy.md
  - wiki/workflows/debug-perf-bottleneck.md
---

## Inputs
- A buildable Legion application.
- A target node count `N`.
- (Optional) a representative input that runs in 1–10 seconds — long enough to see steady state, short enough to iterate.

## Steps
1. **Build in release mode.**
   ```bash
   DEBUG=0 make
   ```
   Strip all check flags from the build: `-DPRIVILEGE_CHECKS`, `-DBOUNDS_CHECKS`, `-DLEGION_SPY`. These are correctness tools, not perf tools, and they distort timings dramatically.

2. **Install `legion_prof_rs` from the matching Legion commit.**
   ```bash
   cargo install --locked --all-features --path legion/tools/legion_prof_rs
   ```
   Version skew between profiler and runtime yields silently wrong views.

3. **Run with profiling enabled.**
   ```bash
   ./app -lg:prof <N> -lg:prof_logfile prof_%.gz <app-args>
   ```
   The `%` is replaced by node index, producing one log file per node.

4. **View.** Pick one:
   - `legion_prof --view prof_*.gz` — local desktop UI.
   - `legion_prof --archive prof_*.gz -o out/` — shareable web archive.
   - `legion_prof --serve prof_*.gz` and `--attach` for remote viewing.

5. **Scan the timeline.** Look in this order:
   - **Critical path** (press `a`): how long, what's on it.
   - **Idle gaps** on application processor rows: see "Cause" mapping in `wiki/workflows/debug-perf-bottleneck.md`.
   - **Channel rows**: how busy, between which memories.
   - **Utility processor rows**: if saturated, runtime overhead or mapper is the bottleneck.

6. **(Optional) pair with Legion Spy** for a causality view of the same run:
   ```bash
   ./app -lg:prof <N> -lg:spy -lg:prof_logfile prof_%.gz -logfile spy_%.log <app-args>
   legion/tools/legion_spy.py -dez spy_*.log
   ```
   Note Spy adds overhead; A/B against a prof-only run if absolute timings matter.

## Outputs
- A directory of gzipped per-node profile logs.
- An interactive timeline view (or a shareable archive).
- A first-pass hypothesis about which pitfall best matches the timeline.

## When to use
- Any time the application is slower than expected.
- Whenever you change the mapper, the partition layout, the privilege declarations, or the trace markers — to confirm the change had the intended effect.
- Before reporting a perf issue to the Legion maintainers — they will ask for a profile.

## Related
- `wiki/concepts/legion-prof.md` — the tool reference.
- `wiki/concepts/legion-spy.md` — companion tool for causality.
- `wiki/workflows/debug-perf-bottleneck.md` — what to do with what the profile shows.
- `wiki/workflows/enable-tracing.md` — common perf-debug remediation.
- `wiki/workflows/write-a-custom-mapper.md` — common perf-debug remediation.
- `wiki/workflows/move-from-single-node-to-distributed.md` — scaling workflow.
- `wiki/workflows/debug-correctness-bug.md` — sister workflow for correctness issues.

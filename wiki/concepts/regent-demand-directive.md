---
title: Regent __demand Directives
slug: regent-demand-directive
summary: Regent's pragma-like annotations (`__demand(__cuda)`, `__demand(__index_launch)`, `__demand(__inner)`, ...) that request specific compiler optimizations or task properties and fail compilation if the requested guarantee can't be provided.
tags: [execution, configuration, gpu, for-perf-debug]
subsystem: regent
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/youtube_transcripts/retreat_2024/transcripts/003_Legion_Retreat_2024_-_Regent_and_Pygion_-_Elliott_Slaughter.txt
  - raw/youtube_transcripts/bootcamp_2017/transcripts/010_Regent_Update_-_Legion_Bootcamp_2017_10_of_10.txt
github:
  - https://github.com/StanfordLegion/legion/tree/master/language
related:
  - wiki/concepts/regent-language.md
  - wiki/concepts/task.md
  - wiki/concepts/index-space-launch.md
  - wiki/concepts/control-replication.md
  - wiki/applications/miniaero.md
  - wiki/applications/pennant.md
---

## TL;DR
`__demand` directives are Regent's "fail-loud" optimization hints: write `__demand(__cuda) task ...` and the compiler must produce a CUDA-runnable task variant or refuse to compile. Unlike C-style `#pragma` (advisory, may be ignored), Regent's `__demand` is **contractual** — the program either gets the requested optimization or errors out. The confusion: these are not assertions about runtime behavior — they're commands to the **compiler** about what code to emit and what static properties to enforce.

## Mental model
Think of `__demand` like Rust's `#[derive(Send, Sync)]`: a request that some property hold, with a compile-time error if it doesn't. Unlike `#[inline]` (a hint), `__demand` is what you reach for when you can't ship if the optimization didn't happen — e.g., a GPU code path where falling back to CPU silently would mask a bug.

## Mechanism & API
The Regent compiler recognizes several `__demand` directives. From the transcripts and Regent docs:

- **`__demand(__cuda)`** — emit a CUDA task variant (or AMDGPU; the same directive covers both, via LLVM's NVPTX and AMDGPU backends). Compile fails if the task body can't be lowered to GPU code.
- **`__demand(__index_launch)`** — the enclosing `for` loop must be compiled into a single `IndexLauncher`. Useful when the compiler's auto-detection might silently fall back to per-iteration launches.
- **`__demand(__inner)`** — the task must be an **inner task** (launches subtasks but does not access region instances directly). Enables virtual mapping.
- **`__demand(__leaf)`** — the task must be a **leaf task** (launches no subtasks). Enables the optimized leaf-context fast path.
- **`__demand(__replicable)`** — the task must be control-replicable (deterministic given inputs; suitable for SPMD shard execution). See `control-replication.md`.
- **`__demand(__vectorize)`** — emit vector-instruction code paths where the compiler can prove it's safe.

Usage:
```
__demand(__cuda)
task saxpy(x : region(ispace(int1d), float),
           y : region(ispace(int1d), float),
           a : float)
where reads(x), reads writes(y) do
  for p in x do y[p] = a * x[p] + y[p] end
end
```

The transcripts highlight that **the compiler does much of this automatically** even without `__demand` — e.g., loop iterations launching identical tasks get auto-promoted to index launches, and assignments inside `if` blocks get auto-predicated. The directive's value is to **make failure explicit** when the optimization is required for correctness or perf.

## Invariants
- `__demand` is a **compile-time constraint**. The compiler emits the requested form or errors out — never silently ignores it.
- Multiple directives can compose: `__demand(__cuda) __demand(__leaf)` is a leaf GPU task.
- A `__demand(__index_launch)` failure typically points at a non-uniform loop body (different launchers per iteration) or a hidden interference the compiler can't prove away.
- A `__demand(__cuda)` failure usually means the body uses unsupported features (host-only calls, non-leaf launches, allocations).
- `__demand` does not change **runtime behavior** beyond what the corresponding C++-Legion construct would do; it changes what the compiler is willing to emit.

## Performance implications
- **The standard way to get GPU performance from Regent.** Without `__demand(__cuda)` you get a CPU variant; with it, GPU code. Mappers (`mapper.md`) decide which to run, given both exist.
- **`__demand(__index_launch)`** is the safety net against accidentally serializing what looks data-parallel — the compile-time check guarantees the launch becomes one operation, not N (see `index-space-launch.md`).
- **`__demand(__leaf)`** unlocks the leaf-context fast path; the runtime knows the task launches nothing further and skips bookkeeping.
- **`__demand(__replicable)`** is how Regent opts a top-level task into control replication for multi-node scaling.

## Debug signals
- **Compiler diagnostics** name the directive that failed and the rule that was violated. The error is the first place to look.
- **No directive → silent fallback** is the *failure mode the directive is designed to prevent*. If you suspect Regent silently emitted CPU code, add `__demand(__cuda)` and recompile.

## Failure modes
- Forgetting `__demand` and getting silent CPU fallback when GPU was expected.
- `__demand(__leaf)` on a task that does launch subtasks → compile error.

## Source pointers
- **Compiler tree**: https://github.com/StanfordLegion/legion/tree/master/language
- **Transcript (retreat 2024)**: `raw/youtube_transcripts/retreat_2024/transcripts/003_..._Regent_and_Pygion_-_Elliott_Slaughter.txt`
- **Transcript (bootcamp 2017 Regent update)**: `raw/youtube_transcripts/bootcamp_2017/transcripts/010_Regent_Update_-_Legion_Bootcamp_2017_10_of_10.txt`

## Related
- `wiki/concepts/regent-language.md` — host concept.
- `wiki/concepts/task.md` — `__demand(__leaf)` / `__demand(__inner)` align with task variants.
- `wiki/concepts/index-space-launch.md` — `__demand(__index_launch)` enforces this transformation.
- `wiki/concepts/control-replication.md` — `__demand(__replicable)` opts in.

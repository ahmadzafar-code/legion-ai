---
title: Debug a Correctness Bug
slug: debug-correctness-bug
summary: A staged recipe for diagnosing wrong-answer / crash / hang bugs in Legion apps; layer build flags, runtime flags, and tools in the order Legion's own debugging guide recommends.
tags: [for-correctness-debug, debugging, tooling]
status: draft
created: 2026-05-15
updated: 2026-05-15
related:
  - wiki/concepts/debug-mode.md
  - wiki/concepts/backtrace-mode.md
  - wiki/concepts/freeze-on-error.md
  - wiki/concepts/privilege-checks.md
  - wiki/concepts/bounds-checks.md
  - wiki/concepts/partition-checks.md
  - wiki/concepts/legion-spy.md
  - wiki/concepts/mapper-logging.md
  - wiki/concepts/in-order-execution.md
  - wiki/concepts/error-message-catalog.md
---

## Inputs

- A Legion or Regent application that crashes, hangs, or produces wrong output.
- A minimal reproducer if possible (smaller is faster to iterate on).

## Steps

The order is the one `raw/website-pages/debugging.md` recommends. Layer the tools — each step rules out a class of bug.

1. **Compile in debug mode**.
   ```bash
   DEBUG=1 make
   ./app
   ```
   This activates the runtime's internal assertions and consistency checks (`debug-mode.md`). Bugs that fire here are usually internal invariant violations the release build silently allows. Many bugs are caught right here.

2. **Add backtrace on error**.
   ```bash
   LEGION_BACKTRACE=1 ./app
   ```
   Turns "Segmentation fault" into a stack at the failure (`backtrace-mode.md`). For MPI, propagate with `-x LEGION_BACKTRACE=1`.

3. **For sporadic / multi-node bugs, add freeze-on-error**.
   ```bash
   LEGION_BACKTRACE=1 LEGION_FREEZE_ON_ERROR=1 ./app
   ```
   The failing process pauses and prints its PID; attach `gdb -p <PID>` (`freeze-on-error.md`). Pair with `-ll:force_kthreads` to make all Realm threads visible.

4. **Enable privilege checks** (if the bug looks data-related):
   ```bash
   CC_FLAGS=-DPRIVILEGE_CHECKS DEBUG=1 make
   ./app
   ```
   Every accessor checks the declared privilege matches actual access (`privilege-checks.md`). Catches the canonical "WRITE_DISCARD but I secretly read" and "READ_ONLY but I wrote" bugs.

5. **Add bounds checks** (if accesses look out-of-bounds):
   ```bash
   CC_FLAGS="-DPRIVILEGE_CHECKS -DBOUNDS_CHECKS" DEBUG=1 make
   ./app
   ```
   Catches indexing outside a granted region (`bounds-checks.md`). Common with partition-mismatch bugs.

6. **Add partition disjointness checks** (if partitioning code changed recently or the bug is non-deterministic):
   ```bash
   ./app -lg:partcheck
   ```
   Verifies declared-disjoint partitions actually are (`partition-checks.md`). Catches the silent data-race form of `non-disjoint-disjoint-partition`.

7. **Force in-order execution** for non-deterministic / timing-dependent bugs:
   ```bash
   ./app -lg:inorder
   ```
   Eliminates parallelism (`in-order-execution.md`). If the bug disappears, you have a race condition or missing dependence — fix privilege/coherence/partition declarations. If the bug persists, it's deterministic and lives in your logic.

8. **Use Legion Spy** for dependence analysis:
   ```bash
   ./app -lg:spy -logfile spy_%.log
   legion/tools/legion_spy.py -dez spy_*.log
   ```
   The dataflow graph (`dataflow-graph.md`) shows the logical operation DAG with privilege-labelled edges. Look for edges that shouldn't exist (false dependences) or missing edges where you expected them. Event graph (`event-graph.md`) shows per-point precision.

9. **For mapper-related bugs, enable mapper logging**:
   ```cpp
   // In your registration code:
   runtime->replace_default_mapper(new LoggingWrapper(new MyMapper(...)), p);
   ```
   ```bash
   ./app -level mapper=2 -logfile mapper_%.log
   ```
   See exactly what decisions the mapper made for each task (`mapper-logging.md` + `logger-categories.md`).

10. **Look up any error messages** in `wiki/concepts/error-message-catalog.md` and `raw/website-pages/error_messages.md` — codes are stable and identify the bug class.

## Outputs

- A specific failure point (line of code, task name, region/field involved).
- A categorical diagnosis (privilege bug / bounds bug / disjointness bug / race / mapper bug / runtime bug).
- A reproducer the fix can be tested against.

## When to use

- The application crashes, hangs, or returns wrong output.
- Symptoms are consistent or sporadic — both are addressable through this recipe.
- Before reporting a suspected runtime bug, run through this checklist; most "Legion bugs" turn out to be application-side privilege or partition misuse.

## Related

- `wiki/concepts/debug-mode.md` — foundational build flag.
- `wiki/concepts/backtrace-mode.md` — stack on crash.
- `wiki/concepts/freeze-on-error.md` — post-mortem attach.
- `wiki/concepts/privilege-checks.md` / `wiki/concepts/bounds-checks.md` / `wiki/concepts/partition-checks.md` — the layered correctness checks.
- `wiki/concepts/legion-spy.md` + `wiki/concepts/dataflow-graph.md` — dependence-level diagnosis.
- `wiki/concepts/mapper-logging.md` — mapper-decision diagnosis.
- `wiki/concepts/in-order-execution.md` — race-vs-logic-bug discriminator.
- `wiki/concepts/error-message-catalog.md` — code lookup.

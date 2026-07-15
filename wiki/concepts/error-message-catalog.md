---
title: Error Message Catalog
slug: error-message-catalog
summary: The Legion runtime's catalog of 632 standard error codes, 13 fatal codes, and 76 warning codes; each names a specific failure with a category (task, region, mapping, partition, etc.) that points at a concept page for the fix.
tags: [errors, debugging, tooling, for-correctness-debug]
subsystem: legion
layer: tooling
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/website-pages/error_messages.md
github:
  - https://github.com/StanfordLegion/legion/tree/master/runtime/legion
related:
  - wiki/concepts/debug-mode.md
  - wiki/concepts/privilege-checks.md
  - wiki/concepts/bounds-checks.md
  - wiki/concepts/partition-checks.md
  - wiki/concepts/freeze-on-error.md
  - wiki/concepts/event-poisoning.md
---

## TL;DR
The Legion runtime emits **632 standard error codes + 13 fatal codes + 76 warnings** (per `raw/website-pages/error_messages.md`), each with a numbered ID and a human-readable category (Task / Region / Mapping / Partition / Index space / Copy & fill / etc.). When the runtime prints an error, the message identifies the category and what invariant was violated — usually with a clear pointer to the responsible task or operation. The confusion: many codes are emitted only when a corresponding check is enabled (e.g., disjointness violations need `-lg:partcheck`; privilege violations need `-DPRIVILEGE_CHECKS`). Without the right flags, the bug fires later as wrong output rather than as a clean error code.

## Mental model
The error catalog is Legion's `errno` table. Each code is a known failure mode with a known cause; the right fix is usually unambiguous once you know which category fired. Reading an error message is like reading a `man errno` page — the code identifies the bug class, not the line where the bug was written.

## Mechanism & API
**Categories** (per `raw/website-pages/error_messages.md`):

| Code range | Category | Typical causes |
|---|---|---|
| 1–100 | Task-related | Invalid task ID, duplicate registration, missing variant, malformed region requirement, parent privilege error |
| 101–200 | Region-related | Invalid region handle, invalid field ID, incompatible region trees, field-space mismatch |
| 201–350 | Mapping-related | Invalid mapper output, no valid instances, incompatible layout, allocation failure, virtual mapping error |
| 351–450 | Partition-related | Disjointness violation, completeness violation, invalid color, cross-product/image/preimage errors |
| 451–500 | Index-space errors | Invalid handle, bounds error, dimension mismatch, transform error |
| 501–550 | Copy & fill | (continued in raw file) |
| 551–632 | Misc + system | Realm-side and inter-node errors |

Format printed at runtime:
```
[error N] (CATEGORY) message text
  in task <task-name> launched from <parent-task>
```

**Triggering classes of error**:
- Always emitted: invariant violations the runtime detects unconditionally (e.g., invalid handles, allocation failure).
- Conditional on build flag: `-DPRIVILEGE_CHECKS` catches privilege violations (200-range mapping errors related to accessor privileges); `-DBOUNDS_CHECKS` catches bounds errors.
- Conditional on runtime flag: `-lg:partcheck` catches disjointness violations in the 351-450 range.

**Where the catalog lives**: the implementation is in `runtime/legion/legion.cc` (and friends). The numbered codes are stable across releases; new codes append.

**Fatal codes** (13 of them) are emitted for unrecoverable conditions where the runtime cannot continue safely (e.g., out-of-memory in a memory the application has no fallback for, internal consistency violations).

**Warning codes** (76 of them) flag non-fatal issues — typically inefficiencies the runtime can route around but the user probably wanted to know about (e.g., a partition declared aliased that happens to be disjoint, suggesting the user might want to switch).

## Invariants
- Codes are **stable** across releases; new codes append rather than renumber.
- Every error message includes enough information to identify the responsible task and operation.
- Fatal codes always terminate; standard codes typically terminate but may be recoverable in some contexts; warnings never terminate.
- Many codes are emitted only with the right build/runtime flag enabled; without the flag, the bug manifests later as wrong output.
- The runtime never emits the same error code from two distinct invariant violations — codes are 1:1 with bugs.

## Performance implications
- Errors have no perf impact on the happy path.
- Warnings have negligible cost; they're useful diagnostic signals.
- Catching errors earlier (via debug flags) is cheaper than chasing wrong output later.

## Debug signals
- **Look up the error code in `raw/website-pages/error_messages.md`** for the canonical description.
- The category guides which concept page is relevant:
  - Task → `privilege.md`, `task.md`, `region-requirement.md`.
  - Mapping → `mapper.md`, `physical-instance.md`, `map-task.md`.
  - Partition → `partition.md`, `partition-checks.md`, `non-interference.md`.
  - Region → `logical-region.md`, `region-tree.md`.
- **Privilege/parent-task errors** (codes near 1–100, "Parent task privilege error") → child requested privileges not held by parent. Trace via `dataflow-graph.md` to find the offending requirement.
- **"Disjointness violation"** → re-run with `-lg:partcheck` if you weren't already.

## Failure modes
- An error fires without an obvious source — usually means the wrong category of check is enabled. Match the error category to the appropriate build/runtime flag.
- A fatal error in production — almost always points at insufficient `-ll:csize` / `-ll:fsize` (memory) or `-ll:rsize` (RDMA).

## Source pointers
- **Reference (full catalog)**: `raw/website-pages/error_messages.md`
- **Runtime (where errors are emitted)**: https://github.com/StanfordLegion/legion/tree/master/runtime/legion

## Related
- `wiki/concepts/debug-mode.md` — assertion coverage.
- `wiki/concepts/privilege-checks.md` — gates the privilege-error codes.
- `wiki/concepts/bounds-checks.md` — gates the bounds-error codes.
- `wiki/concepts/partition-checks.md` — gates disjointness/completeness codes.
- `wiki/concepts/freeze-on-error.md` — pause-on-error workflow.

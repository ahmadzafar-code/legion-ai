---
title: Pygion
slug: pygion
summary: Stanford's Python bindings for the Legion programming model; expose tasks, regions, partitions, privileges as Python objects, with a typical decorator-based task syntax.
tags: [execution, data-model, configuration, for-program-reasoning]
subsystem: pygion
layer: programming-model
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/publications/publications.md
  - raw/youtube_transcripts/retreat_2024/transcripts/003_Legion_Retreat_2024_-_Regent_and_Pygion_-_Elliott_Slaughter.txt
github:
  - https://github.com/StanfordLegion/legion/tree/master/bindings/python
related:
  - wiki/concepts/task.md
  - wiki/concepts/regent-language.md
  - wiki/concepts/control-replication.md
  - wiki/concepts/logical-region.md
---

## TL;DR
Pygion is the Python binding for Legion: you write tasks as decorated Python functions, declare regions/partitions/privileges via Python objects, and Pygion does the work of registering them with the Legion runtime. Unlike `regent-language.md`, Pygion has **no static checking** — Python's runtime type system limits what the bindings can prove — but it gives you the same Legion semantics with Python's ecosystem (NumPy, CuPy, Numba, third-party CUDA libraries). The confusion: Pygion is **not** "Legate" or "cuNumeric"; those are domain-specific libraries built on top. Pygion is the *direct* interface to Legion in Python.

## Mental model
Pygion is to Legion-Python what PyTorch is to Caffe-Python: a Pythonic wrapper around an underlying C++ runtime, where the wrapper exposes the runtime's semantics directly rather than imposing a higher-level abstraction. You're still programming Legion — tasks, regions, partitions — just with Python syntax. Use Pygion when you want Legion semantics in a Python codebase and need NumPy/CuPy interop; use Legate/cuNumeric when you want NumPy-shaped APIs that *happen* to scale through Legion.

## Mechanism & API
**Stack**: a Python interpreter task (`PY_PROC`) running CPython; Pygion is the Python module that registers tasks and forwards launchers to the underlying Legion runtime.

**Task pattern** (illustrative, per the Pygion paper `pygion2019.pdf`):
```python
import pygion
from pygion import task, R, RW, Region, Ispace, Fspace

@task(privileges=[RW])
def init(r):
    # r is a Pygion region-handle; access via numpy-style indexing
    ...

@task
def top_level():
    ispace = Ispace([1024])
    fspace = Fspace({'x': pygion.float64})
    r = Region(ispace, fspace)
    init(r)
```

Per the retreat 2024 transcript:
- Pygion is **stable** and used in production (single-particle imaging on Frontier, scaled to 4,000 GPUs).
- The 2024 update is mainly **improved Regent interop** — Pygion tasks can call Regent tasks and vice versa.
- The Python ecosystem (**NumPy / CuPy / Numba** + third-party CUDA libraries) is directly usable from Pygion tasks. The single-particle-imaging code referenced in the talk uses NumPy + CuPy + Numba + custom CUDA kernels with Pygion as the **task orchestration layer**.

**Comparison with Regent**:
- Regent has static checking; Pygion does not (Python doesn't).
- Regent has automatic GPU code generation via Terra/LLVM; Pygion expects you to bring your own GPU kernels (CuPy/Numba/CUDA libraries).
- Both are "direct" interfaces — same Legion semantics, just different syntax.

**Comparison with the C++ API**:
- Pygion handles boilerplate (task registration, launcher construction, future unwrap).
- Lag time for new Legion features: bleeding-edge runtime features land in C++ first.

## Invariants
- Pygion tasks are **Python functions** executed by Realm's `PY_PROC` processor kind under the Legion runtime — they participate in dependence analysis exactly like C++ tasks.
- Privileges are declared via the `@task(privileges=[...])` decorator (or equivalent); the runtime enforces them dynamically. There is **no compile-time check** (Python).
- Regions, index spaces, and field spaces are Pygion objects that wrap Legion handles; their lifetimes follow Python ref-counting, with destruction deferred by Legion's `destroy_*` semantics.
- Pygion tasks **interop with Regent** — tasks defined in either can call into the other.
- The same Legion + Realm runtime executes both Pygion and C++ programs; same debugging tools apply.

## Performance implications
- Per-task **Python call overhead** is real; coarsen tasks rather than launching millions of trivial ones.
- For numerical hot paths, delegate to **NumPy/CuPy/Numba** or a CUDA library inside the task. The orchestration is Python; the kernels are not.
- **Control replication** works for Pygion top-level tasks the same as for C++ — required for multi-node scaling.
- Tracing (`tracing.md`) and automatic tracing (`automatic-tracing.md`) apply to Pygion task streams.
- Mapper, profiling (`legion-prof.md`), and debugging (`legion-spy.md`) tooling all work unchanged.

## Debug signals
- **Standard Legion Prof / Spy tools** work for Pygion runs — the runtime layer is the same.
- Python-side errors surface in the usual Python traceback; Legion runtime errors via `LEGION_BACKTRACE=1` continue to work.
- **`PY_PROC` rows in Legion Prof** show Pygion task execution.

## Failure modes
- Python interpreter overhead dominating execution time → coarsen tasks, delegate to NumPy/CuPy.
- Forgetting privilege declarations → runtime error inside the binding layer.

## Source pointers
- **Python bindings**: https://github.com/StanfordLegion/legion/tree/master/bindings/python
- **Paper**: `raw/publications/pdfs/pygion2019.pdf` (PAW-ATM 2019)
- **Paper (SPI code)**: `raw/publications/pdfs/wamta2024.pdf` (WAMTA 2024)
- **Talk**: `raw/youtube_transcripts/retreat_2024/transcripts/003_..._Regent_and_Pygion.txt`

## Related
- `wiki/concepts/task.md` — what Pygion tasks compile to under the bindings.
- `wiki/concepts/regent-language.md` — sibling direct interface; interop is supported.
- `wiki/concepts/control-replication.md` — applies to Pygion top-level tasks.
- `wiki/concepts/logical-region.md` — what Pygion `Region` objects wrap.

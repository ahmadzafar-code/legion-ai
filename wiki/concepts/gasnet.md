---
title: GASNet
slug: gasnet
summary: The networking layer beneath Realm for inter-node communication; provides put/get RDMA and active messages; built per-network via "conduits" (`ibv`, `ucx`, `ofi`, `mpi`, ...). Required for multi-node Legion runs.
tags: [distributed, configuration, for-perf-debug, for-correctness-debug]
subsystem: realm
layer: system
status: draft
created: 2026-05-15
updated: 2026-05-15
sources:
  - raw/website-pages/gasnet.md
  - raw/website-pages/getting_started.md
github:
  - https://github.com/StanfordLegion/legion/tree/master/runtime/realm/gasnetex
related:
  - wiki/concepts/realm-machine-model.md
  - wiki/concepts/dma-system.md
  - wiki/concepts/memory-kinds.md
  - wiki/concepts/runtime-flags-reference.md
  - wiki/workflows/move-from-single-node-to-distributed.md
---

## TL;DR
GASNet (Global-Address Space Networking) is the substrate Realm uses for inter-node communication. It provides one-sided RDMA (put/get) and active messages — the primitives Realm uses to implement distributed reference counting, event triggering, and DMA across nodes. Single-node runs don't need it; **multi-node runs require it**. GASNet is configured per-network via "conduits": `ibv` for InfiniBand, `ucx` / `ofi` for Slingshot and modern fabrics, `aries`/`gemini` for legacy Cray, `mpi` as portable fallback. The confusion: GASNet is below Legion's debug toolchain. Most Legion users don't read its source — they install it, set `CONDUIT=ibv`, and forget. But when multi-node runs hang at startup or scale poorly, GASNet config is the first place to look.

## Mental model
GASNet is to Realm what MPI is to a typical HPC program: the inter-node messaging layer. Where MPI gives you `MPI_Send`/`MPI_Recv` + collectives, GASNet gives Realm a lower-level "remote memory access" primitive + active-message dispatch. Legion never calls GASNet directly — Realm does.

## Mechanism & API

**Conduit selection** (per `raw/website-pages/gasnet.md`):

| Conduit | Hardware | When to use |
|---|---|---|
| `ibv` | InfiniBand (ibverbs direct) | InfiniBand clusters |
| `ucx` | UCX-supported (IB, RoCE, ...) | Modern fabrics with UCX 1.9+ |
| `ofi` | libfabric (Slingshot, ...) | HPE Slingshot, modern fabrics |
| `aries` | Cray Aries | Cray XC systems |
| `gemini` | Cray Gemini | Legacy Cray XE/XK |
| `mpi` | Any MPI-2+ | Portable fallback, lower perf |
| `udp` | TCP / Ethernet | Development and testing only |
| `smp` | Shared memory | Single-node only |

**Build paths**:

1. **Auto-build** (recommended for most): the Legion build system downloads + builds GASNet:
   ```bash
   export CONDUIT=ibv
   USE_GASNET=1 make -j$(nproc)
   ```

2. **External GASNet**:
   ```bash
   export GASNET_ROOT=/path/to/gasnet
   export CONDUIT=ibv
   USE_GASNET=1 GASNET=$GASNET_ROOT make
   ```

3. **CMake**:
   ```cmake
   cmake -DLegion_USE_GASNet=ON -DGASNet_CONDUIT=ibv -DGASNet_ROOT=/path/to/gasnet
   ```

**Launching**:
- MPI conduit: `mpirun -np <N> ./app`
- IBV conduit: `gasnetrun_ibv -n <N> ./app` or `mpirun` if MPI bootstrap is configured
- UDP conduit (dev only): `GASNET_SPAWNFN=L gasnetrun_udp -n <N> ./app`

**Sizing knobs** (Legion runtime flags + GASNet env vars):
- `-ll:rsize N` — Realm-registered DMA memory in MB. **Required for high-BW inter-node transfers; default is 0**. Set to 1024+ for production multi-node.
- `-ll:gsize N` — GASNet global memory segment in MB.
- `GASNET_MAX_SEGSIZE` — max shared segment size (e.g., `4GB`).
- `GASNET_PHYSMEM_MAX` — max physical memory GASNet may use.

## Invariants
- **Single-node runs do not need GASNet.** Build with `USE_GASNET=0` (default) if you only run single-node.
- The conduit is **selected at GASNet build time** and **must match the network hardware**. Wrong conduit = no connectivity.
- GASNet must be built in **PAR mode** (`--enable-par`) — fully thread-safe. Required for Legion.
- The conduit's home node and the application's home node coordinate at startup; mismatched configuration causes silent hangs.
- `-ll:rsize` defaults to 0, which means **no RDMA**; inter-node copies fall back to slower paths. Always set explicitly for multi-node.

## Performance implications
- **The network bandwidth ceiling for any Legion application** is set by GASNet + hardware. Pick the right conduit.
- **InfiniBand**: use `ibv` (not `mpi`) for direct verbs access. The `mpi` conduit is portable but loses ~30-50% bandwidth.
- **Slingshot (Frontier, etc.)**: use `ofi` with libfabric 1.5+.
- `-ll:rsize` size limits the working set of in-flight cross-node copies. Too small → throttling.
- `-ll:amsg N` — number of active-message handler threads. Bump for heavy fan-out / fan-in patterns.

## Debug signals
- **Multi-node hang at startup** → conduit mismatch, missing MPI bootstrap, or `ulimit -l` too low for pinned memory. Check `ulimit -l`, conduit selection, and `gasnetrun_*` output.
- **Multi-node `mpirun` hangs after `Initialized`** → MPI bootstrap issue. Try `export GASNET_BARRIER=DISSEM`.
- **"Failed to pin memory"** → `ulimit -l unlimited` or set `/etc/security/limits.conf` accordingly.
- **Slow inter-node copies** → `GASNET_VERBOSEENV=1 ./app` shows the actual GASNet configuration. Verify the conduit is what you expect.
- **`GASNET_BACKTRACE=1`** prints a stack on GASNet errors.

## Failure modes
- Wrong conduit → no connectivity; hangs at startup.
- `-ll:rsize 0` (default) on multi-node → slow inter-node copies via fallback paths.
- `ulimit -l` too low → pinned-memory allocation fails; choose between "use much less memory" and "fix the limit".
- Mixing different GASNet versions across nodes → undefined behavior; ensure same install everywhere.

## Source pointers
- **Reference**: `raw/website-pages/gasnet.md`
- **GASNet upstream**: https://gasnet.lbl.gov/ (GASNet-EX is the current generation)
- **Legion getting-started**: `raw/website-pages/getting_started.md`

## Related
- `wiki/concepts/realm-machine-model.md` — what GASNet enables for distributed memories.
- `wiki/concepts/dma-system.md` — what runs on GASNet for inter-node copies.
- `wiki/concepts/memory-kinds.md` — `REGDMA_MEM` is GASNet-pinned memory.
- `wiki/concepts/runtime-flags-reference.md` — `-ll:rsize`, `-ll:gsize`, `-ll:amsg`, etc.
- `wiki/workflows/move-from-single-node-to-distributed.md` — the workflow that uses GASNet.

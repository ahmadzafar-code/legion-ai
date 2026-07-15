# Reviewing this fork against upstream

This document orients a reviewer of **legion-ai** (the Legion AI fork of
[StanfordLegion/prof-viewer](https://github.com/StanfordLegion/prof-viewer))
to exactly how the fork differs from upstream `master`, and how to convince
yourself the differences are contained.

## What the fork adds, in three sentences

Legion AI is an AI diagnostic co-pilot inside the Legion Prof viewer: the
user's own **Claude Code** is spawned headless against an **in-viewer,
loopback-only HTTP MCP server** that exposes read-only SQL over a DuckDB
export of the profile, sandboxed source/wiki readers, and visual timeline
tools (screenshot / zoom / highlight). Answers arrive in a chat panel with
every tool call auditable, and diagnoses land as clickable highlights on the
timeline. Machine-touching tool calls (shell, file edits, web) block on a
Deny / Allow / Always-allow dialog rendered by the viewer.

## Ground rules the diff obeys

1. **Everything is feature-gated, all off by default.** Features: `ai`
   (panel + agent layer), `duckdb` (upstream's existing feature; the data
   tools require it), `viewer-mcp` (= `ai` + `duckdb` + `httparse`; the MCP
   server + Claude Code engine), `eval` (oracle-graded eval harness).
2. **A no-features build is the upstream viewer.** `cargo build` compiles
   none of the AI layer (module declarations themselves are cfg-gated in
   `src/lib.rs` / `src/app/mod.rs`).
3. **Upstream-owned files carry only thin seams.** The pattern throughout:
   a one-to-four-line `#[cfg(feature = "ai")]`-gated call at the integration
   point, with the body in a fork-owned file.

## Where the code lives

### Additive files (new; review as ordinary new code)

| Path | What it is |
|---|---|
| `src/ai/mod.rs` | module wiring, shared event/type vocabulary, feature gates |
| `src/ai/chat_panel.rs` | the chat panel UI: composer, ＋ context menu, model/effort picker, transcript, approval dialog |
| `src/ai/claude_code.rs` | the engine: persistent headless `claude` child (stream-json), settings isolation, PreToolUse hook plumbing |
| `src/ai/viewer_mcp.rs` | in-viewer HTTP MCP transport: loopback bind, per-session bearer token, `/approve` bridge |
| `src/ai/mcp_core.rs` | transport-agnostic MCP dispatch (shared by the HTTP server and the stdio `mcp` bin) |
| `src/ai/tools/{mod,defs,query,source,wiki,overview}.rs` | tool implementations: schemas, hardened read-only DuckDB executor, sandboxed source reader, wiki retrieval, pre-computed diagnostic overview |
| `src/ai/bridge.rs` | agent/MCP → live-timeline request bridge (viewport token, screenshot round-trip) |
| `src/ai/agent.rs` | the built-in direct-API engine — currently **disabled** (`chat_panel::NATIVE_ENGINE_ENABLED = false`); retained for the eval harness |
| `src/ai/trace.rs` | default-on local session transcripts (JSONL, 0600/0700) |
| `src/app/core/legion_ai.rs` | **every AI addition to the viewer core** — see below |
| `src/bin/{mcp,eval,embedded_runner}.rs` | stdio MCP server; eval harness; eval gradee |
| `build.rs`, `check.sh`, `deny.toml`, `SECURITY.md`, `docs/` | build identity, verify script, supply-chain policy, threat model, docs |

### Modified upstream files (the actual review surface)

| Path | Added lines | Nature of the changes |
|---|---|---|
| `src/app/core.rs` | ~370 (~320 with `-w`) | cfg-gated struct fields + one-line seams calling into `core/legion_ai.rs` |
| `src/main.rs` | ~165 | an `ai`-gated `main` with `--duckdb/--code/--wiki/--help` + sibling-DB auto-detect; the non-`ai` `main` is upstream's, byte-for-byte behavior |
| `src/lib.rs`, `src/app/mod.rs` | ~6 | cfg-gated module declarations + re-exports |
| `Cargo.toml` | ~85 | fork metadata, optional AI dependencies, the three feature definitions |
| `.github/workflows/*` | ~100 | feature-matrix CI, release binaries, cargo-audit/deny jobs |

## The `core.rs` arrangement (read this before diffing it)

All AI code that must live inside the viewer core is in
**`src/app/core/legion_ai.rs`** — a **child module** of `app::core`. A child
module can access the parent's private types and fields (`Context`, `Config`,
`Window`, `Slot`, `ProfApp`), which is what lets the fork extend the core
**without widening any visibility** and without scattering code through the
upstream file. `core.rs` itself keeps only:

- cfg-gated **struct fields** (panel handle, screenshot slots, highlight maps),
- cfg-gated **one-line calls** into `legion_ai::*` at the integration points
  (frame services, screenshot pipeline, selection sync, UI seams),
- two irreducible cfg **expression pairs** in the drag handler, and
- the gated `mod legion_ai;` declaration at the bottom.

The one seam that re-indents upstream code (the sidebar-toggle wrap around the
left panel) is annotated in-source; review that hunk with `git diff -w`.

## How to review

```sh
git remote add upstream https://github.com/StanfordLegion/prof-viewer.git
git fetch upstream master

git diff --stat upstream/master           # the map above
git diff upstream/master -- src/app/core.rs   # the seams (use -w for the sidebar hunk)
```

Suggested order:

1. `Cargo.toml` — features and optional deps (all AI deps are `optional = true`).
2. `src/lib.rs`, `src/app/mod.rs` — the cfg-gated module declarations (this is
   the "no features ⇒ no AI code" guarantee).
3. `src/app/core.rs` — the seams; then `src/app/core/legion_ai.rs` as a new file.
4. `src/ai/` bottom-up: `tools/` (pure functions) → `mcp_core.rs` (dispatch) →
   `viewer_mcp.rs` (transport + auth) → `claude_code.rs` (engine) →
   `chat_panel.rs` (UI).
5. `SECURITY.md` for the threat model; `docs/architecture.md` for the system map.

## Upstream-equivalence check

```sh
cargo check --no-default-features --lib   # upstream-equivalent build
cargo check --all-features --all-targets
# every feature combo CI enforces:
for f in ai duckdb ai,duckdb viewer-mcp eval; do cargo check --features $f --all-targets; done
cargo test --all-features --all-targets
```

CI (`.github/workflows/rust.yml`) runs the full matrix on every push, plus
clippy `-D warnings`, rustfmt, cargo-audit (informational), and cargo-deny
(bans + sources).

## Merge posture

The fork tracks upstream `master` (currently merged through 0.8.1 / egui 0.30)
and versions itself to match the merged upstream version. Because the AI layer
is additive-plus-seams, an upstream merge typically conflicts only if upstream
edits the exact lines adjacent to a seam — resolution is re-placing a one-line
gated call.

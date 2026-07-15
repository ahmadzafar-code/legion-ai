# AI Co-Pilot architecture

One sentence: **two agents drive one shared tool layer over a live egui
viewer**, and everything else is plumbing to make that safe and honest.

```
                                  ┌──────────────────────────────┐
   ┌────────────────┐   HTTPS     │  src/ai/agent.rs             │
   │ Anthropic API  │◄───────────►│  built-in API loop           │
   └────────────────┘             │  (AgentSession, ureq)        │
                                  └────────┬─────────────────────┘
                                           │ direct calls
                                  ┌────────▼─────────────────────┐
                                  │  src/ai/tools/               │
                                  │  ONE tool layer: run_query,  │
                                  │  overview, read_code,        │
                                  │  wiki_*, visual tools        │
                                  └────────▲─────────────────────┘
                                           │ mcp_core.rs dispatch
   ┌────────────────┐  stream-json ┌───────┴──────────────────────┐
   │ your `claude`  │◄────────────►│  src/ai/claude_code.rs       │
   │ (Claude Code)  │  stdin/out   │  ClaudeCodeAgent (subprocess)│
   └───────┬────────┘              └───────▲──────────────────────┘
           │ HTTP POST /mcp + /approve     │ spawn / events
   ┌───────▼───────────────────────────────┴──────┐
   │  src/ai/viewer_mcp.rs — loopback HTTP server │
   │  (bearer token, Origin check, approval route)│
   └───────▲──────────────────────────────────────┘
           │ UiBridge (channels + viewport token)
   ┌───────▼──────────────────────────────────────┐
   │  egui viewer (src/app/core.rs + chat_panel)  │
   └──────────────────────────────────────────────┘
```

(The Anthropic-API driver is eval-only today: the chat panel ships with the
built-in engine disabled, so Claude Code is the only interactive agent.)

## The pieces

- **`tools/`** — ONE tool layer, split by concern: `query.rs` (read-only
  DuckDB executor: `AccessMode::ReadOnly` + `enable_external_access(false)`,
  50-row cap with LIMIT stripping), `source.rs` (sandboxed source reader),
  `overview.rs` (pre-computed diagnostic signals), `wiki.rs` (knowledge
  retrieval), `defs.rs` (tool schemas). Both agents call exactly these; there
  is no second implementation of anything.
- **`agent.rs`** — the built-in engine: a hand-rolled agentic loop over the
  Anthropic API (prompt caching, exponential backoff, extended thinking on
  Opus). Currently DISABLED in the chat panel (`NATIVE_ENGINE_ENABLED = false`
  in `chat_panel.rs` — Claude Code is the only interactive engine); kept alive
  as the eval harness's out-of-process gradee via `bin/embedded_runner`.
- **`claude_code.rs`** — the Claude Code engine: spawns the user's own
  `claude` headless (`--input-format stream-json`), persistent across turns on
  one stdin. Contains the invocation constants (tool availability/allow
  lists, settings isolation), the stream-json → `AgentEvent` parser, and the
  **approval broker** behind the `/approve` route (per-call Deny/Allow/Always
  dialogs for action tools, delivered via a `PreToolUse` hook that shells out
  to `curl`).
- **`mcp_core.rs`** — transport-agnostic MCP dispatch (tools/list, tools/call)
  reusing `tools.rs`. Wrapped by two transports: the stdio `mcp` bin and —
- **`viewer_mcp.rs`** — the in-viewer loopback HTTP server (bearer token,
  Origin check, `/approve`). This is also the BYOA surface: any external MCP
  client can register against it.
- **`bridge.rs`** — how a *second* consumer drives the live window:
  `UiBridge` (blocking request/reply over channels) plus the **viewport
  token**, a structural single-driver lock so the embedded agent and an MCP
  client can never interleave screenshots/navigation. Guards release on every
  exit path via `Drop`.
- **`trace.rs`** — default-on session traces: JSONL transcripts of every turn
  (untruncated tool inputs/outputs, images redacted) under
  `~/.legion_prof_viewer/traces/` (dir 0700, files 0600);
  `LEGION_PROF_AI_TRACE=off` disables, `LEGION_PROF_AI_TRACE_DIR` redirects.
  See [SECURITY.md](../SECURITY.md).
- **`chat_panel.rs`** — the egui chat UI: ＋ menu (connect DB / repo, attach
  files), context chips, markdown transcript, the approval dialog, engine
  auto-detection, the model + effort picker (drives the child's
  `--model`/`--effort`), and Stop (one interrupt per turn).
- **`app/core/legion_ai.rs`** — the fork's additions to the viewer core, as a
  *child module* of `core`: the one Rust arrangement that lets this code use
  `core`'s private types and fields (`ProfApp`, `Window`, `Slot`, `Context`)
  without widening their visibility. Upstream `core.rs` keeps only thin
  `#[cfg(feature = "ai")]`-gated one-line seams, so merges from
  StanfordLegion/prof-viewer barely touch fork code.

## Load-bearing invariants (each documented at its definition)

1. **Channel lifetime** (`chat_panel.rs`, `ChatBackendKind`): the built-in
   engine uses per-turn channels; the Claude Code engine's channels are created
   once at spawn and must outlive turns — dropping its receiver after a turn
   orphans the persistent child's event stream.
2. **Single driver** (`bridge.rs`): at most one consumer owns the viewport at
   a time; the in-viewer MCP server additionally serializes `POST /mcp`
   handling (the `/approve` route runs on detached threads precisely because a
   human decision blocks for minutes).
3. **Live project root** (`mcp_core.rs`, `SharedCodeRoot`): the code root is
   read per request through a shared handle — caching it across requests
   reintroduces the stale-root bug where a folder connected after startup
   never reaches the server.
4. **One tool layer**: agents differ in *transport*, never in *behavior*. A
   fix to a tool fixes it for the chat panel, external MCP clients, and the
   eval harness simultaneously.
5. **Oracle independence** (`bin/eval.rs`): the eval never imports this crate;
   the gradee runs out-of-process (`embedded_runner` / `claude` + the `mcp`
   bin) so the grader cannot share bugs with the graded.

## Security

See [SECURITY.md](../SECURITY.md). The short version: loopback + bearer token
+ Origin check on the server; read-only anti-exfil SQL; sandboxed source
reads; availability-filtered child tools with per-call approval, settings
isolation, and process-group kill.

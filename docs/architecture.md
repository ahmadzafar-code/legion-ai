# Legion AI architecture

Legion AI adds agent-driven performance diagnosis to the Legion Prof viewer:
two agent engines drive one shared tool layer over a live egui timeline.
Engines differ only in transport, never in behavior, so a fix to any tool
applies simultaneously to the chat panel, external MCP clients, and the eval
harness.

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

The Anthropic-API driver is eval-only: the chat panel ships with the built-in
engine disabled (`NATIVE_ENGINE_ENABLED = false` in `chat_panel.rs`), so
Claude Code is the only interactive agent.

## Components

- `src/ai/tools/` — the single tool layer both engines call; see
  [Tool layer](#tool-layer).
- `src/ai/agent.rs` — built-in agentic loop over the Anthropic API; eval-only.
- `src/ai/claude_code.rs` — Claude Code engine: persistent headless `claude`
  subprocess plus the approval broker.
- `src/ai/mcp_core.rs` — transport-agnostic MCP dispatch reusing `tools.rs`.
- `src/ai/viewer_mcp.rs` — in-viewer loopback HTTP server; the BYOA surface.
- `src/ai/bridge.rs` — `UiBridge`: single-driver access to the live viewport.
- `src/ai/trace.rs` — default-on JSONL session traces.
- `src/ai/chat_panel.rs` — egui chat UI, engine auto-detection, approval
  dialog.
- `src/app/core/legion_ai.rs` — the fork's viewer-core additions, as a child
  module of `core`.

## Tool layer

`src/ai/tools/` is split by concern: `query.rs` executes DuckDB SQL read-only
(`AccessMode::ReadOnly` plus `enable_external_access(false)`) with a 50-row
cap and `LIMIT` stripping; `source.rs` is a sandboxed source reader;
`overview.rs` produces roughly 25 pre-computed diagnostic signal sections;
`wiki.rs` retrieves Legion knowledge; `defs.rs` holds the tool schemas. Both
engines call exactly these implementations — no tool has a second
implementation anywhere in the crate.

## Engines

`agent.rs` is a hand-rolled agentic loop over the Anthropic API with prompt
caching, exponential backoff, and extended thinking on Opus. It is disabled
in the chat panel and kept alive as the eval harness's out-of-process gradee
via `bin/embedded_runner`.

`claude_code.rs` spawns the user's own `claude` headless
(`--input-format stream-json`), persistent across turns on one stdin. It
contains the invocation constants (tool availability and allow lists,
settings isolation), the stream-json to `AgentEvent` parser, and the approval
broker behind the `/approve` route: per-call Deny/Allow/Always dialogs for
action tools, delivered via a `PreToolUse` hook that shells out to `curl`.

## Transports

`mcp_core.rs` provides transport-agnostic MCP dispatch (`tools/list`,
`tools/call`) reusing `tools.rs`. Two transports wrap it: the stdio `mcp` bin
and `viewer_mcp.rs`, the in-viewer loopback HTTP server (bearer token, Origin
check, `/approve`). The HTTP server is also the bring-your-own-agent surface:
any external MCP client can register against it.

## Viewer integration

`bridge.rs` lets a second consumer drive the live window: `UiBridge` performs
blocking request/reply over channels, and the viewport token acts as a
structural single-driver lock so the embedded agent and an MCP client can
never interleave screenshots or navigation. Guards release on every exit path
via `Drop`.

`chat_panel.rs` is the egui chat UI: the ＋ menu (connect DB or repo, attach
files), context chips, markdown transcript, the approval dialog, engine
auto-detection, the model and effort picker (drives the child's
`--model`/`--effort`), and Stop (one interrupt per turn).

`app/core/legion_ai.rs` holds the fork's additions to the viewer core as a
child module of `core` — the one Rust arrangement that lets this code use
`core`'s private types and fields (`ProfApp`, `Window`, `Slot`, `Context`)
without widening their visibility. Upstream `core.rs` keeps only thin
`#[cfg(feature = "ai")]`-gated one-line seams, so merges from
StanfordLegion/prof-viewer barely touch fork code.

## Session traces

`trace.rs` writes JSONL transcripts of every turn under
`~/.legion_prof_viewer/traces/` (directory mode 0700, files 0600), on by
default. `LEGION_PROF_AI_TRACE=off` disables tracing; `LEGION_PROF_AI_TRACE_DIR`
redirects it. See [SECURITY.md](../SECURITY.md).

> **Important:** Traces record untruncated tool inputs and outputs (images
> redacted), including any source snippets the agent read.

## Load-bearing invariants

Each invariant is documented in code at its definition.

- Channel lifetime (`chat_panel.rs`, `ChatBackendKind`): the built-in engine
  uses per-turn channels; the Claude Code engine's channels are created once
  at spawn and must outlive turns — dropping its receiver after a turn
  orphans the persistent child's event stream.
- Single driver (`bridge.rs`): at most one consumer owns the viewport at a
  time. The in-viewer MCP server additionally serializes `POST /mcp`
  handling; the `/approve` route runs on detached threads precisely because a
  human decision blocks for minutes.
- Live project root (`mcp_core.rs`, `SharedCodeRoot`): the code root is read
  per request through a shared handle — caching it across requests
  reintroduces the stale-root bug where a folder connected after startup
  never reaches the server.
- One tool layer: agents differ in transport, never in behavior. A fix to a
  tool fixes it for the chat panel, external MCP clients, and the eval
  harness simultaneously.
- Oracle independence (`bin/eval.rs`): the eval never imports this crate; the
  gradee runs out-of-process (`embedded_runner`, or `claude` plus the `mcp`
  bin) so the grader cannot share bugs with the graded.

## Security

See [SECURITY.md](../SECURITY.md). In brief: the server combines a loopback
bind, a per-session bearer token, and an Origin check; SQL execution is
read-only and anti-exfiltration; source reads are sandboxed; the Claude Code
child runs with availability-filtered tools, per-call approval, settings
isolation, and process-group kill.

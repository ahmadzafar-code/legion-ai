# Security model — AI Co-Pilot layer

This document covers the fork's AI additions. It exists because the co-pilot
handles two kinds of untrusted input by design: **profile databases** (task
titles and other strings inside a `.duckdb` you may have received from someone
else) and **the profiled application's source tree** (which the agent is
encouraged to read). Both flow through a language model, so both are treated as
potential prompt-injection vectors, and every mitigation below assumes they are
attacker-influenceable.

## Network exposure

- The in-viewer MCP server binds **127.0.0.1 only**, never `0.0.0.0`.
- Requests with a non-loopback `Origin` header are rejected (DNS-rebinding /
  CSRF defense — a browser can be induced to POST to localhost; it cannot fake
  a loopback Origin).
- Every `POST /mcp` and `POST /approve` requires `Authorization: Bearer
  <token>`. Without this, *any local process* could drive the tools (the Origin
  check passes when no Origin is present). The token is random per session,
  compared in constant time, printed once at startup, and overridable via
  `LEGION_VIEWER_MCP_TOKEN` for a stable external registration.
- Request bodies are capped at 1 MiB.

Threat-model non-goal: same-user local processes that can read this process's
memory or environment are out of scope (they already win by other means).

## Database access

All SQL — the agent's and the overview's — routes through one hardened
executor: the DuckDB connection is opened read-only with
`enable_external_access(false)`, so `read_text`/`read_csv`-style exfiltration
of local files through SQL is rejected. Query results are row-capped in Rust
(trailing `LIMIT`s the model writes are stripped, so the cap cannot be talked
around).

## Source access

`read_code`/`list_files` are sandboxed to the connected project root: relative
paths only, `..` and absolute prefixes rejected, and the effective root is
canonicalized so symlinks inside an untrusted tree cannot escape it.

## The Claude Code engine

When the panel spawns your `claude` as a subprocess, the child runs headless
with a deliberately shaped surface:

- **Availability filter** (`--tools`): sub-agent and command-expansion tools
  (`Task`, `Skill`, `SlashCommand`, `KillShell`) are not advertised at all —
  they could route around per-call gating.
- **Read tier** (`Read`/`Glob`/`Grep` + the viewer's MCP tools) runs without
  prompts. Reading is ingestion; harm requires an action, and every action is
  gated:
- **Action tier** (`Bash`, `Edit`, `Write`, `NotebookEdit`, `WebFetch`,
  `WebSearch`) triggers a **Deny / Allow / Always allow** dialog in the viewer
  for every call, showing the full untruncated command / path / URL with a
  severity badge. The bridge is a `PreToolUse` hook that POSTs to the viewer's
  `/approve` route and blocks on your verdict; it **fails closed** (no answer =
  deny) and a denial is fed back to the model as feedback, not a crash.
- **"Always allow"** rules are session-scoped and in-memory only — they die
  with ↺ New session or app exit, and Bash rules match a command *prefix* only
  when the command contains no shell metacharacters (`;`, `$(`, backticks,
  pipes…), so `cargo build; curl evil` can never ride an approved `cargo …`.
- **Settings isolation**: the child runs with `--setting-sources ""` plus a
  viewer-owned settings file, and its working directory is a viewer-owned
  scratch dir — a malicious project tree cannot inject `.claude/settings.json`
  allow-rules or hooks (Claude Code auto-discovers project settings from its
  cwd; the connected repo is therefore granted via `--add-dir`, never cwd).
- **Process-group kill**: the child is spawned as a process-group leader and
  the whole group is killed on stop, so a mid-run Bash grandchild (or an
  injected `nohup … &`) cannot outlive the viewer.
- The temp files carrying the MCP config and hook settings (which embed the
  session token) are created `0600` and deleted on shutdown. After a hard
  crash (SIGKILL), stale `legion_cc_*` files may remain in the temp dir until
  the next session; the token inside is already invalid by then (new session,
  new token) unless you pinned `LEGION_VIEWER_MCP_TOKEN`.

## Prompt injection, honestly

The approval dialog makes dangerous calls **accountable, not impossible** — a
user who reflexively clicks Allow defeats it. The structural mitigations
(loopback+token server, read-only DB, sandboxed source access, availability
filtering, settings isolation, group kill) carry the real weight, and the
system prompt additionally instructs the model to treat all tool-returned
strings as data. If you review one thing before trusting a shared profile with
action tools enabled, review your own click habits on that dialog.

## Reporting

Please open a GitHub issue on this fork (or a private security advisory if the
issue is sensitive).

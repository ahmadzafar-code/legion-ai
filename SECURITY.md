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
  prompts. Two important caveats on reading:
  - The viewer's MCP `read_code`/`list_files` are root-confined (see *Source
    access* above), but the harness's own `Read`/`Glob`/`Grep` are **not** — they
    can read any file your user account can read, not only the `--add-dir` root.
  - Reading is an **egress channel**, not just ingestion: whatever the agent
    reads becomes conversation context sent to the model provider (Anthropic) and
    is also written to the local session trace. A prompt-injection in profile
    data or source could therefore make the agent read an unrelated host file and
    exfiltrate it via the provider/trace, *without* tripping the action gate.
  - Current posture: reads are **ungated by design** (an approval prompt on every
    file read would train users to click *Always allow* — arguably worse). The
    mitigations are to point the co-pilot only at profiles and code you trust,
    and that every *action* (below) is gated. Path-scoped gating of out-of-root
    reads is a considered future hardening (tracked in TODO).
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
- **The session token never appears in a process's arguments.** It lives only
  in two viewer-owned temp files — the `--mcp-config` (the child's
  `Authorization` header) and a `curl --config` file the approval hook reads —
  both created *atomically* at `0600` (no world-readable window) and deleted on
  shutdown. The token is deliberately kept out of the hook's `curl` argv because
  a process's command line is readable by other local users
  (`/proc/<pid>/cmdline`, `ps`) — on a shared login node that would hand a
  co-tenant the key to the loopback server. After a hard crash (SIGKILL), stale
  `legion_cc_*` files may remain until the next session; the token inside is
  already invalid by then (new session, new token) unless you pinned
  `LEGION_VIEWER_MCP_TOKEN`.
- **Session traces at rest.** The default-on reasoning trace
  (`~/.legion_prof_viewer/traces/`, see the README) records untruncated tool
  inputs/outputs — SQL, Bash commands and their output, source the agent read.
  Its directory is created `0700` and each file `0600`, so a co-tenant cannot
  read another user's session. Disable with `LEGION_PROF_AI_TRACE=off`.

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

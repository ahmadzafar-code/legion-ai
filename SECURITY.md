# Security model

This document covers the AI co-pilot layer this fork adds to the Legion Prof
viewer. The co-pilot handles two kinds of untrusted input by design: profile
databases (task titles and other strings inside a `.duckdb` you may have
received from someone else) and the profiled application's source tree, which
the agent is encouraged to read. Both flow through a language model, so both
are treated as prompt-injection vectors; every mitigation below assumes they
are attacker-influenceable.

## Network exposure

The in-viewer MCP server binds `127.0.0.1` only, never `0.0.0.0`. Requests
carrying a non-loopback `Origin` header are rejected: a browser can be induced
to POST to localhost, but it cannot fake a loopback `Origin`, so this check
closes DNS rebinding and CSRF. Request bodies are capped at 1 MiB.

The `Origin` check passes when no `Origin` header is present, so it stops
browsers, not local processes — without a further gate, any local process
could drive the tools. Every `POST /mcp` and `POST /approve` therefore
requires `Authorization: Bearer <token>`. The token is random per session,
compared in constant time, and printed once at startup. To keep a stable token
for an external registration, set `LEGION_VIEWER_MCP_TOKEN`.

Same-user local processes that can read this process's memory or environment
are out of scope: they already win by other means, and no guarantees are given
against them.

## Database access

All SQL — the agent's and the overview's — routes through a single executor.
The DuckDB connection is opened read-only with
`enable_external_access(false)`, so `read_text`/`read_csv`-style exfiltration
of local files through SQL is rejected. Query results are row-capped in Rust;
trailing `LIMIT` clauses the model writes are stripped first, so the cap
cannot be talked around.

## Source access

`read_code` and `list_files` are confined to the connected project root:
relative paths only, `..` and absolute prefixes are rejected, and the
effective root is canonicalized, so a symlink inside an untrusted tree cannot
escape it.

## The Claude Code engine

When the panel spawns your `claude` as a subprocess, the child runs headless
with a deliberately shaped surface, described per mechanism below.

### Tool availability

Sub-agent and command-expansion tools (`Task`, `Skill`, `SlashCommand`,
`KillShell`) are not advertised at all (`--tools`), because they could route
around per-call gating.

### Ungated reads

`Read`, `Glob`, `Grep`, `BashOutput`, `TodoWrite`, and the viewer's MCP tools
run without prompts. `BashOutput` only reads the output of already-approved
`Bash` calls; `TodoWrite` writes only harness-internal task state. Two caveats
apply to reading:

- The viewer's `read_code`/`list_files` are root-confined (see
  [Source access](#source-access)), but the harness's own `Read`/`Glob`/`Grep`
  are not: they can read any file your user account can read, not only the
  `--add-dir` root.
- Reading is an egress channel, not just ingestion. Whatever the agent reads
  becomes conversation context sent to the model provider (Anthropic) and is
  also written to the local session trace, so a prompt injection in profile
  data or source could make the agent read an unrelated host file and
  exfiltrate it via the provider or the trace, without tripping the action
  gate.

> **Important:** Reads are ungated by design: an approval prompt on every file
> read would train users to click Always allow, which is arguably worse. Point
> the co-pilot only at profiles and code you trust; every action below is
> gated. Path-scoped gating of out-of-root reads is a considered future
> hardening (tracked in TODO).

### Gated actions

`Bash`, `Edit`, `Write`, `NotebookEdit`, `WebFetch`, and `WebSearch` trigger a
Deny / Allow / Always allow dialog in the viewer for every call, showing the
full untruncated command, path, or URL with a severity badge. The bridge is a
`PreToolUse` hook that POSTs to the viewer's `/approve` route and blocks on
your verdict. It fails closed — no answer is a deny — and a denial is fed back
to the model as feedback, not a crash.

Always-allow rules are session-scoped and in-memory only: they die with
↺ New session or app exit. A `Bash` rule matches a command prefix only when
the command contains no shell metacharacters (such as `;`, `$(`, backticks,
and pipes), so `cargo build; curl evil` can never ride an approved `cargo …`
rule.

### Settings isolation

The child runs with `--setting-sources ""` plus a viewer-owned settings file,
and its working directory is a viewer-owned scratch directory. Claude Code
auto-discovers project settings from its working directory, so the connected
repo is granted via `--add-dir` and never used as the working directory; a
malicious project tree therefore cannot inject `.claude/settings.json`
allow-rules or hooks.

### Process lifetime

The child is spawned as a process-group leader and the whole group is killed
on stop, so a mid-run `Bash` grandchild (or an injected `nohup … &`) cannot
outlive the viewer.

### Token handling

The session token never appears in any process's arguments, because a command
line is readable by other local users (`/proc/<pid>/cmdline`, `ps`) — on a
shared login node that would hand a co-tenant the key to the loopback server.
The token lives only in two viewer-owned temp files: the `--mcp-config` (the
child's `Authorization` header) and a `curl --config` file the approval hook
reads. Both are created atomically at mode `0600`, so there is no
world-readable window, and both are deleted on shutdown.

> **Note:** After a hard crash (SIGKILL), stale `legion_cc_*` files may remain
> until the next session. The token inside is already invalid by then — new
> session, new token — unless you pinned `LEGION_VIEWER_MCP_TOKEN`.

### Session traces at rest

The default-on reasoning trace (`~/.legion_prof_viewer/traces/`; see the
[README](README.md)) records untruncated tool inputs and outputs: SQL, `Bash`
commands and their output, and source the agent read. The trace directory is
created at mode `0700` and each file at `0600`, so a co-tenant cannot read
another user's session. To disable tracing, set `LEGION_PROF_AI_TRACE=off`.

## Prompt injection

The approval dialog makes dangerous calls accountable, not impossible: a user
who reflexively clicks Allow defeats it. The structural mitigations — the
loopback-plus-token server, the read-only database, root-confined source
access, tool availability filtering, settings isolation, and the process-group
kill — carry the real weight, and the system prompt additionally instructs the
model to treat all tool-returned strings as data. If you review one thing
before trusting a shared profile with action tools enabled, review your own
click habits on that dialog.

## Reporting

Please report security issues by opening a GitHub issue on this fork, or a
private security advisory if the issue is sensitive.

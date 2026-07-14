# Legion Prof Viewer — AI Co-Pilot

An AI diagnostic co-pilot built into the Legion Prof timeline viewer. Open a
profile, click **Legion AI Co-Pilot**, and ask questions in plain English —
the agent runs SQL over your profile, looks at the live timeline (it can zoom,
filter, and take screenshots), reads your application's source if you connect
it, and answers with root-cause diagnoses and clickable highlights on the
timeline itself.

> **This is a modified fork of
> [StanfordLegion/prof-viewer](https://github.com/StanfordLegion/prof-viewer)**
> (Apache-2.0). All additions live in `src/ai/`, `src/bin/`, and feature-gated
> blocks of `src/app/core.rs` / `src/main.rs`; a default build (`cargo build`)
> behaves exactly like upstream. Upstream's original README is preserved at the
> bottom of this file.

## Contents

- [What it can do](#what-it-can-do)
- [Requirements](#requirements)
- [Quick start](#quick-start)
- [Step 1 — Profile your Legion application](#step-1--profile-your-legion-application)
- [Step 2 — Generate the viewer inputs](#step-2--generate-the-viewer-inputs)
- [Step 3 — Launch the viewer](#step-3--launch-the-viewer)
- [Step 4 — Ask questions](#step-4--ask-questions)
- [The chat panel, piece by piece](#the-chat-panel-piece-by-piece)
- [Engines and authentication](#engines-and-authentication)
- [Using your own agent over MCP (BYOA)](#using-your-own-agent-over-mcp-byoa)
- [Optional knowledge wiki](#optional-knowledge-wiki)
- [Session traces (test program)](#session-traces-test-program)
- [Security model](#security-model)
- [Feature flags](#feature-flags)
- [Troubleshooting](#troubleshooting)
- [Development](#development)
- [License and acknowledgments](#license-and-acknowledgments)
- [Upstream README](#upstream-readme)

## What it can do

- **Answer "where did the time go?"** — a pre-computed diagnostic overview
  (utilization, idle gaps, task rankings, critical-path signals) plus ad-hoc
  SQL over a DuckDB export of your profile. Every number the agent states is
  backed by a query you can expand and copy from the transcript.
- **See the timeline like you do** — the agent drives the live viewer: zoom to
  a nanosecond range, filter processor kinds, scroll to a row, search, and
  capture screenshots that it reads as images.
- **Mark what it finds** — diagnoses arrive as timeline **highlights** with
  severity and labels; manage them in the sidebar's highlight manager and
  click a chip to zoom to the evidence.
- **Read your code** — use **+ → Connect Code…** to point it at the profiled
  application's source; it explains what a slow task actually computes
  (`--code <dir>` does the same at launch).
- **Answer about a selection** — click a task bar or shift-drag a time range
  in the timeline, then ask "what's happening here?"; the selection rides
  along as context.
- **Stay under your control** — a turn in flight can be stopped with the
  square stop button; anything that touches your machine (shell, file edits,
  web) raises a Deny / Allow / Always-allow dialog first.

## Requirements

| Requirement | Needed for | Notes |
|---|---|---|
| Rust toolchain (stable) | building the viewer | first build compiles DuckDB's C++ — expect 5–10 minutes once, seconds afterwards |
| Linux GUI libraries | native viewer on Linux | package list in the [upstream README](#upstream-readme) below |
| `legion_prof` (from the [Legion repo](https://github.com/StanfordLegion/legion)) | producing profile inputs | `cargo install --locked --all-features --path legion/tools/legion_prof_rs` |
| **Claude Code CLI** ≥ 2.1 (`claude` on PATH) + `curl` | the recommended AI engine | one-time `claude login` (Pro/Max subscription), *or* an `ANTHROPIC_API_KEY` in the environment |
| — or just an `ANTHROPIC_API_KEY` | the built-in fallback engine | zero extra installs; plain HTTPS to the Anthropic API |

The AI layer is **native-only** (macOS / Linux / Windows). Wasm builds serve
the plain upstream viewer.

## Quick start

```sh
# 1. Build the viewer with the full AI layer
git clone https://github.com/ahmadzafar-code/prof-viewer.git
cd prof-viewer
cargo build --release --features viewer-mcp

# 2. Profile your Legion app, then convert the logs (see Steps 1–2)
legion_prof archive -o myrun_archive prof_*.gz
legion_prof duckdb  -o myrun_db      prof_*.gz

# 3. Launch — the *_db file is auto-detected next to the *_archive
./target/release/legion_prof_viewer myrun_archive

# 4. Click "Legion AI Co-Pilot" (top right) and ask:
#      "Give me an overview of this profile"
```

If the welcome screen says Claude Code isn't signed in, run `claude login` in
any terminal — the hint flips to ready within seconds, no restart needed.

## Step 1 — Profile your Legion application

Run your application with Legion's profiler enabled, e.g.:

```sh
./my_app -lg:prof <N> -lg:prof_logfile prof_%.gz
```

where `<N>` is the number of nodes to profile and `%` expands to the node
number. This produces the standard `prof_*.gz` logs that every `legion_prof`
workflow starts from. (See the
[Legion profiling docs](https://legion.stanford.edu/profiling/) for details.)

## Step 2 — Generate the viewer inputs

The co-pilot uses **two artifacts**, both produced by `legion_prof`:

```sh
legion_prof archive -o myrun_archive prof_*.gz   # timeline for the viewer
legion_prof duckdb  -o myrun_db      prof_*.gz   # database for the SQL tools
```

- The **archive** is what you open in the viewer (same as upstream).
- The **DuckDB database** is what the agent queries. `legion_prof duckdb` uses
  the writer this repository ships, so the schema always matches the tools.

**Naming convention worth keeping**: name the database `<base>_db` for an
archive named `<base>_archive` (as above) — or give it any `*.duckdb` / `*_db`
name in the same directory — and the viewer **auto-detects** it, so you never
pass `--duckdb` at all.

Only have an archive (e.g. one someone shared with you)? Convert it directly:

```sh
cargo run --release --features duckdb --example prof2duckdb -- \
    myrun_archive -o myrun_db
```

## Step 3 — Launch the viewer

```sh
legion_prof_viewer <archive-dir-or-URL> \
    [--duckdb <path>]   # profile database (skip if auto-detected)
    [--code   <dir>]    # profiled application's source
    [--wiki   <dir>]    # Legion knowledge wiki (optional)
```

Everything passed by flag can also be connected later from inside the panel
(the **+** menu), and connected paths persist across restarts — CLI flags win
over persisted values when both exist.

## Step 4 — Ask questions

Open the panel with the **Legion AI Co-Pilot** button (top right). Good first
questions:

- *"Give me an overview of this profile — what ran, where the time went, and
  anything unusual."*
- *"Highlight the largest idle gaps and find what's preventing that work from
  starting earlier."*
- *"Why is `update_voltages` so slow?"* (connect your code first)
- Shift-drag a region on the timeline, then: *"What's happening in this
  region?"*

The agent narrates as it works — every tool call it makes (`run_query`,
`set_view`, `highlight`, …) appears as an expandable row in the transcript, so
you can audit exactly which SQL produced which number.

## The chat panel, piece by piece

| Control | What it does |
|---|---|
| **Legion AI Co-Pilot** (top bar, right) | shows/hides the chat panel |
| **Sidebar** (top bar, left) | shows/hides the controls sidebar — more room for timeline + chat |
| **DB / Code / Visual chips** | live status of the agent's three tool groups; hover for detail |
| **+ menu** (composer) | **Connect DuckDB…**, **Connect Code…** (lets the agent read your source), **Add file…** (attach a text file as context) — connected items show as chips with **×** to disconnect |
| **Send / Stop button** | send when you've typed something; during a turn it becomes a square **stop** button — one click gracefully interrupts the agent (the session survives, keep chatting) |
| **↺** (panel header) | hard reset: kills the engine process and starts a fresh session |
| **Selection chip** | click a task bar or shift-drag a range in the timeline, and your next question includes it |
| **Highlights** (left sidebar) | every diagnosis the agent marks lands here — toggle, zoom to, or clear |
| **Done. (tokens: …)** | per-turn token and cost line, straight from the engine's own usage report |
| **Copy transcript / Copy** | export the full conversation (screenshots are elided as `[image … KB]` placeholders) |

## Engines and authentication

The panel picks its engine automatically from what your machine has:

| You have | Engine used | Auth |
|---|---|---|
| `claude` CLI installed | **your Claude Code**, spawned headless against the viewer's local MCP server | one-time `claude login`, *or* `ANTHROPIC_API_KEY` (inherited) |
| no `claude` | **built-in API loop** | `ANTHROPIC_API_KEY` env var, or the key popup on first use |

The Claude Code engine is preferred when available: it brings the full agent
harness — its own file tools over your connected repo, shell/web behind the
approval dialog — on your existing subscription or key, with whatever model
your `claude` install is configured for. The built-in engine is the
zero-install fallback.

The welcome screen tells you where you stand: if Claude Code isn't signed in
it shows the one-time `claude login` step; once signed in it suggests
connecting your code.

## Using your own agent over MCP (BYOA)

The viewer runs a loopback-only HTTP MCP server exposing the data, source,
wiki, and visual-timeline tools. At startup it prints a ready-to-paste
registration:

```sh
claude mcp add --transport http legion-viewer \
    http://127.0.0.1:8765/mcp --header "Authorization: Bearer <token>"
```

Any MCP-capable agent can drive the profiler through it. The bearer token is
random per session; set `LEGION_VIEWER_MCP_TOKEN` for a stable registration
across restarts. Port 8765 is preferred, with an ephemeral fallback (the real
port is printed at startup).

A headless stdio variant (data tools only, no GUI) ships as the `mcp` bin:

```sh
cargo run --features ai,duckdb --bin mcp -- --duckdb <db> [--code-root <dir>]
```

## Optional knowledge wiki

The `wiki_*` tools serve a curated Legion-concepts corpus (task lifecycle,
mapper behavior, common bottleneck patterns) that the agent consults when
diagnosing. Point `--wiki <dir>` at a corpus; `wiki-legion/wiki` relative to
the launch directory is auto-detected. The corpus used during development is
published separately — see this fork's release notes.

## Session traces (test program)

While this fork is in its evaluation phase, the viewer records a **local
reasoning transcript** of each chat session so the team can replay how a
diagnosis was reached and improve the product. On the first question of a
session it prints where the file lives:

```
[legion-ai] session trace: ~/.legion_prof_viewer/traces/session_<id>.jsonl
            (set LEGION_PROF_AI_TRACE=off to disable)
```

**What's recorded** (JSON Lines, one event per line): your prompts, the
agent's narration and thinking, every tool call **with its full input**
(e.g. the exact SQL), tool results, per-turn token usage/cost, stop clicks,
and errors. **What's not**: screenshot image bytes are replaced with a
`[image … KB elided]` note, and nothing is uploaded anywhere — the trace is a
plain local file.

- **Disable**: `LEGION_PROF_AI_TRACE=off` (or `0`/`false`).
- **Relocate**: `LEGION_PROF_AI_TRACE_DIR=<dir>`.
- **Share with the team**: zip your `~/.legion_prof_viewer/traces/` folder and
  attach it to your feedback. Traces contain your prompts, profile-derived
  numbers, and any source snippets the agent read — skim before sharing if
  your application code is sensitive.

(Separately, `LEGION_PROF_AI_TRACE_DIR` also enables the low-level span-timing
log for the built-in engine — `agent_traces/agent.jsonl` — mainly of interest
to maintainers.)

## Security model

Short version (full details in [SECURITY.md](SECURITY.md)):

- The MCP server binds **127.0.0.1 only**, requires a **per-session bearer
  token** on every request, and rejects non-local `Origin`s.
- Engine tool calls that touch your machine (Bash, file edits, web fetch)
  block on a **Deny / Allow / Always-allow** dialog in the viewer, showing the
  full command — never a truncated preview.
- The spawned Claude Code child runs with an isolated settings file and a
  neutral working directory, so repository-local `.claude/` configuration is
  never picked up implicitly.
- Profile data and connected source are sent to the model (Anthropic API) as
  conversation context — connect only code you're comfortable sharing with
  your configured provider.

## Feature flags

| Build | What you get |
|---|---|
| `cargo build` | the plain upstream viewer (no AI) |
| `--features ai` | chat panel + built-in API engine (no SQL tools) |
| `--features ai,duckdb` | + DuckDB data tools (`run_query`, overview, …) |
| `--features viewer-mcp` | + in-viewer MCP server + the Claude Code engine (implies `ai,duckdb`) — **the recommended build** |
| `--features eval` | + the oracle-graded eval harness (`eval` bin; maintainers) |

Session reasoning traces are ON by default during the test program — see
[Session traces](#session-traces-test-program).

## Troubleshooting

| Symptom | Fix |
|---|---|
| Welcome screen: "Claude Code isn't signed in yet" | run `claude login` in any terminal; the hint updates within seconds |
| First turn errors with 401 | same as above — or export `ANTHROPIC_API_KEY`, then **↺** for a fresh session |
| `The socket connection was closed unexpectedly` on one tool call | benign transport race between Claude Code's HTTP client and the viewer's one-shot connections; the agent retries and succeeds |
| First `cargo build` takes ~10 minutes | DuckDB's C++ compiles once and is cached afterwards |
| Panel uses the built-in engine although `claude` is installed | make sure `claude` resolves on the PATH of the shell that launched the viewer |
| No SQL tools / "DB ○" chip gray | pass `--duckdb`, use the naming convention from Step 2, or **+ → Connect DuckDB…** |
| Linux: viewer fails to start | install the GUI packages listed in the [upstream README](#upstream-readme) |
| Stop button doesn't appear while running | it replaces the send button only for the Claude Code engine; the built-in engine can't be interrupted mid-turn (use **↺**) |

## Development

```sh
cargo check --features ai,duckdb
cargo clippy --features ai,duckdb -- -W clippy::all
cargo test  --features ai,duckdb
# claude_code.rs / viewer_mcp.rs compile ONLY under viewer-mcp:
cargo test  --features viewer-mcp
```

All five feature combinations must compile: `{}`, `{ai}`, `{duckdb}`,
`{ai,duckdb}`, `{viewer-mcp}`. The AI layer lives in `src/ai/` (agent loop,
tools, chat panel, MCP core + HTTP transport, Claude Code backend); see
[CONTRIBUTING.md](CONTRIBUTING.md) for the upstream contribution process.

## License and acknowledgments

Apache-2.0, same as upstream — see [LICENSE.txt](LICENSE.txt). Built on the
[Legion](https://legion.stanford.edu/) ecosystem and the
[StanfordLegion/prof-viewer](https://github.com/StanfordLegion/prof-viewer)
frontend; the AI layer talks to [Anthropic](https://www.anthropic.com/)'s
Claude models via your own Claude Code install or API key.

---

# Upstream README


This repository contains the Legion Prof frontend in Rust. The frontend here is
intended to be used with Legion Prof and is not (typically) used
standalone. Most users want the integrated version (i.e., that can parse Legion
Prof logs and generate a visualization). To use the integrated version of
Legion Prof, clone the [Legion
repository](https://github.com/StanfordLegion/legion) and run:

```
git clone https://github.com/StanfordLegion/legion.git
cargo install --locked --all-features --path legion/tools/legion_prof_rs
```

To start a native viewer right away, run:

```
legion_prof view prof_*.gz
```

To start a server (and attach a viewer to it), run (in separate shells):

```
legion_prof serve prof_*.gz
legion_prof attach http://127.0.0.1:8080/
```

If you really want to run the frontend by itself, continue to the instructions
below.

## Quickstart

### Native

Run:

```
cargo run --release <URL>
```

Ubuntu dependencies:

```
sudo apt-get install libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libspeechd-dev libxkbcommon-dev libssl-dev
```

Fedora Rawhide dependencies:

```
dnf install clang clang-devel clang-tools-extra speech-dispatcher-devel libxkbcommon-devel pkg-config openssl-devel libxcb-devel fontconfig-devel
```

### Web Locally

Install dependencies:

```
cargo install --locked trunk
```

Then run:

```
trunk serve
```

Go to <http://127.0.0.1:8080/#dev> in your browser. (The `#dev` skips
client-side caching, so that you don't need to clear your browser cache as you
develop the app.)

### Web Deploy

Install `trunk` as above. Then run:

```
trunk build --release
```

This will generate a static site under `dist` that you can upload. Note that
`trunk` by default assumes the site will live in the root of the domain (e.g.,
`https://example.com/`). If that is not true, add `--public-url ...` to the
`trunk` command where `...` is the path the build is hosted under (e.g.,
`https://example.com/.../`).

### Web Auto-Deploy

This repository is configured via GitHub Actions to deploy automatically on
each push to the `master` branch. You can test it at
<https://legion.stanford.edu/prof-viewer/?url=https://...> where
`https://...` is the URL of the profile to load.

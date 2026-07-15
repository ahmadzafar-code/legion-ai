# Legion AI

**Legion AI** is an AI diagnostic co-pilot built into the Legion Prof timeline
viewer. Open a profile, click **Legion AI**, and ask questions in plain
English — the agent runs SQL over your profile, drives the live timeline
(zoom, filter, screenshots), reads your application's source if you connect
it, and answers with root-cause diagnoses and clickable highlights on the
timeline itself.

> **This is a modified fork of
> [StanfordLegion/prof-viewer](https://github.com/StanfordLegion/prof-viewer)**
> (Apache-2.0). A default build (`cargo build`) behaves exactly like upstream;
> everything else is feature-gated. Reviewers: see
> [For reviewers & development](#for-reviewers--development) and
> [docs/UPSTREAM-DELTA.md](docs/UPSTREAM-DELTA.md).

## Contents

- [Quick start](#quick-start)
- [What it can do](#what-it-can-do)
- [Using the co-pilot](#using-the-co-pilot)
- [Connecting your application source](#connecting-your-application-source)
- [Troubleshooting](#troubleshooting)
- [How it works & security](#how-it-works--security)
- [Session traces (test program)](#session-traces-test-program)
- [Advanced](#advanced)
- [For reviewers & development](#for-reviewers--development)
- [License and acknowledgments](#license-and-acknowledgments)

## Quick start

### 0. Prerequisites (once)

| What | How | Why |
|---|---|---|
| Rust ≥ 1.85 | `rustup update` | edition-2024 crate |
| C / C++ toolchain | Linux: `sudo apt-get install build-essential` · macOS: `xcode-select --install` | first build compiles DuckDB's C++ (~5–10 min once, cached afterwards) |
| Linux GUI libraries (Linux only) | `sudo apt-get install libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libspeechd-dev libxkbcommon-dev libssl-dev` | native egui viewer (Fedora list in [Troubleshooting](#troubleshooting)) |
| `legion_prof` | `git clone https://github.com/StanfordLegion/legion.git && cargo install --locked --all-features --path legion/tools/legion_prof_rs` | converts profiler logs into the viewer's inputs |
| **Claude Code** ≥ 2.1 + `curl` | install from [claude.com/claude-code](https://claude.com/claude-code) — `npm install -g @anthropic-ai/claude-code` (or the native installer) — then run **`claude auth login`** once | **the AI engine.** Uses your existing Claude subscription (Pro/Max), or an `ANTHROPIC_API_KEY` in the environment (the spawned CLI inherits it) |

### 1. Build (or download) the viewer

```sh
git clone https://github.com/ahmadzafar-code/legion-ai.git
cd legion-ai
cargo build --release --features viewer-mcp
```

**Prefer a prebuilt binary?** Tagged releases attach Linux (x86_64) and macOS
(arm64) binaries — the full AI build — on the
[Releases page](https://github.com/ahmadzafar-code/legion-ai/releases).
macOS note: the binary is unsigned; after extracting, clear quarantine with
`xattr -d com.apple.quarantine legion_prof_viewer` (or right-click → Open the
first time).

### 2. Profile your app and convert the logs

```sh
# Run with Legion's profiler enabled (Legate: `legate --profile ...`):
./my_app -lg:prof <N> -lg:prof_logfile prof_%.gz

# Convert the logs into the viewer's two inputs:
legion_prof archive -o myrun_archive prof_*.gz   # timeline for the viewer
legion_prof duckdb  -o myrun_db      prof_*.gz   # database for the SQL tools
```

Name the database `<base>_db` next to `<base>_archive` (or any `*.duckdb` /
`*_db` file in the same directory) and the viewer **auto-detects** it — you
never pass `--duckdb`.

### 3. Launch and ask

```sh
./target/release/legion_prof_viewer myrun_archive
```

Click **Legion AI** (top right) and ask:

> *"Give me an overview of this profile — what ran, where the time went, and
> anything unusual."*

If the welcome screen says Claude Code isn't signed in, run `claude auth login`
in any terminal — the hint flips to ready within seconds, no restart needed.

**Profiling a Legate / cuNumeric application?** Legate programs run on Legion,
so the same flow applies — pass the profiling flags through Legate's launcher
(`legate --profile --logging ... your_app.py`) and feed the resulting
`prof_*.gz` through step 2 unchanged. The co-pilot detects Python/Legate
processors and factors them into its diagnosis.

## What it can do

- **Answer "where did the time go?"** — a pre-computed diagnostic overview
  (utilization, idle gaps, task rankings, critical-path signals) plus ad-hoc
  SQL over a DuckDB export of your profile. Every number the agent states is
  backed by a query you can expand and copy from the transcript.
- **See the timeline like you do** — the agent drives the live viewer: zoom to
  a nanosecond range, filter processor kinds, scroll to a row, search, and
  capture screenshots that it reads as images.
- **Mark what it finds** — diagnoses arrive as timeline **highlights** with
  labels; manage them in the sidebar's highlight manager and click a chip to
  zoom to the evidence.
- **Read your code** — connect the profiled application's source and it
  explains what a slow task actually computes.
- **Answer about a selection** — click a task bar or shift-drag a time range,
  then ask "what's happening here?"; the selection rides along as context.
- **Stay under your control** — a turn in flight can be stopped with the
  square stop button; anything that touches your machine (shell, file edits,
  web) raises a Deny / Allow / Always-allow dialog first.

## Using the co-pilot

Good first questions:

- *"Give me an overview of this profile — what ran, where the time went, and
  anything unusual."*
- *"Highlight the largest idle gaps and find what's preventing that work from
  starting earlier."*
- *"Why is `update_voltages` so slow?"* (connect your code first)
- Shift-drag a region on the timeline, then: *"What's happening in this
  region?"*

The agent narrates as it works — every tool call (`run_query`, `set_view`,
`highlight`, …) appears as an expandable row in the transcript, so you can
audit exactly which SQL produced which number.

| Control | What it does |
|---|---|
| **Legion AI** (top bar, right) | shows/hides the chat panel |
| **Sidebar** (top bar, left) | shows/hides the controls sidebar — more room for timeline + chat |
| **DB / Code / Wiki / Visual chips** | live status of the agent's tool groups; hover for detail |
| **+ menu** (composer) | **Connect DuckDB…**, **Connect Code…**, **Add file…** (attach a text file as context) — connected items show as chips with **×** to disconnect |
| **Model · Strength picker** (composer) | model tier (**Default** / **Fable** / **Opus** / **Sonnet** / **Haiku**) and reasoning strength (**Default** / **Low** / **Medium** / **High** / **Max**). Default inherits your own Claude Code configuration; a change applies on your next message |
| **Send / Stop** | send when you've typed something; during a turn it becomes a square **stop** button — one click gracefully interrupts (the session survives, keep chatting) |
| **↺** (panel header) | hard reset: kills the engine process and starts a fresh session |
| **Selection chip** | click a task bar or shift-drag a range, and your next question includes it |
| **Highlights** (left sidebar) | every diagnosis the agent marks lands here — toggle, zoom to, or clear |
| **Done. (tokens: …)** | per-turn token and cost line, straight from the engine's own usage report |
| **Copy transcript / Copy** | export the conversation (screenshots elided as `[image … KB]` placeholders) |

## Connecting your application source

Use **+ → Connect Code…** to point the agent at the profiled application's
source tree (or pass `--code <dir>` at launch). The agent then reads the
functions behind slow tasks and explains what they actually compute.
Connected paths persist across restarts; CLI flags win over persisted values.

Connect only code you're comfortable sending to your configured model
provider — source the agent reads becomes conversation context.

## Troubleshooting

| Symptom | Fix |
|---|---|
| Welcome screen: "Claude Code isn't signed in yet" | run `claude auth login` in any terminal; the hint updates within seconds |
| First turn errors with 401 | same as above, then **↺** for a fresh session |
| Panel says Claude Code isn't available although it's installed | make sure `claude` resolves on the PATH of the shell that launched the viewer; when launching from Finder/an IDE, start from a terminal instead (or symlink `claude` into `/usr/local/bin`) |
| `cc` / `c++` not found during first build | `sudo apt-get install build-essential` (Linux) or `xcode-select --install` (macOS) |
| Error about `edition2024` / rustc version | `rustup update` (needs Rust ≥ 1.85) |
| First `cargo build` takes ~10 minutes | DuckDB's C++ compiles once and is cached afterwards |
| No SQL tools / "DB ○" chip gray | pass `--duckdb`, use the naming convention from step 2, or **+ → Connect DuckDB…** |
| Port 8765 in use | the viewer picks an ephemeral port and prints it; re-run the printed `claude mcp add` line if you registered an external agent |
| `The socket connection was closed unexpectedly` on one tool call | transient transport error; the viewer now answers 408 and logs the cause to stderr — the agent retries and succeeds |
| Linux: viewer fails to start | install the GUI packages from [Prerequisites](#0-prerequisites-once); Fedora: `dnf install clang clang-devel clang-tools-extra speech-dispatcher-devel libxkbcommon-devel pkg-config openssl-devel libxcb-devel fontconfig-devel` |
| macOS: "cannot be opened" on a prebuilt binary | `xattr -d com.apple.quarantine legion_prof_viewer`, or right-click → Open |

## How it works & security

Legion AI runs on **your own Claude Code**, spawned headless against a local
MCP server inside the viewer. Authentication is whatever your `claude` already
uses: a one-time `claude auth login` (Pro/Max subscription) or an
`ANTHROPIC_API_KEY` in the environment (inherited by the spawned CLI). There
is no separate account, server, or telemetry. (A built-in direct-API engine
exists in the code but is currently disabled.)

Security model, short version (full details in [SECURITY.md](SECURITY.md)):

- The MCP server binds **127.0.0.1 only**, requires a **per-session bearer
  token** on every request, and rejects non-local `Origin`s.
- Engine tool calls that touch your machine (shell, file edits, web fetch)
  block on a **Deny / Allow / Always-allow** dialog in the viewer, showing the
  full command — never a truncated preview.
- The spawned Claude Code child runs with an isolated settings file and a
  neutral working directory, so repository-local `.claude/` configuration is
  never picked up implicitly.
- Profile data and connected source are sent to the model (Anthropic API) as
  conversation context — connect only code you're comfortable sharing with
  your configured provider.

## Session traces (test program)

While this fork is in its evaluation phase, the viewer records a **local
reasoning transcript** of each chat session so the team can replay how a
diagnosis was reached and improve the product. On the first question of a
session it prints where the file lives:

```
[legion-ai] session trace: ~/.legion_prof_viewer/traces/session_<id>.jsonl
            (set LEGION_PROF_AI_TRACE=off to disable)
```

**What's recorded** (JSON Lines): your prompts, the agent's narration and
thinking, every tool call **with its full input** (e.g. the exact SQL), tool
results, per-turn token usage/cost, stop clicks, and errors. **What's not**:
screenshot image bytes are replaced with a `[image … KB elided]` note, and
nothing is uploaded anywhere — the trace is a plain local file.

- **Disable**: `LEGION_PROF_AI_TRACE=off` (or `0`/`false`).
- **Relocate**: `LEGION_PROF_AI_TRACE_DIR=<dir>`.
- **Share with the team**: zip `~/.legion_prof_viewer/traces/` and attach it
  to your feedback. Traces contain your prompts, profile-derived numbers, and
  any source snippets the agent read — skim before sharing if your application
  code is sensitive.

## Advanced

### Full CLI

```sh
legion_prof_viewer <archive-dir-or-URL> \
    [--duckdb <path.duckdb>]   # profile database (skip if auto-detected)
    [--code   <dir>]           # profiled application's source
    [--wiki   <dir>]           # Legion knowledge wiki (optional)
```

Everything passed by flag can also be connected later from the panel's **+**
menu; connected paths persist across restarts (CLI flags win when both exist).

### Only have an archive?

Convert it to a database directly (no `prof_*.gz` needed):

```sh
cargo run --release --features duckdb --example prof2duckdb -- \
    myrun_archive -o myrun_db
```

The DuckDB writer is shared with upstream prof-viewer (this fork does not
modify it), so any recent `legion_prof` produces a database with the schema
the tools expect.

### Optional knowledge wiki

The `wiki_*` tools serve a curated Legion-concepts corpus (task lifecycle,
mapper behavior, common bottleneck patterns) that the agent consults when
diagnosing. Point `--wiki <dir>` at a corpus; `wiki-legion/wiki` relative to
the launch directory is auto-detected. The corpus used during development is
published separately — see this fork's release notes.

### Using your own agent over MCP (BYOA)

The viewer runs a loopback-only HTTP MCP server exposing the data, source,
wiki, and visual-timeline tools. At startup it prints a ready-to-paste
registration:

```sh
claude mcp add --transport http legion-viewer \
    http://127.0.0.1:8765/mcp --header "Authorization: Bearer <token>"
```

Any MCP-capable agent can drive the profiler through it. The bearer token is
random per session; set `LEGION_VIEWER_MCP_TOKEN` for a stable registration.
Port 8765 is preferred, with an ephemeral fallback (the real port is printed
at startup).

A headless stdio variant (data tools only, no GUI) ships as the `mcp` bin:

```sh
cargo run --features ai,duckdb --bin mcp -- --duckdb <db> [--code-root <dir>]
```

## For reviewers & development

**Fork layout.** All AI code lives in `src/ai/`, `src/bin/`, and
`src/app/core/legion_ai.rs` (a child module holding every AI addition to the
viewer core — upstream files carry only thin `#[cfg(feature = "ai")]`-gated
call sites). [docs/UPSTREAM-DELTA.md](docs/UPSTREAM-DELTA.md) maps the full
delta against upstream and how to review it.

| Build | What you get |
|---|---|
| `cargo build` | the plain upstream viewer (no AI) |
| `--features ai` | chat panel + UI (the engine requires Claude Code, enabled by `viewer-mcp`) |
| `--features ai,duckdb` | + DuckDB data tools (`run_query`, overview, …) |
| `--features viewer-mcp` | + in-viewer MCP server + the Claude Code engine (implies `ai,duckdb`) — **the recommended build** |
| `--features eval` | + the oracle-graded eval harness (`eval` bin; maintainers) |

```sh
cargo check --features ai,duckdb
cargo clippy --features ai,duckdb -- -W clippy::all
cargo test  --features ai,duckdb
# claude_code.rs / viewer_mcp.rs compile ONLY under viewer-mcp:
cargo test  --features viewer-mcp
```

All five feature combinations must compile: `{}`, `{ai}`, `{duckdb}`,
`{ai,duckdb}`, `{viewer-mcp}`. See [CONTRIBUTING.md](CONTRIBUTING.md).

## License and acknowledgments

Apache-2.0, same as upstream — see [LICENSE.txt](LICENSE.txt). Built on the
[Legion](https://legion.stanford.edu/) ecosystem and the
[StanfordLegion/prof-viewer](https://github.com/StanfordLegion/prof-viewer)
frontend (original README preserved at
[docs/UPSTREAM-README.md](docs/UPSTREAM-README.md)); the AI layer talks to
[Anthropic](https://www.anthropic.com/)'s Claude models via your own Claude
Code install or API key.

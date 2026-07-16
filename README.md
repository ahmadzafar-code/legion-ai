# Legion AI

Legion AI is an AI diagnostic co-pilot built into the Legion Prof timeline
viewer: ask plain-English questions about a Legion or Legate profile and it
answers with root-cause diagnoses, backed by SQL it runs over your profile
and marked as clickable highlights on the live timeline.

![Legion AI system architecture](docs/img/architecture.png)

> **Important:** This is a modified fork of
> [StanfordLegion/prof-viewer](https://github.com/StanfordLegion/prof-viewer)
> (Apache-2.0). A default build (`cargo build`) behaves exactly like upstream;
> everything else is feature-gated — see
> [For reviewers & development](#for-reviewers--development).

## Contents

- [Install](#install)
- [Two ways to use it](#two-ways-to-use-it)
- [Capabilities](#capabilities)
- [Example questions](#example-questions)
- [The chat panel](#the-chat-panel)
- [Troubleshooting](#troubleshooting)
- [Security](#security)
- [Session traces](#session-traces)
- [Advanced](#advanced)
- [For reviewers & development](#for-reviewers--development)
- [License and acknowledgments](#license-and-acknowledgments)

## Install

### 1. Build the viewer

Requires Rust ≥ 1.85 (`rustup update`) and a C/C++ toolchain —
`build-essential` on Linux, `xcode-select --install` on macOS:

```sh
# Linux only — GUI libraries (Fedora list in Troubleshooting):
$ sudo apt-get install libxcb-render0-dev libxcb-shape0-dev \
      libxcb-xfixes0-dev libspeechd-dev libxkbcommon-dev libssl-dev

$ git clone https://github.com/ahmadzafar-code/legion-ai.git
$ cd legion-ai
$ cargo build --release --features viewer-mcp
```

The first build compiles DuckDB's C++ and takes 5–10 minutes; later builds
are fast. Prebuilt Linux (x86_64) and macOS (arm64) binaries are attached to
tagged releases on the
[Releases page](https://github.com/ahmadzafar-code/legion-ai/releases) as
they are published.

> **Note:** The macOS binary is unsigned. After extracting, clear quarantine
> with `xattr -d com.apple.quarantine legion_prof_viewer`, or right-click →
> Open the first time.

### 2. Convert your profiler logs

Conversion requires `legion_prof`, built from the same Legion source tree
your application runs on — it rejects logs from a mismatched Legion version:

```sh
$ cargo install --locked --all-features --path <legion-source>/tools/legion_prof_rs
```

> **Note:** `--all-features` is required: the `duckdb` subcommand is not in
> `legion_prof`'s default feature set.

Run your application with the profiler enabled, then convert:

```sh
$ ./my_app -lg:prof <N> -lg:prof_logfile prof_%.gz

$ legion_prof archive -o myrun_archive prof_*.gz   # timeline (a directory)
$ legion_prof duckdb  -o myrun_db      prof_*.gz   # SQL database (a file)
```

Name the database `<base>_db` next to `<base>_archive` and the viewer
auto-detects it (pass `--duckdb` to choose among several). Legate and
cuNumeric applications work the same way: profile through Legate's launcher
(`legate --profile ...`) and convert the resulting `prof_*.gz` as above.

### 3. Install Claude Code and launch

The AI engine is your own Claude Code (≥ 2.1, plus `curl`); without it, the
viewer works as a plain timeline viewer. Install it from
[claude.com/claude-code](https://claude.com/claude-code)
(`npm install -g @anthropic-ai/claude-code`), then run `claude auth login`
once in your terminal — it uses your Claude subscription (Pro/Max) or an
`ANTHROPIC_API_KEY` in the environment.

```sh
$ ./target/release/legion_prof_viewer myrun_archive
```

If the welcome screen says Claude Code isn't signed in, run
`claude auth login` in your terminal — not in the chat panel; the hint flips
to ready within seconds.

If you normally run `legion_prof view`, this binary is the AI-enabled
replacement for that frontend: it reads the same archives.

## Two ways to use it

1. **Embedded chat** — click **Legion AI** (top right). The panel spawns your
   own Claude Code headless against the viewer's local MCP server.
2. **Your own Claude Code / IDE** — at startup the viewer prints a
   ready-to-paste registration; run it once and drive the profiler from the
   tool you already work in:

```sh
$ claude mcp add --transport http legion-viewer \
    http://127.0.0.1:8765/mcp --header "Authorization: Bearer <token>"
```

Any MCP-capable client works — Claude Code in a terminal, an LLM IDE, or a
custom agent. Both ways expose the same tools. The bearer token is random per
session (`LEGION_VIEWER_MCP_TOKEN` pins it); the real port is printed at
startup.

## Capabilities

- **The profile database (DuckDB)** — ask it to compute anything: time
  attribution, task rankings, idle gaps, critical paths. Every number it
  states is backed by a query you can expand in the transcript.
- **The live GUI** — ask it to render, navigate, or highlight anything, or to
  explain what you're seeing; it zooms, filters, screenshots, and marks its
  findings on the timeline. Click a task or shift-drag a range and your
  selection rides along as context.
- **Legion concepts (built-in wiki)** — it grounds its reasoning in a curated
  corpus of Legion knowledge: task lifecycle, mapping, common bottleneck
  patterns.
- **Your code (optional)** — use **+ → Connect Code** (or `--code <dir>`) and
  it reads the functions behind slow tasks to analyze, explain, and suggest
  changes.

The possibilities are wide — ask freely. Every tool call appears as an
expandable row in the transcript, so each answer is auditable.

## Example questions

- "Give me an overview of this profile — what ran, where the time went, and
  anything unusual."
- Shift-drag an idle gap, then: "What's blocking here — why can't the next
  task start earlier?"
- "Highlight the ten longest-running tasks."
- "Is this run compute-, communication-, or runtime-bound?"
- "Which GPUs sit idle, and what are they waiting on?"
- "Why is `update_voltages` slow, and what would you change?" (with your
  code connected)

## The chat panel

| Control | What it does |
|---|---|
| DB / Code / Wiki / Visual chips | live status of the agent's tool groups; hover for detail |
| + menu | Connect DuckDB…, Connect Code…, Add file… — connected items show as chips with × to disconnect |
| Model · Strength picker | model tier and reasoning strength; Default inherits your Claude Code configuration, changes apply on the next message |
| Send / Stop | during a turn the send button becomes a square stop — one click interrupts, the session survives |
| ↺ (header) | hard reset: kills the engine process and starts a fresh session |
| Highlights (left sidebar) | every diagnosis the agent marks lands here — toggle, zoom to, or clear |
| Done. (tokens: …) | per-turn token and cost line from the engine's own usage report |

## Troubleshooting

| Symptom | Fix |
|---|---|
| Welcome screen: "Claude Code isn't signed in yet" | run `claude auth login` in your terminal (not in the chat panel); the hint updates within seconds |
| First turn errors with 401 | same as above, then ↺ for a fresh session |
| Panel says Claude Code isn't available although it's installed | make sure `claude` resolves on the PATH of the shell that launched the viewer; when launching from Finder or an IDE, start from a terminal instead |
| `cc` / `c++` not found during first build | `sudo apt-get install build-essential` (Linux) or `xcode-select --install` (macOS) |
| Error about `edition2024` / rustc version | `rustup update` (needs Rust ≥ 1.85) |
| `legion_prof duckdb` fails: unknown subcommand, or panics "not built with the duckdb feature" | reinstall with `cargo install --locked --all-features --path <legion-source>/tools/legion_prof_rs` — the subcommand requires a Legion checkout from June 2025 or later |
| `legion_prof archive` / `duckdb` panics on your logs | `legion_prof` only reads logs from the Legion version it was built from — rebuild it from the source tree your application runs on |
| No SQL tools / "DB ○" chip gray | pass `--duckdb`, use the naming convention from [step 2](#2-convert-your-profiler-logs), or + → Connect DuckDB… |
| Port 8765 in use | the viewer picks an ephemeral port and prints it; re-run the printed `claude mcp add` line if you registered an external agent |
| `The socket connection was closed unexpectedly` on one tool call | transient transport error; the viewer answers 408 and logs the cause — the agent retries and succeeds |
| Linux: viewer fails to start | install the GUI packages from [step 1](#1-build-the-viewer); Fedora: `dnf install clang clang-devel clang-tools-extra speech-dispatcher-devel libxkbcommon-devel pkg-config openssl-devel libxcb-devel fontconfig-devel` |
| macOS: "cannot be opened" on a prebuilt binary | `xattr -d com.apple.quarantine legion_prof_viewer`, or right-click → Open |

## Security

Legion AI adds no separate account, server, or telemetry — authentication is
whatever your `claude` already uses. Full threat model in
[SECURITY.md](SECURITY.md); the short version:

- The MCP server binds `127.0.0.1` only, requires a per-session bearer token
  on every request, and rejects non-local `Origin`s.
- Engine actions that touch your machine (shell, file edits, web fetch) block
  on a Deny / Allow / Always-allow dialog showing the full command.
- The spawned Claude Code child runs with an isolated settings file and a
  neutral working directory — repository-local `.claude/` configuration is
  never picked up.
- Profile data, timeline screenshots, and connected source are sent to the
  model (Anthropic API) as conversation context — connect only code you're
  comfortable sharing with your configured provider.

## Session traces

While this fork is in its evaluation phase, the viewer records a local
reasoning transcript of each session so the team can replay how a diagnosis
was reached:

```
[legion-ai] session trace: ~/.legion_prof_viewer/traces/session_<id>.jsonl
            (set LEGION_PROF_AI_TRACE=off to disable)
```

Recorded: prompts, narration, every tool call with its full input, results,
token usage, and errors. Screenshot bytes are elided, and nothing is uploaded
— the trace is a plain local file. `LEGION_PROF_AI_TRACE=off` disables it;
`LEGION_PROF_AI_TRACE_DIR=<dir>` relocates it. To share feedback, zip the
traces folder — skim first if your application code is sensitive.

## Advanced

### Full CLI

Synopsis:

```sh
legion_prof_viewer <archive-dir-or-URL> \
    [--duckdb <path.duckdb>]   # profile database (skip if auto-detected)
    [--code   <dir>]           # profiled application's source
    [--wiki   <dir>]           # override the built-in Legion knowledge wiki
```

Everything passed by flag can also be connected later from the panel's +
menu; connected paths persist across restarts (CLI flags win when both
exist).

### Converting an archive to a database

If you have an archive but no `prof_*.gz` logs:

```sh
$ cargo run --release --features duckdb --example prof2duckdb -- \
    myrun_archive -o myrun_db
```

### Knowledge wiki

The corpus lives in this repository under [`wiki/`](wiki/) and is embedded
into the binary at build time, so it works in every AI build with no
configuration. Pass `--wiki <dir>` to serve a different corpus from disk;
edits take effect without a rebuild — the corpus-development workflow.

### Headless MCP server

A stdio variant (data + wiki tools, no GUI) ships as the `mcp` bin:

```sh
$ cargo run --features ai,duckdb --bin mcp -- --duckdb <db> [--code-root <dir>]
```

## For reviewers & development

All AI code lives in `src/ai/`, `src/bin/`, and `src/app/core/legion_ai.rs`
(a child module holding every AI addition to the viewer core — upstream files
carry only thin `#[cfg(feature = "ai")]`-gated call sites).
[docs/UPSTREAM-DELTA.md](docs/UPSTREAM-DELTA.md) maps the full delta against
upstream and how to review it. A built-in direct-API engine exists in the
code but is currently disabled.

| Build | What you get |
|---|---|
| `cargo build` | the plain upstream viewer (no AI) |
| `--features ai` | chat panel + UI |
| `--features ai,duckdb` | + DuckDB data tools |
| `--features viewer-mcp` | + in-viewer MCP server + the Claude Code engine — the recommended build |
| `--features eval` | + the oracle-graded eval harness (maintainers) |

```sh
$ cargo check --features ai,duckdb
$ cargo clippy --features ai,duckdb -- -W clippy::all
$ cargo test  --features ai,duckdb
# claude_code.rs / viewer_mcp.rs compile ONLY under viewer-mcp:
$ cargo test  --features viewer-mcp
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

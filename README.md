# Legion Prof Viewer — AI Co-Pilot fork

> **This is a modified fork of
> [StanfordLegion/prof-viewer](https://github.com/StanfordLegion/prof-viewer)**
> (Apache-2.0) that adds an **AI diagnostic co-pilot** for Legion Runtime
> profiles: an embedded chat panel that queries the profile database, reads the
> profiled application's source, drives the live timeline (zoom, search,
> highlights), and produces root-cause performance diagnoses. All additions live
> in `src/ai/`, `src/bin/`, and feature-gated blocks of `src/app/core.rs` /
> `src/main.rs`; a default build (`cargo build`) behaves like upstream.
> Upstream's original README content follows the AI sections below.

## The AI Co-Pilot in one minute

```sh
# Build with the full AI layer (in-viewer MCP server + Claude Code engine)
cargo run --release --features viewer-mcp -- /path/to/profile_archive \
    --duckdb /path/to/profile.duckdb

# Open the panel with the "Legion AI Co-Pilot" button (top right) and ask:
#   "Where does the time go in this run?"
```

- **Profile database**: the data tools run SQL over a DuckDB export of the
  profile. If a `*_db`/`*.duckdb` file sits next to the profile you open, it is
  auto-detected; otherwise pass `--duckdb`. (`examples/prof2duckdb.rs` converts
  a profile archive.)
- **Context via the ＋ menu** (bottom-left of the composer): *Connect DuckDB…*,
  *Connect code repo…* (enables source reading — `--code <dir>` does the same at
  launch), *Add file…* (inline context). Connected items show as chips with ×.
- Connected paths persist across restarts. CLI flags win over persisted values.

## Engines (auto-detected — no configuration)

The panel picks its engine from what you have authenticated on your machine:

| You have | Engine used | Auth |
|---|---|---|
| `claude` CLI installed | **Your Claude Code**, spawned headless against the viewer's MCP server | one-time `claude login` *or* `ANTHROPIC_API_KEY` (inherited) |
| no `claude` | **Built-in API loop** | `ANTHROPIC_API_KEY` env var, or the key popup on first use |

The Claude Code engine is preferred when available: it brings the full agent
harness (its own file tools over your connected repo, bash/web behind an
approval dialog) on your existing subscription or key, with the model your
`claude` install is configured for. The built-in engine is the zero-install
fallback (plain HTTPS to the Anthropic API).

Requirements for the Claude Code engine: `claude` ≥ 2.1.x on PATH and `curl`
(used by the tool-approval bridge). Tool calls that touch your machine
(Bash/Edit/Write/Web) raise a **Deny / Allow / Always allow** dialog in the
viewer; see [SECURITY.md](SECURITY.md) for the full model.

## Using your own agent over MCP (BYOA)

The viewer runs a loopback-only HTTP MCP server (data + source + wiki + visual
timeline tools). At startup it prints a ready-to-paste registration:

```sh
claude mcp add --transport http legion-viewer \
    http://127.0.0.1:8765/mcp --header "Authorization: Bearer <token>"
```

The bearer token is random per session; set `LEGION_VIEWER_MCP_TOKEN` for a
stable registration across restarts. A headless stdio variant ships as the
`mcp` bin (`cargo run --features ai,duckdb --bin mcp -- --duckdb <db>
[--code-root <dir>]`).

## Optional knowledge wiki

The `wiki_*` tools serve a curated Legion-concepts wiki to the agent. Point
`--wiki <dir>` at a corpus (auto-detected at `wiki-legion/wiki` relative to the
launch directory). The corpus used during development is published separately —
see the release notes of this fork for the companion wiki repository.

## Feature flags

| Build | What you get |
|---|---|
| `cargo build` | the plain upstream viewer (no AI) |
| `--features ai` | chat panel + built-in API engine (no SQL tools) |
| `--features ai,duckdb` | + DuckDB data tools (`run_query`, overview, …) |
| `--features viewer-mcp` | + in-viewer MCP server + the Claude Code engine (implies `ai,duckdb`) |
| `--features eval` | + the oracle-graded eval harness (`eval` bin; maintainers) |

AI features are **native-only**; wasm builds get the plain viewer. Diagnostics
tracing is opt-in via `LEGION_PROF_AI_TRACE_DIR`.

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

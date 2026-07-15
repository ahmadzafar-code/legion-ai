# Contributing

This fork layers an AI diagnostic co-pilot onto
[StanfordLegion/prof-viewer](https://github.com/StanfordLegion/prof-viewer).
This document covers the AI layer — `src/ai/`, `src/bin/`,
`src/app/core/legion_ai.rs`, and the thin feature-gated seams in
`src/app/core.rs` / `src/main.rs`. Changes to the viewer itself are usually
better sent upstream; to review this fork against upstream, start with
[docs/UPSTREAM-DELTA.md](docs/UPSTREAM-DELTA.md).

## Checking the feature matrix

Every feature combination must compile on its own, because a `cfg` mistake in
one combination can hide behind `--all-features`. This is the one hard rule of
the repository:

```sh
$ cargo check                              # plain upstream viewer
$ cargo check --features ai
$ cargo check --features duckdb
$ cargo check --features ai,duckdb
$ cargo check --features viewer-mcp       # implies ai,duckdb; adds the MCP server
$ cargo check --features eval             # implies ai,duckdb; adds sha2 for the eval bin
```

These six checks, plus `cargo fmt`, clippy with `-D warnings`, the tests, the
wasm checks, and a `trunk` build, are what `./check.sh` runs — the build,
lint, and test portion of CI, locally. `cargo audit` and
`cargo deny check bans sources` run in CI only.

## Running the tests

```sh
$ cargo test --features viewer-mcp        # lib tests (the bulk)
$ cargo test --features eval              # + the eval bin's unit tests
```

Tests that need a profile database soft-skip when it is absent, so a fresh
clone runs green. To exercise them, place a DuckDB profile where the test
banner says it looks, or set `LEGION_EVAL_FIXTURES_DIR` for the eval cases;
fixture databases are not distributed because they sha-pin multi-GB profiles.

Two `#[ignore]` tests drive a real `claude` end-to-end:

```sh
$ cargo test --features viewer-mcp -- --ignored live_claude_code
```

> **Important:** These live tests require Claude Code installed and logged in,
> plus the bg4N2 fixture database, and they bill your account.

A third `#[ignore]` test in `tools/` is merely slow, not live.

The eval gate (`cargo run --features eval --bin eval -- run-all …`) is a local
pre-ship gate, deliberately excluded from CI: it hits a live model.

## Architecture

See [docs/architecture.md](docs/architecture.md) for the full map. The short
version: your spawned Claude Code, plus the eval harness's embedded API loop,
drives one shared tool layer. The load-bearing invariants are documented where
they live:

- the channel-lifetime contract (`chat_panel.rs`)
- the viewport single-driver token (`bridge.rs`)
- the approval bridge (`claude_code.rs`)
- the read-only anti-exfiltration SQL executor (`tools/query.rs`)

## Conventions

- Errors are `Result<_, String>`, and messages are written for the
  model/transcript; see `src/ai/mod.rs` for the rationale. Locks fail fast
  with `.lock().unwrap()`.
- Comments state invariants, not history: no process tags, no "replaces the
  old X". If a fact was verified empirically against a specific `claude`
  version, say exactly that.
- `cargo fmt --all` and `cargo clippy --all-features --all-targets --
  -D warnings` must both pass; CI enforces them.

# Contributing

This fork layers an AI diagnostic co-pilot on top of
[StanfordLegion/prof-viewer](https://github.com/StanfordLegion/prof-viewer).
Reviewing this fork against upstream? Start with
[docs/UPSTREAM-DELTA.md](docs/UPSTREAM-DELTA.md).
Changes to the *viewer* itself are usually better sent upstream; this document
covers the AI layer (`src/ai/`, `src/bin/`, `src/app/core/legion_ai.rs`, and
the thin feature-gated seams in `src/app/core.rs` / `src/main.rs`).

## The feature matrix (the one hard rule)

Every combination must compile on its own — a `cfg` mistake in one combo can
hide behind `--all-features`:

```sh
cargo check                              # plain upstream viewer
cargo check --features ai
cargo check --features duckdb
cargo check --features ai,duckdb
cargo check --features viewer-mcp       # implies ai,duckdb; adds the MCP server
cargo check --features eval             # implies ai,duckdb; adds sha2 for the eval bin
```

`./check.sh` runs the build/lint/test portion of CI locally (matrix + fmt +
clippy `-D warnings` + tests + wasm + trunk); `cargo audit` and
`cargo deny check bans sources` run in CI only.

## Tests

```sh
cargo test --features viewer-mcp        # lib tests (the bulk)
cargo test --features eval              # + the eval bin's unit tests
```

- Tests that need a profile database **soft-skip** when it is absent; a fresh
  clone runs green. To exercise them, place a DuckDB profile where the test
  banner says it looks, or set `LEGION_EVAL_FIXTURES_DIR` for the eval cases
  (fixture databases are not distributed — they sha-pin multi-GB profiles).
- Two `#[ignore]` live tests drive a real `claude` end-to-end
  (`cargo test --features viewer-mcp -- --ignored live_claude_code`); they need
  Claude Code installed and logged in, plus the bg4N2 fixture DB, and they bill
  your account. (A third `#[ignore]` in `tools/` is merely slow, not live.)
- The eval gate (`cargo run --features eval --bin eval -- run-all …`) is a
  **local** pre-ship gate, deliberately not CI: it hits a live model.

## Architecture

Read [docs/architecture.md](docs/architecture.md) first. The short version:
your spawned Claude Code (plus the eval harness's embedded API loop) drives
**one** shared tool layer; the load-bearing invariants are documented where
they live — the channel-lifetime contract (`chat_panel.rs`), the viewport
single-driver token (`bridge.rs`), the approval bridge (`claude_code.rs`), and
the read-only anti-exfiltration SQL executor (`tools/query.rs`).

## Conventions

- Errors are `Result<_, String>`; messages are written for the model/transcript
  (see `src/ai/mod.rs` for the rationale). Locks use fail-fast
  `.lock().unwrap()`.
- Comments state invariants, not history: no process tags, no
  "replaces the old X". If a fact was verified empirically against a specific
  `claude` version, say exactly that.
- `cargo fmt --all` and `cargo clippy --all-features --all-targets --
  -D warnings` must both pass (CI enforces them).

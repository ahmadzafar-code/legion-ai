//! Capture a build identity so a tester's bug report / session trace can name
//! the exact commit. `git describe --always --dirty` yields the short commit
//! (plus `-dirty` for uncommitted changes); on a shallow CI clone it still
//! returns the SHA, and with no `.git` (a source tarball, `cargo install` from
//! crates.io) the command fails and the code falls back to CARGO_PKG_VERSION.
use std::process::Command;

fn main() {
    let build = Command::new("git")
        .args(["describe", "--always", "--dirty", "--tags"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    if let Some(b) = build {
        println!("cargo:rustc-env=LEGION_AI_BUILD={b}");
    }
    // Rebuild the identity when HEAD moves (only if we're in a git checkout).
    if std::path::Path::new(".git/HEAD").exists() {
        println!("cargo:rerun-if-changed=.git/HEAD");
    }
}

#![warn(clippy::all, rust_2018_idioms)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

use legion_prof_viewer::deferred_data::DeferredDataSource;
#[cfg(not(target_arch = "wasm32"))]
use legion_prof_viewer::file_data::FileDataSource;
use legion_prof_viewer::http::client::HTTPClientDataSource;
#[cfg(not(target_arch = "wasm32"))]
use legion_prof_viewer::parallel_data::ParallelDeferredDataSource;

use url::Url;

fn http_ds(url: Url) -> Box<dyn DeferredDataSource> {
    Box::new(HTTPClientDataSource::new(url))
}

#[cfg(not(target_arch = "wasm32"))]
fn file_ds(path: impl AsRef<Path>) -> Box<dyn DeferredDataSource> {
    Box::new(ParallelDeferredDataSource::new(FileDataSource::new(path)))
}

/// Look for a DuckDB database next to an opened profile/archive path.
/// Convention: `<base>_archive` → `<base>_db`; otherwise scan the same
/// directory for any `*_db` or `*.duckdb` file.
#[cfg(all(not(target_arch = "wasm32"), feature = "ai"))]
fn detect_sibling_duckdb(path: &str) -> Option<String> {
    let p = Path::new(path);
    if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
        if let Some(base) = name.strip_suffix("_archive") {
            let cand = p.with_file_name(format!("{base}_db"));
            if cand.is_file() {
                return Some(cand.to_string_lossy().into_owned());
            }
        }
    }
    let dir = match p.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => std::path::PathBuf::from("."),
    };
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let ep = entry.path();
        if ep.is_file() {
            if let Some(n) = ep.file_name().and_then(|n| n.to_str()) {
                if n.ends_with(".duckdb") || n.ends_with("_db") {
                    return Some(ep.to_string_lossy().into_owned());
                }
            }
        }
    }
    None
}

/// Without the `ai` feature this is upstream's entry point, byte-for-byte
/// behavior: every argument is a URL or a file path (non-UTF-8 paths included).
#[cfg(all(not(target_arch = "wasm32"), not(feature = "ai")))]
fn main() {
    let ds: Vec<_> = std::env::args_os()
        .skip(1)
        .map(|arg| {
            arg.into_string()
                .map(|s| {
                    Url::parse(&s).map(http_ds).unwrap_or_else(|_| {
                        println!(
                            "The argument '{}' does not appear to be a valid URL. Attempting to open it as a local file...",
                            &s
                        );
                        file_ds(&s)
                    })
                })
                .unwrap_or_else(file_ds)
        })
        .collect();

    legion_prof_viewer::app::start(ds);
}

/// With the `ai` feature, Legion AI adds three flags on top of upstream's
/// URL/path arguments:
///   --duckdb <path.duckdb>   profile database for the data tools
///   --code <dir>             profiled application's source (read_code root)
///   --wiki <dir>             Legion knowledge wiki root
/// A missing --duckdb is auto-detected next to an opened profile; a missing
/// --wiki auto-detects `wiki-legion/wiki` relative to the launch directory.
#[cfg(all(not(target_arch = "wasm32"), feature = "ai"))]
fn main() {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();

    let mut ds: Vec<Box<dyn DeferredDataSource>> = Vec::new();
    let mut file_paths: Vec<String> = Vec::new();
    let mut duckdb_path: Option<String> = None;
    let mut code_path: Option<String> = None;
    let mut wiki_path: Option<String> = None;

    // A flag's value must exist and must not itself be a flag — catching the
    // classic `--duckdb --wiki` (empty shell variable) mistake at launch with a
    // clear error instead of a viewer misconfigured with the literal "--wiki".
    let flag_value = |args: &[std::ffi::OsString], i: usize| -> String {
        let flag = args[i].to_string_lossy();
        match args.get(i + 1).map(|v| v.to_string_lossy().into_owned()) {
            Some(v) if !v.starts_with("--") => v,
            _ => {
                eprintln!(
                    "error: {flag} requires a path value\n\
                     usage: legion_prof_viewer [<profile-path-or-URL>...] \
                     [--duckdb <path.duckdb>] [--code <dir>] [--wiki <dir>]"
                );
                std::process::exit(2);
            }
        }
    };

    let mut i = 0;
    while i < args.len() {
        match args[i].to_str() {
            Some("--duckdb") => {
                duckdb_path = Some(flag_value(&args, i));
                i += 2;
            }
            Some("--code") => {
                code_path = Some(flag_value(&args, i));
                i += 2;
            }
            Some("--wiki") => {
                wiki_path = Some(flag_value(&args, i));
                i += 2;
            }
            Some("--help") | Some("-h") => {
                println!(
                    "usage: legion_prof_viewer [<profile-path-or-URL>...] \
                     [--duckdb <path.duckdb>] [--code <dir>] [--wiki <dir>]"
                );
                return;
            }
            Some(s) => {
                match Url::parse(s) {
                    Ok(url) => ds.push(http_ds(url)),
                    Err(_) => {
                        println!("The argument '{s}' is not a URL. Opening it as a local file...");
                        ds.push(file_ds(s));
                        file_paths.push(s.to_owned());
                    }
                }
                i += 1;
            }
            // Non-UTF-8 argument: same as upstream — open it as a file path.
            None => {
                ds.push(file_ds(&args[i]));
                i += 1;
            }
        }
    }

    if ds.is_empty() {
        println!(
            "No profile opened — pass a profile archive path or URL \
             (the viewer starts empty otherwise)."
        );
    }

    // Auto-detect a sibling DuckDB next to the opened profile when not given.
    if duckdb_path.is_none() {
        duckdb_path = file_paths.iter().find_map(|p| detect_sibling_duckdb(p));
    }
    if let Some(ref db) = duckdb_path {
        println!("Legion AI DuckDB: {db}");
    }

    // The code root is explicit-only: set solely by the `--code` flag (no
    // autodetect, no cwd default) — guessing an application source tree wrong
    // is worse than leaving read_code off until the user connects one.
    if let Some(ref code) = code_path {
        println!("Legion AI code root: {code}");
    }

    // Auto-detect the Legion wiki at `wiki-legion/wiki` (relative to the launch
    // dir) when no --wiki was given, so the knowledge tools work out of the box.
    if wiki_path.is_none() {
        let cand = Path::new("wiki-legion").join("wiki");
        if cand.is_dir() {
            wiki_path = Some(cand.to_string_lossy().into_owned());
        }
    }
    if let Some(ref wiki) = wiki_path {
        println!("Legion AI wiki root: {wiki}");
    }

    legion_prof_viewer::app::start_with_options(
        ds,
        legion_prof_viewer::app::StartOptions {
            ai_duckdb_path: duckdb_path,
            ai_code_path: code_path,
            ai_wiki_path: wiki_path,
        },
    );
}

#[cfg(target_arch = "wasm32")]
fn main() {
    let loc: web_sys::Location = web_sys::window().unwrap().location();
    let href: String = loc.href().expect("unable to get window URL");
    let browser_url = Url::parse(&href).expect("unable to parse location URL");

    let ds: Vec<_> = browser_url
        .query_pairs()
        .filter(|(key, _)| key.starts_with("url"))
        .map(|(_, value)| http_ds(Url::parse(&value).expect("unable to parse query URL")))
        .collect();

    legion_prof_viewer::app::start(ds);
}

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
#[cfg(not(target_arch = "wasm32"))]
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

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .filter_map(|a| a.into_string().ok())
        .collect();

    let mut ds: Vec<Box<dyn DeferredDataSource>> = Vec::new();
    let mut file_paths: Vec<String> = Vec::new();
    let mut duckdb_path: Option<String> = None;
    let mut code_path: Option<String> = None;
    let mut wiki_path: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--duckdb" => {
                duckdb_path = args.get(i + 1).cloned();
                i += 2;
            }
            "--code" => {
                code_path = args.get(i + 1).cloned();
                i += 2;
            }
            "--wiki" => {
                wiki_path = args.get(i + 1).cloned();
                i += 2;
            }
            s => {
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
        }
    }

    // Auto-detect a sibling DuckDB next to the opened profile when not given.
    if duckdb_path.is_none() {
        duckdb_path = file_paths.iter().find_map(|p| detect_sibling_duckdb(p));
    }
    if let Some(ref db) = duckdb_path {
        println!("Legion AI Co-Pilot DuckDB: {db}");
    }

    // When no --code was given, prefer a `code_examples/` directory relative to the
    // launch dir (the app source); fall back to the launch cwd so read_code/list_files
    // still work. Explicit --code always overrides (code_path is already Some here).
    if code_path.is_none() {
        let cand = Path::new("code_examples");
        if cand.is_dir() {
            let p = cand.to_string_lossy().into_owned();
            println!("Legion AI Co-Pilot source root: {p}");
            code_path = Some(p);
        } else if let Ok(cwd) = std::env::current_dir() {
            code_path = Some(cwd.to_string_lossy().into_owned());
        }
    }
    if let Some(ref code) = code_path {
        println!("Legion AI Co-Pilot code root: {code}");
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
        println!("Legion AI Co-Pilot wiki root: {wiki}");
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

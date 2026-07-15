//! File / source-tree tools (path-sandboxed `read_code`/`list_files`).

use std::path::Path;

/// Source extensions included in file listings and tree views.
const SOURCE_EXTS: &[&str] = &[
    "cc", "cpp", "c", "h", "hpp", "cu", "cuh", "py", "rs", "rg", "mk", "cmake", "toml", "json",
    "yaml", "yml", "txt", "md",
];

/// Directories to skip when walking the source tree.
pub(crate) const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "build",
    "__pycache__",
    ".cache",
    ".vscode",
    ".idea",
];

/// Format a byte count as a human-readable size string.
fn format_size(bytes: u64) -> String {
    if bytes >= 1_048_576 {
        format!("{:.1}MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1}KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes}B")
    }
}

/// Recursively walk a directory tree, appending an indented listing to `output`.
///
/// Caps at `max_depth` levels and `max_files` total entries to prevent
/// runaway scanning on large repositories.
fn walk_dir_tree(
    dir: &Path,
    prefix: &str,
    depth: usize,
    max_depth: usize,
    output: &mut String,
    file_count: &mut usize,
    max_files: usize,
) {
    if depth > max_depth || *file_count >= max_files {
        return;
    }

    let mut entries: Vec<std::fs::DirEntry> = match std::fs::read_dir(dir) {
        Ok(rd) => rd.flatten().collect(),
        Err(_) => return,
    };

    // Sort: directories first, then files, both alphabetical
    entries.sort_by(|a, b| {
        let a_dir = a.path().is_dir();
        let b_dir = b.path().is_dir();
        match (a_dir, b_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.file_name().cmp(&b.file_name()),
        }
    });

    let indent = "  ".repeat(depth);

    for entry in entries {
        if *file_count >= max_files {
            output.push_str(&format!(
                "{indent}  ... (truncated at {max_files} entries)\n"
            ));
            return;
        }

        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        if path.is_dir() {
            if SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            output.push_str(&format!("{indent}{prefix}{name}/\n"));
            walk_dir_tree(
                &path,
                "",
                depth + 1,
                max_depth,
                output,
                file_count,
                max_files,
            );
        } else {
            // Only list files with recognised source extensions
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !SOURCE_EXTS.contains(&ext) {
                continue;
            }
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            output.push_str(&format!("{indent}{prefix}{name} ({})\n", format_size(size)));
            *file_count += 1;
        }
    }
}

/// Build a recursive file tree listing for the given code root directory.
///
/// Returns a formatted string showing directories and source files with sizes,
/// capped at 6 levels deep and 500 files. Backs the `list_files` tool and the
/// self-correction hint appended to `read_code` file-not-found errors.
fn recursive_file_tree(code_root: &str) -> Result<String, String> {
    if code_root.is_empty() {
        return Err(
            "Code path not configured. Connect a code repo via the + menu, or \
                    launch with --code <dir>."
                .into(),
        );
    }

    let root = Path::new(code_root);
    if !root.is_dir() {
        return Err(format!("'{code_root}' is not a directory."));
    }

    let mut output = format!("Files in `{code_root}`:\n");
    let mut file_count = 0usize;
    walk_dir_tree(root, "", 0, 6, &mut output, &mut file_count, 500);

    if file_count == 0 {
        output.push_str("  (no source files found)\n");
    }

    Ok(output)
}

/// Execute the `list_files` tool: list source files in a subdirectory of the code root.
///
/// If `path` is empty or `"."`, lists from the code root itself.
pub fn execute_list_files(code_root: &str, path: &str) -> Result<String, String> {
    if code_root.is_empty() {
        return Err(
            "Code path not configured. Connect a code repo via the + menu, or \
                    launch with --code <dir>."
                .into(),
        );
    }

    let target = if path.is_empty() || path == "." {
        code_root.to_owned()
    } else {
        if path.contains("..") || path.starts_with('/') || path.starts_with('\\') {
            return Err("Invalid path: must be relative with no '..' or absolute prefix.".into());
        }
        Path::new(code_root)
            .join(path)
            .to_string_lossy()
            .to_string()
    };

    recursive_file_tree(&target)
}

/// Read a source file from the code root directory.
///
/// The path must be relative and within `code_root` — path traversal (`..`) is rejected.
pub fn execute_read_code(code_root: &str, path: &str) -> Result<String, String> {
    if code_root.is_empty() {
        return Err(
            "Code path not configured. Connect a code repo via the + menu, or launch with --code <dir>."
                .into(),
        );
    }

    if path.contains("..") || path.starts_with('/') || path.starts_with('\\') {
        return Err("Invalid path: must be relative with no '..' or absolute prefix.".into());
    }

    let full_path = Path::new(code_root).join(path);
    // Reject symlink/canonicalization escapes. The `..` string check above does
    // NOT stop a symlink INSIDE the root pointing at, e.g., ~/.ssh/id_rsa; the resolved
    // target must stay under the resolved root. Only enforced when the target exists —
    // a not-found path falls through to the helpful "Available files" tree below.
    if let Ok(canon_target) = full_path.canonicalize() {
        if let Ok(canon_root) = Path::new(code_root).canonicalize() {
            if !canon_target.starts_with(&canon_root) {
                return Err(format!(
                    "Invalid path: '{path}' resolves outside the configured code root."
                ));
            }
        }
    }
    std::fs::read_to_string(&full_path).map_err(|e| {
        let mut msg = format!("Cannot read '{}': {}", full_path.display(), e);
        // On file-not-found, show the recursive file tree so the agent can self-correct.
        if e.kind() == std::io::ErrorKind::NotFound {
            if let Ok(tree) = recursive_file_tree(code_root) {
                msg.push_str("\n\nAvailable files:\n");
                msg.push_str(&tree);
            }
        }
        msg
    })
}

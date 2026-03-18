//! Diagnostic knowledge and case record loader.
//!
//! Loads knowledge files (Legion domain model, profiler signal reference, etc.)
//! and diagnostic case records from disk, producing a concatenated string for
//! system prompt injection.

use std::fs;
use std::path::Path;

/// Holds the concatenated system context string built from knowledge files
/// and diagnostic case records.
pub struct RecordStore {
    record_count: usize,
    system_context: String,
}

impl RecordStore {
    /// Load knowledge files and diagnostic case records from disk.
    ///
    /// `records_dir` — directory containing `*.md` case record files
    /// `knowledge_dir` — directory containing knowledge files (legionconcepts.md, etc.)
    pub fn load(records_dir: &Path, knowledge_dir: &Path) -> Result<Self, String> {
        let mut context = String::new();

        // Load knowledge files (each optional — warn and skip if missing)
        let knowledge_files: &[(&str, &str)] = &[
            ("legionconcepts.md", "## Legion Runtime Model"),
            ("profiler-signal-reference.md", "## Profiler Signal Reference"),
            ("profilerdata-guide.md", "## Profiler Data Guide"),
            (
                "gold-standard-diagnostic-traces-v2.md",
                "## Expert Diagnostic Examples",
            ),
        ];

        for &(filename, heading) in knowledge_files {
            let path = knowledge_dir.join(filename);
            if let Some(section) = load_knowledge_file(&path, heading) {
                context.push_str(&section);
            }
        }

        // Load case records
        if !records_dir.is_dir() {
            return Err(format!(
                "Records directory does not exist: {}",
                records_dir.display()
            ));
        }

        let mut records: Vec<(String, String)> = Vec::new(); // (id, full_text)

        let entries = fs::read_dir(records_dir)
            .map_err(|e| format!("Failed to read records directory: {e}"))?;

        for entry in entries {
            let entry = entry.map_err(|e| format!("Failed to read directory entry: {e}"))?;
            let path = entry.path();

            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }

            let text =
                fs::read_to_string(&path).map_err(|e| format!("Failed to read {}: {e}", path.display()))?;

            // Extract id from first line: "id: <name>"
            let id = text
                .lines()
                .next()
                .and_then(|line| line.strip_prefix("id: "))
                .unwrap_or_else(|| {
                    eprintln!(
                        "Warning: record file has no 'id:' first line: {}",
                        path.display()
                    );
                    "unknown"
                })
                .to_string();

            records.push((id, text));
        }

        // Sort by id for deterministic ordering
        records.sort_by(|a, b| a.0.cmp(&b.0));

        let record_count = records.len();

        // Build case library section
        if record_count > 0 {
            context.push_str("## Diagnostic Case Library\n\n");
            context.push_str(
                "The following cases describe known Legion performance patterns. Each includes\n\
                 symptoms (what_you_see, key_metrics, distinguishing_features), root_cause,\n\
                 gotchas, and fix. Match your findings from the profiler data against these\n\
                 patterns. Pay attention to distinguishing_features — they tell you how to\n\
                 differentiate confusable patterns.\n\n",
            );

            let texts: Vec<&str> = records.iter().map(|(_, text)| text.as_str()).collect();
            context.push_str(&texts.join("\n---\n\n"));
        }

        Ok(Self {
            record_count,
            system_context: context,
        })
    }

    /// Create an empty store (no knowledge, no records).
    pub fn empty() -> Self {
        Self {
            record_count: 0,
            system_context: String::new(),
        }
    }

    /// The full system context string for injection into the system prompt.
    pub fn system_context(&self) -> &str {
        &self.system_context
    }

    /// Number of diagnostic case records loaded.
    pub fn record_count(&self) -> usize {
        self.record_count
    }
}

/// Load a single knowledge file and wrap it with a markdown heading.
/// Returns `None` with a warning if the file doesn't exist or can't be read.
fn load_knowledge_file(path: &Path, heading: &str) -> Option<String> {
    match fs::read_to_string(path) {
        Ok(contents) => Some(format!("{heading}\n\n{contents}\n\n")),
        Err(_) => {
            eprintln!(
                "Warning: knowledge file not found, skipping: {}",
                path.display()
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_empty_store() {
        let store = RecordStore::empty();
        assert_eq!(store.record_count(), 0);
        assert!(store.system_context().is_empty());
    }

    #[test]
    fn test_load_nonexistent_dir() {
        let result = RecordStore::load(Path::new("/nonexistent/path"), Path::new("/tmp"));
        assert!(result.is_err());
    }

    #[test]
    fn test_load_empty_dir() {
        let dir = std::env::temp_dir().join("legion_test_empty_records");
        let _ = fs::create_dir_all(&dir);
        let result = RecordStore::load(&dir, Path::new("/nonexistent/knowledge"));
        let _ = fs::remove_dir_all(&dir);

        assert!(result.is_ok());
        let store = result.unwrap();
        assert_eq!(store.record_count(), 0);
        // No records, but no crash
    }

    #[test]
    fn test_load_with_records() {
        let dir = std::env::temp_dir().join("legion_test_records");
        let _ = fs::create_dir_all(&dir);

        // Write two test record files
        fs::write(
            dir.join("alpha.md"),
            "id: alpha_pattern\ntitle: Alpha\nroot_cause: |\n  Something\n",
        )
        .unwrap();
        fs::write(
            dir.join("beta.md"),
            "id: beta_pattern\ntitle: Beta\nroot_cause: |\n  Other\n",
        )
        .unwrap();

        let result = RecordStore::load(&dir, Path::new("/nonexistent/knowledge"));
        let _ = fs::remove_dir_all(&dir);

        let store = result.unwrap();
        assert_eq!(store.record_count(), 2);
        // Sorted: alpha before beta
        assert!(store.system_context().contains("Diagnostic Case Library"));
        assert!(store.system_context().contains("id: alpha_pattern"));
        assert!(store.system_context().contains("id: beta_pattern"));
        let alpha_pos = store.system_context().find("alpha_pattern").unwrap();
        let beta_pos = store.system_context().find("beta_pattern").unwrap();
        assert!(alpha_pos < beta_pos, "Records should be sorted by id");
    }

}

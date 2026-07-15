//! The advertised tool definitions (JSON schemas + when-to-use descriptions).
//!
//! Consumed by BOTH the built-in loop's `tools` array and `mcp_core`'s
//! `tools/list` — one source of truth for the advertised surface. The
//! descriptions are the #1 behavioral lever: they must carry the WHEN/WHEN-NOT
//! triggers themselves, because MCP `instructions` MAY be ignored by clients.
//! Every SQL example must use the real schema: `entry_slug` (not proc_id),
//! STRUCT dot notation (`running.duration`), the `items` table.

/// Return Claude API tool definitions for the agent.
///
/// - `has_duckdb`: include `run_query` tool (only if duckdb feature AND path is set)
/// - `has_code`: include `read_code` tool (only if code path is configured)
/// - `has_wiki`: include `wiki_index`/`wiki_read`/`wiki_search` (only if a wiki root is configured)
pub fn tool_definitions(
    has_duckdb: bool,
    has_code: bool,
    has_wiki: bool,
) -> Vec<serde_json::Value> {
    let mut tools = Vec::new();

    if has_duckdb {
        tools.push(serde_json::json!({
            "name": "run_query",
            "description":
                "Execute a read-only SQL query against the Legion profiling DuckDB database. \
                 Returns up to 50 rows as JSON. Do NOT include a trailing semicolon.\n\n\
                 RANGE QUERIES — READ FIRST: when the question says \"in the range\", \
                 \"in the highlighted region\", \"in the selection\", or \"longest / most time \
                 within a window\", you MUST CLIP each item to the window with \
                 SUM(LEAST(running.stop, {hi_ns}) - GREATEST(running.start, {lo_ns})) (see example #10) — \
                 do NOT use the plain time-range FILTER (example #2), and do NOT compare full \
                 durations: an item mostly OUTSIDE the window must not win.\n\n\
                 TRUNCATION: results are capped at 50 rows. If MORE rows matched, the JSON array \
                 holds the first 50 plus a final marker object {\"_truncated\": true, \"_shown\": 50, ...} — \
                 when you see it, the result is INCOMPLETE; refine with aggregation (GROUP BY/COUNT/SUM) \
                 or a narrower filter rather than trusting the 50 shown.\n\n\
                 SCHEMA REMINDER: Two tables — `entries` (entry_slug, short_name, long_name, parent_slug, type) \
                 and `items` (entry_slug, item_uid, title, plus STRUCT columns). \
                 All STRUCT columns use dot notation: running.start, running.duration, critical_path.item_uid, etc.\n\n\
                 EXAMPLE QUERIES:\n\
                 1. Per-processor utilization:\n\
                    SELECT entry_slug, COUNT(*) AS task_count,\n\
                      ROUND(SUM(running.duration) / 1e6, 1) AS busy_ms\n\
                    FROM items WHERE running IS NOT NULL\n\
                    GROUP BY entry_slug ORDER BY busy_ms DESC\n\n\
                 2. Tasks in a time range on a specific processor:\n\
                    SELECT item_uid, title, running.start, running.stop,\n\
                      running.duration / 1e6 AS run_ms\n\
                    FROM items\n\
                    WHERE entry_slug = 'n0_gpu_g0' AND running.start < 500000000\n\
                      AND running.stop > 400000000\n\
                    ORDER BY running.start\n\n\
                 3. GPU idle gaps (find gaps between consecutive tasks):\n\
                    WITH ordered AS (\n\
                      SELECT running.stop AS task_end,\n\
                        LEAD(running.start) OVER (PARTITION BY entry_slug ORDER BY running.start) AS next_start\n\
                      FROM items\n\
                      WHERE entry_slug LIKE '%gpu%' AND running IS NOT NULL\n\
                    )\n\
                    SELECT (next_start - task_end) / 1e6 AS gap_ms,\n\
                      task_end AS gap_start_ns, next_start AS gap_end_ns\n\
                    FROM ordered WHERE next_start > task_end\n\
                    ORDER BY gap_ms DESC LIMIT 10\n\n\
                 4. Walk critical path from a task:\n\
                    WITH RECURSIVE chain AS (\n\
                      SELECT item_uid, title, entry_slug,\n\
                        running.duration / 1e6 AS run_ms,\n\
                        critical_path.item_uid AS cp_uid, 1 AS depth\n\
                      FROM items WHERE item_uid = <START_UID>\n\
                      UNION ALL\n\
                      SELECT i.item_uid, i.title, i.entry_slug,\n\
                        i.running.duration / 1e6, i.critical_path.item_uid, c.depth + 1\n\
                      FROM items i JOIN chain c ON i.item_uid = c.cp_uid\n\
                      WHERE c.cp_uid IS NOT NULL AND c.depth < 10\n\
                    )\n\
                    SELECT * FROM chain ORDER BY depth\n\n\
                 5. Identify processor kinds by entry_slug pattern:\n\
                    SELECT entry_slug FROM entries\n\
                    WHERE type = 'slot' AND entry_slug LIKE '%gpu%'\n\
                    ORDER BY entry_slug\n\n\
                 6. Task lifecycle breakdown:\n\
                    SELECT title,\n\
                      ROUND(AVG(waiting.duration) / 1e6, 2) AS avg_wait_ms,\n\
                      ROUND(AVG(deferred.duration) / 1e6, 2) AS avg_defer_ms,\n\
                      ROUND(AVG(running.duration) / 1e6, 2) AS avg_run_ms\n\
                    FROM items WHERE running IS NOT NULL\n\
                    GROUP BY title ORDER BY avg_wait_ms DESC LIMIT 10\n\n\
                 7. Channel copy analysis (copies use lifetime + size, NEVER running; dedup by item_uid).\n\
                    For copy VOLUME, sum `size` on %chan% rows ONLY — instances/fills on other rows also\n\
                    carry `size` and must not be mixed into channel totals; `size` is a unit-suffixed\n\
                    string ('76.000 KiB', '96 B') so parse units before summing:\n\
                    SELECT entry_slug, COUNT(*) AS copy_count,\n\
                      ROUND(SUM(ld) / 1e6, 1) AS total_ms\n\
                    FROM (SELECT item_uid, min(entry_slug) AS entry_slug,\n\
                            any_value(lifetime.duration) AS ld\n\
                          FROM (SELECT DISTINCT item_uid, entry_slug, lifetime\n\
                                FROM items WHERE entry_slug LIKE '%chan%') s\n\
                          GROUP BY item_uid)\n\
                    GROUP BY entry_slug ORDER BY total_ms DESC\n\
                    (size is a unit-suffixed string like '76.000 KiB'/'96 B' \u{2014} parse units for bytes)\n\n\
                 8. Utility activity during a time window:\n\
                    SELECT title, COUNT(*) AS cnt,\n\
                      ROUND(SUM(running.duration) / 1e6, 1) AS total_ms\n\
                    FROM items\n\
                    WHERE entry_slug LIKE '%util%' AND running IS NOT NULL\n\
                      AND running.start < {end_ns} AND running.stop > {start_ns}\n\
                    GROUP BY title ORDER BY total_ms DESC\n\n\
                 9. Tasks with thin pipeline (deferred near zero):\n\
                    SELECT entry_slug, title, item_uid,\n\
                      ROUND(deferred.duration / 1e6, 3) AS deferred_ms,\n\
                      ROUND(running.duration / 1e6, 2) AS run_ms\n\
                    FROM items\n\
                    WHERE deferred IS NOT NULL AND deferred.duration < 100000\n\
                      AND running IS NOT NULL\n\
                    ORDER BY running.start\n\n\
                 10. Most running time WITHIN a range (CLIP to the range — for \"longest / most\n\
                     time in the selected range\"; count only the portion inside [{lo_ns}, {hi_ns}]):\n\
                    SELECT item_uid,\n\
                      ROUND(SUM(LEAST(e, {hi_ns}) - GREATEST(s, {lo_ns})) / 1e6, 2) AS ms_in_range\n\
                    FROM (SELECT DISTINCT item_uid, entry_slug,\n\
                            running.start AS s, running.stop AS e\n\
                          FROM items WHERE running.start IS NOT NULL\n\
                            AND (entry_slug LIKE '%_cpu_%' OR entry_slug LIKE '%_gpudev_%')\n\
                            AND running.start < {hi_ns} AND running.stop > {lo_ns})\n\
                    GROUP BY item_uid ORDER BY ms_in_range DESC, item_uid ASC\n\
                    (CLIP each task's running to the range with LEAST/GREATEST; do NOT compare\n\
                     full task durations — a task mostly outside the range should not win)\n\n\
                 IMPORTANT: You can call this tool multiple times per response to batch independent queries. \
                 Do NOT include LIMIT in your query — a hard cap of 50 rows is applied automatically. \
                 Before writing a query, check the overview's Schema section for exact column names. \
                 If a column shows 'Not available in this profile' in the overview, do NOT attempt to query it.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "sql": {
                        "type": "string",
                        "description": "The SELECT query to execute. No semicolon needed."
                    },
                    "purpose": {
                        "type": "string",
                        "description": "Brief description of why you are running this query."
                    }
                },
                "required": ["sql", "purpose"]
            }
        }));

        tools.push(serde_json::json!({
            "name": "overview",
            "description":
                "Return a precomputed structured overview of the profiling database: \
                 schema, row counts, processor hierarchy, per-kind utilization, timeline \
                 bounds, top task types, and other orientation signals. Takes no arguments. \
                 Call this once at the start when you need to get oriented before writing queries.",
            "input_schema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }));
    }

    if has_code {
        tools.push(serde_json::json!({
            "name": "list_files",
            "description":
                "List source files in the code directory tree. Shows files recursively with \
                 sizes, organized by directory. Use BEFORE read_code to discover what files \
                 exist. Pass a subdirectory path to narrow the listing.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Subdirectory to list (relative to code root). Empty or '.' for the root."
                    }
                },
                "required": []
            }
        }));

        tools.push(serde_json::json!({
            "name": "read_code",
            "description":
                "Read an application source file (path relative to the configured code root). \
                 Use to understand task logic, mapper policies, and application structure. \
                 Use list_files first to discover available files.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative file path, e.g. 'circuit.cc' or 'src/main.cc'."
                    }
                },
                "required": ["path"]
            }
        }));
    }

    if has_wiki {
        tools.push(serde_json::json!({
            "name": "wiki_index",
            "description":
                "Browse the structured Legion knowledge wiki: a grouped, one-line-per-page \
                 listing (path, tier, summary) of every page. CONSUMPTION LOOP: call wiki_index \
                 (or wiki_search) to find the right page → wiki_read it → follow its `Related` \
                 links to neighbours. Consult the wiki for Legion concepts, pitfalls, and \
                 diagnostic workflows instead of guessing. Pass `section` to list just one \
                 section (concepts, pitfalls, workflows, glossary, meta, applications). \
                 USE THIS WHEN the question asks for a performance CLASSIFICATION \
                 (compute-/communication-/runtime-bound), a LIFECYCLE phase meaning (waiting vs \
                 deferred vs ready), a flag/concept/pitfall definition, or a ROOT-CAUSE verdict — \
                 consult the wiki BEFORE asserting any such claim from prior knowledge. DON'T use \
                 it for raw per-task numbers (use run_query) or to re-read a page you already read.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "section": {
                        "type": "string",
                        "description": "Optional top-level section to scope the listing (e.g. 'pitfalls')."
                    }
                },
                "required": []
            }
        }));

        tools.push(serde_json::json!({
            "name": "wiki_read",
            "description":
                "Read one wiki page (path as shown by wiki_index/wiki_search, e.g. \
                 'concepts/mapper.md'). Returns the page verbatim with its `Related` links \
                 intact — follow those to neighbouring pages. Pass `section` to return ONLY one \
                 `## Header` block (e.g. 'Debug signals', 'Failure modes', 'TL;DR'). Long pages \
                 are capped at max_chars (default 12000) with a truncation marker; raise \
                 max_chars or read a specific section to see more. \
                 USE THIS WHEN the question asks for a performance CLASSIFICATION \
                 (compute-/communication-/runtime-bound), a LIFECYCLE phase meaning (waiting vs \
                 deferred vs ready), a flag/concept/pitfall definition, or a ROOT-CAUSE verdict — \
                 consult the wiki BEFORE asserting any such claim from prior knowledge. DON'T use \
                 it for raw per-task numbers (use run_query) or to re-read a page you already read.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative page path under the wiki root, e.g. 'concepts/mapper.md'."
                    },
                    "section": {
                        "type": "string",
                        "description": "Optional `## Header` to return just that block (e.g. 'Debug signals')."
                    },
                    "max_chars": {
                        "type": "integer",
                        "description": "Optional character cap (default 12000)."
                    }
                },
                "required": ["path"]
            }
        }));

        tools.push(serde_json::json!({
            "name": "wiki_search",
            "description":
                "Search the wiki by keyword/substring over page titles, summaries, TL;DRs, and \
                 tags. Returns up to `limit` ranked {path, section, tldr, tags, score} matches — \
                 these are PAGES TO READ, not a synthesized answer; wiki_read the top hits. Use \
                 this when you don't know which page covers a topic. Optional `section` and `tag` \
                 narrow the search. \
                 USE THIS WHEN the question asks for a performance CLASSIFICATION \
                 (compute-/communication-/runtime-bound), a LIFECYCLE phase meaning (waiting vs \
                 deferred vs ready), a flag/concept/pitfall definition, or a ROOT-CAUSE verdict — \
                 consult the wiki BEFORE asserting any such claim from prior knowledge. DON'T use \
                 it for raw per-task numbers (use run_query) or to re-read a page you already read. \
                 If the question contains words like 'bound', 'stall', 'overhead', \
                 'waiting/deferred/ready/lifecycle', 'mapper', or 'critical path' — wiki_search \
                 that term and wiki_read the top hits before answering.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Keyword(s) to match (case-insensitive substring)."
                    },
                    "section": {
                        "type": "string",
                        "description": "Optional top-level section to scope the search."
                    },
                    "tag": {
                        "type": "string",
                        "description": "Optional tag to require (exact, case-insensitive)."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Max results to return (default 5)."
                    }
                },
                "required": ["query"]
            }
        }));
    }

    tools.push(serde_json::json!({
        "name": "screenshot",
        "description":
            "Capture the current profiler timeline view as a PNG image. \
             Returns an image along with metadata showing the visible time \
             range (in nanoseconds) and the list of entry_slugs for each \
             processor row. Use this to visually inspect the timeline layout, \
             verify idle gaps, and understand spatial patterns across processors. \
             Do NOT take another screenshot if you already have a recent screenshot \
             of the same region — use the data you have. Prefer set_view over \
             screenshot when you need to change both zoom and scroll position.",
        "input_schema": {
            "type": "object",
            "properties": {},
            "required": []
        }
    }));

    tools.push(serde_json::json!({
        "name": "zoom_to",
        "description":
            "Zoom the profiler timeline to a specific nanosecond range and capture \
             a screenshot. Returns a screenshot with metadata showing the exact \
             visible range and entry_slugs. Use after identifying a region of \
             interest via queries to see fine-grained task scheduling, verify \
             gaps, and inspect processor utilization within the zoomed range. \
             Prefer set_view over zoom_to + scroll_to when you need both zoom and \
             vertical navigation — it's one round-trip instead of two.",
        "input_schema": {
            "type": "object",
            "properties": {
                "start_ns": {
                    "type": "integer",
                    "description": "Start of the zoom range in nanoseconds"
                },
                "stop_ns": {
                    "type": "integer",
                    "description": "End of the zoom range in nanoseconds"
                }
            },
            "required": ["start_ns", "stop_ns"]
        }
    }));

    tools.push(serde_json::json!({
        "name": "pan",
        "description":
            "Pan the timeline left or right by a percentage of the visible range. \
             Returns a screenshot with metadata after panning. Use to explore \
             adjacent time regions without changing zoom level — e.g. to see \
             what comes after a gap or to scan across the timeline incrementally.",
        "input_schema": {
            "type": "object",
            "properties": {
                "direction": {
                    "type": "string",
                    "enum": ["left", "right"],
                    "description": "Direction to pan: \"left\" moves earlier in time, \"right\" moves later."
                },
                "percent": {
                    "type": "number",
                    "description": "Percentage of the visible range to pan by (default 25). E.g. 50 pans half a screen width."
                }
            },
            "required": ["direction"]
        }
    }));

    tools.push(serde_json::json!({
        "name": "scroll_to",
        "description":
            "Scroll the timeline vertically to bring a specific processor row \
             into view. Identifies the processor by entry_slug (e.g. \"n0_gpu_g0\", \
             \"n0_util_u0\"). Auto-expands the processor's parent panel if collapsed. \
             Returns a screenshot with metadata. Use to navigate to a processor of \
             interest — e.g. scroll to utility rows or channel rows.",
        "input_schema": {
            "type": "object",
            "properties": {
                "entry_slug": {
                    "type": "string",
                    "description": "The entry_slug of the processor to scroll to (e.g. \"n0_gpu_g0\")."
                }
            },
            "required": ["entry_slug"]
        }
    }));

    tools.push(serde_json::json!({
        "name": "set_view",
        "description":
            "Combined view control in one call: zoom to a nanosecond range and \
             optionally scroll to a processor row, restrict the view to specific \
             processor kinds, expand/collapse kinds, and change row height. More \
             efficient than separate calls. Returns a screenshot with metadata. \
             Kind tokens come from the entry slugs / overview — typically \
             \"gpudev\", \"gpuhost\", \"cpu\", \"utility\", \"io\", \"system\", \
             \"framebuffer\", \"chan\", \"dp\". Matching is case-insensitive and \
             by substring, so \"gpu\" selects both \"gpudev\" and \"gpuhost\".",
        "input_schema": {
            "type": "object",
            "properties": {
                "start_ns": {
                    "type": "integer",
                    "description": "Start of the zoom range in nanoseconds"
                },
                "stop_ns": {
                    "type": "integer",
                    "description": "End of the zoom range in nanoseconds"
                },
                "entry_slug": {
                    "type": "string",
                    "description": "Optional entry_slug to scroll to after zooming."
                },
                "filter_kinds": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional: show ONLY these processor kinds (others are hidden). Omit or pass [] to show all kinds."
                },
                "expand_kinds": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional: expand these processor kinds to show their rows."
                },
                "collapse_kinds": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional: collapse these processor kinds to hide their rows."
                },
                "vertical_scale": {
                    "type": "number",
                    "description": "Optional row-height multiplier in [0.25, 4.0]. >1 makes crowded rows taller/readable."
                }
            },
            "required": ["start_ns", "stop_ns"]
        }
    }));

    tools.push(serde_json::json!({
        "name": "search",
        "description":
            "Set the timeline's search box to a string. Every task whose title \
             matches is highlighted in place across the visible rows, and the \
             returned screenshot metadata reports how many matched. Use this to \
             LOCATE tasks visually by name; use run_query when you need an exact \
             list or per-task numbers. Searching clears when you call reset_view.",
        "input_schema": {
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Substring to search task titles for, e.g. \"calculate_new_currents\"."
                }
            },
            "required": ["query"]
        }
    }));

    tools.push(serde_json::json!({
        "name": "reset_view",
        "description":
            "Reset the view to a clean slate: zoom out to the whole profile, clear \
             any kind filter, clear the search, and reset row height to default. \
             Use this before answering about the overall structure of the profile, \
             or to undo a previous set_view/search.",
        "input_schema": {
            "type": "object",
            "properties": {},
            "required": []
        }
    }));

    tools.push(serde_json::json!({
        "name": "highlight",
        "description":
            "Mark a region of the timeline to point the user at it (e.g. the task \
             you identified as the blocker). Highlights appear as clickable chips in \
             the chat and as overlays on the timeline. Use an entry_slug + the \
             nanosecond range from your queries or screenshots.",
        "input_schema": {
            "type": "object",
            "properties": {
                "entry_slug": { "type": "string", "description": "Processor row, e.g. \"n0_gpu_g0\"." },
                "start_ns": { "type": "integer", "description": "Start of the region (ns)." },
                "stop_ns": { "type": "integer", "description": "End of the region (ns)." },
                "severity": { "type": "string", "description": "\"critical\", \"high\", or \"medium\" (default medium)." },
                "label": { "type": "string", "description": "Short description shown on the chip." }
            },
            "required": ["entry_slug", "start_ns", "stop_ns"]
        }
    }));

    tools.push(serde_json::json!({
        "name": "ask_user",
        "description":
            "Ask the user a clarifying question and wait for their answer. Use this \
             when the request is ambiguous, when you must choose between materially \
             different interpretations, or when you are unsure what the user wants — \
             prefer asking over guessing. Provide 2–4 concrete options when you can.",
        "input_schema": {
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The question to ask the user."
                },
                "options": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional 2–4 suggested answers, shown to the user as buttons."
                }
            },
            "required": ["question"]
        }
    }));

    tools.push(serde_json::json!({
        "name": "clear_highlights",
        "description":
            "Remove ALL highlight overlays from the timeline. Use this when the user \
             asks to remove, clear, undo, or hide the highlights you added.",
        "input_schema": {
            "type": "object",
            "properties": {},
            "required": []
        }
    }));

    tools.push(serde_json::json!({
        "name": "update_findings",
        "description":
            "Record durable conclusions about THIS profile (its structure, the main \
             bottleneck, things you've ruled out) as short notes. They persist across \
             the user's questions and are shown back to you at the start of each new \
             question, so you don't re-derive them. Appends one bullet by default; pass \
             replace=true with a consolidated multi-line note to rewrite the whole list \
             (use this to correct or prune outdated notes). Keep each note to one terse \
             sentence.",
        "input_schema": {
            "type": "object",
            "properties": {
                "note": {
                    "type": "string",
                    "description": "The finding(s). One bullet, or multiple lines when replace=true."
                },
                "replace": {
                    "type": "boolean",
                    "description": "If true, replace ALL existing findings with this note. Default false (append)."
                }
            },
            "required": ["note"]
        }
    }));

    tools
}

/// Lock-in tests for the when-to-use trigger text in tool descriptions (tool-descriptions
/// task). `tool_definitions` is a pure JSON builder, so these run under {ai} alone — they
/// guard against a future edit silently dropping the triggers that drive tool selection.
#[cfg(test)]
mod tool_description_tests {
    use super::*;

    /// Description string for a tool, with run_query + wiki tools forced on.
    fn desc(name: &str) -> String {
        tool_definitions(true, false, true)
            .into_iter()
            .find(|t| t["name"] == name)
            .and_then(|t| t["description"].as_str().map(str::to_owned))
            .unwrap_or_default()
    }

    #[test]
    fn test_run_query_range_trigger_precedes_filter_example() {
        let d = desc("run_query");
        assert!(
            d.contains("in the range"),
            "run_query missing the range-trigger phrase"
        );
        assert!(
            d.contains("LEAST") && d.contains("GREATEST"),
            "run_query missing the CLIP formula"
        );
        // The trigger must come BEFORE the plain-filter example #2, or the agent reads
        // the wrong pattern first.
        let trig = d.find("in the range").expect("range trigger present");
        let ex2 = d.find("Tasks in a time range").expect("example #2 present");
        assert!(
            trig < ex2,
            "range trigger must precede example #2 (trig={trig}, ex2={ex2})"
        );
    }

    #[test]
    fn test_wiki_descriptions_have_when_to_use() {
        for name in ["wiki_index", "wiki_read", "wiki_search"] {
            assert!(
                desc(name).contains("USE THIS WHEN"),
                "{name} description missing the WHEN-to-use trigger"
            );
        }
    }

    #[test]
    fn test_wiki_search_has_keyword_cues() {
        let d = desc("wiki_search");
        assert!(
            d.contains("bound"),
            "wiki_search missing the 'bound' keyword cue"
        );
        assert!(
            d.contains("mapper"),
            "wiki_search missing the 'mapper' keyword cue"
        );
    }
}

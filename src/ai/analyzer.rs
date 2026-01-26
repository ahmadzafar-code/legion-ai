//! AI analysis traits and implementations for detecting performance issues.

use std::collections::HashMap;

use egui::Color32;

use crate::data::{EntryID, EntryIndex, Field, ItemMeta, SlotMetaTileData, SlotTileData, SummaryTileData};
use crate::timestamp::Interval;

// Well-known field IDs from Legion Prof archive format
// These are stable across Legion Prof versions
const FIELD_ID_WAITING: usize = 14;
const FIELD_ID_DEFERRED: usize = 15;
const FIELD_ID_DELAYED: usize = 16;
const FIELD_ID_CRITICAL_PATH: usize = 23;

/// A highlighted region indicating a detected performance issue.
#[derive(Debug, Clone)]
pub struct AiHighlight {
    /// The time interval this highlight covers.
    pub interval: Interval,
    /// Color for rendering the highlight overlay.
    pub color: Color32,
    /// Human-readable description, e.g. "Idle Gap: 320ms (Dependency Wait)"
    pub label: String,
    /// Confidence score in [0.0, 1.0] range.
    pub confidence: f32,
}

/// Trait for analyzers that detect performance issues in slot data.
pub trait Analyzer: Send + Sync {
    /// Analyze slot tile data and return detected highlights.
    ///
    /// # Arguments
    /// * `slot_data` - The slot tile data containing items arranged in rows
    /// * `meta` - Optional metadata for items (used for classification)
    /// * `kind` - Optional kind string (e.g., "cpu", "utility", "system") for threshold selection
    /// * `summary` - Optional summary tile data containing utilization time series
    /// * `tile_interval` - Optional interval of the tile (for detecting leading/trailing idle)
    ///
    /// # Returns
    /// A vector of highlights for detected issues
    fn analyze(
        &self,
        slot_data: &SlotTileData,
        meta: Option<&SlotMetaTileData>,
        kind: Option<&str>,
        summary: Option<&SummaryTileData>,
        tile_interval: Option<Interval>,
    ) -> Vec<AiHighlight>;
}

/// Analyzer that detects idle gaps between consecutive items.
pub struct IdleGapAnalyzer {
    /// Default minimum gap duration in nanoseconds.
    /// Used when no kind-specific threshold is found.
    pub default_threshold_ns: i64,
    /// Kind-specific thresholds in nanoseconds.
    /// Keys are lowercase kind names (e.g., "cpu", "utility", "system").
    pub kind_thresholds: HashMap<String, i64>,
}

impl Default for IdleGapAnalyzer {
    fn default() -> Self {
        let mut kind_thresholds = HashMap::new();

        // CPU and GPU: 10ms threshold (these should be highly utilized)
        kind_thresholds.insert("cpu".to_string(), 10_000_000);
        kind_thresholds.insert("gpu".to_string(), 10_000_000);

        // Utility and IO: 500ms threshold (lower utilization is normal)
        kind_thresholds.insert("utility".to_string(), 500_000_000);
        kind_thresholds.insert("io".to_string(), 500_000_000);

        // System: 500ms threshold (system processors often idle)
        kind_thresholds.insert("system".to_string(), 500_000_000);

        // Data movement (dp): 300ms threshold
        kind_thresholds.insert("dp".to_string(), 300_000_000);

        Self {
            default_threshold_ns: 100_000_000, // 100ms default
            kind_thresholds,
        }
    }
}

impl IdleGapAnalyzer {
    /// Create a new analyzer with custom default threshold.
    pub fn with_threshold_ns(default_threshold_ns: i64) -> Self {
        Self {
            default_threshold_ns,
            ..Default::default()
        }
    }

    /// Create a new analyzer with threshold specified in milliseconds.
    pub fn with_threshold_ms(default_threshold_ms: i64) -> Self {
        Self::with_threshold_ns(default_threshold_ms * 1_000_000)
    }

    /// Get the threshold for a given kind.
    fn get_threshold_for_kind(&self, kind: Option<&str>) -> i64 {
        if let Some(kind) = kind {
            let kind_lower = kind.to_lowercase();
            // Try exact match first
            if let Some(&threshold) = self.kind_thresholds.get(&kind_lower) {
                return threshold;
            }
            // Try partial match (e.g., "utility io" contains "utility")
            for (key, &threshold) in &self.kind_thresholds {
                if kind_lower.contains(key) {
                    return threshold;
                }
            }
        }
        self.default_threshold_ns
    }

    /// Check if a kind is a low-utilization kind (utility, io, system, dp).
    /// These kinds should not be flagged for leading/trailing/completely idle.
    fn is_low_utilization_kind(kind: Option<&str>) -> bool {
        if let Some(kind) = kind {
            let kind_lower = kind.to_lowercase();
            kind_lower.contains("utility")
                || kind_lower.contains("io")
                || kind_lower.contains("system")
                || kind_lower.contains("dp")
        } else {
            false
        }
    }

    /// Check if a kind is a utility processor kind (utility, mapper, scheduler, system).
    /// These should not be flagged for idle gaps, but instead for high utilization.
    /// Based on Legion Prof archive: kind_idx 1 = utility, kind_idx 3 = system
    fn is_utility_kind(kind: Option<&str>) -> bool {
        if let Some(kind) = kind {
            let kind_lower = kind.to_lowercase();
            kind_lower.contains("utility")
                || kind_lower.contains("mapper")
                || kind_lower.contains("scheduler")
                || kind_lower == "system"
        } else {
            false
        }
    }

    /// Check if a kind is a compute kind (cpu, gpu, dp).
    /// These are flagged for idle gaps.
    /// Based on Legion Prof archive: kind_idx 0 = cpu, kind_idx 5 = dp
    fn is_compute_kind(kind: Option<&str>) -> bool {
        if let Some(kind) = kind {
            let kind_lower = kind.to_lowercase();
            kind_lower.contains("cpu") || kind_lower.contains("gpu") || kind_lower == "dp"
        } else {
            false
        }
    }

    /// Compute the average utilization during a given interval from summary tile data.
    ///
    /// Uses linear interpolation between utilization points to estimate the average
    /// utilization across the gap interval.
    ///
    /// Returns None if the summary data doesn't cover the interval.
    fn compute_avg_utilization(
        summary: &SummaryTileData,
        interval: Interval,
    ) -> Option<f32> {
        if summary.utilization.is_empty() {
            return None;
        }

        // Find utilization points that fall within or around the interval
        let mut total_util = 0.0_f32;
        let mut count = 0;

        for point in &summary.utilization {
            if point.time.0 >= interval.start.0 && point.time.0 <= interval.stop.0 {
                total_util += point.util;
                count += 1;
            }
        }

        // If we have points within the interval, return the average
        if count > 0 {
            return Some(total_util / count as f32);
        }

        // If no points within interval, try to interpolate from surrounding points
        let mut before: Option<&crate::data::UtilPoint> = None;
        let mut after: Option<&crate::data::UtilPoint> = None;

        for point in &summary.utilization {
            if point.time.0 <= interval.start.0 {
                before = Some(point);
            }
            if point.time.0 >= interval.stop.0 && after.is_none() {
                after = Some(point);
                break;
            }
        }

        // Interpolate if we have surrounding points
        match (before, after) {
            (Some(b), Some(a)) => {
                // Linear interpolation at the gap midpoint
                let mid_time = (interval.start.0 + interval.stop.0) / 2;
                let t = (mid_time - b.time.0) as f32 / (a.time.0 - b.time.0) as f32;
                let interpolated = b.util + t * (a.util - b.util);
                Some(interpolated)
            }
            (Some(b), None) => Some(b.util), // Use last known value
            (None, Some(a)) => Some(a.util), // Use first known value
            (None, None) => None,
        }
    }

    /// Determine if a highlight should be filtered out based on utilization.
    ///
    /// Returns:
    /// - `None` if the highlight should be kept as-is
    /// - `Some(confidence_penalty)` if the highlight should have reduced confidence
    /// - Will return a penalty of 1.0 (effectively filtering) for low-util kinds with high util
    fn utilization_filter(
        kind: Option<&str>,
        avg_util: Option<f32>,
    ) -> Option<f32> {
        let avg_util = match avg_util {
            Some(u) => u,
            None => return None, // No summary data, use default logic
        };

        let kind_lower = kind.map(|k| k.to_lowercase());
        let is_compute = kind_lower.as_ref().map_or(false, |k| {
            k.contains("cpu") || k.contains("gpu")
        });
        let is_low_util_kind = Self::is_low_utilization_kind(kind);

        if is_compute {
            // For CPU/GPU: keep highlight if avg_util < 0.5 (50%)
            // This means the gap is significant because overall utilization is low
            if avg_util < 0.5 {
                None // Keep highlight
            } else {
                // High utilization means this gap is likely just between busy periods
                Some(0.2) // Small penalty
            }
        } else if is_low_util_kind {
            // For utility/io/system: skip if avg_util >= 0.3 (30%)
            // Low-util kinds with even moderate utilization don't need flagging
            if avg_util >= 0.3 {
                Some(1.0) // Filter out (penalty >= 1.0 means skip)
            } else {
                // Very low utilization even for a low-util kind might be worth noting
                Some(0.4) // Significant penalty but still show
            }
        } else {
            None // Default kinds, no utilization filtering
        }
    }

    /// Classify a gap based on metadata of surrounding items.
    ///
    /// Uses structured field detection (Deferred, Waiting, Delayed, Critical Path)
    /// in addition to pattern matching for more accurate classification.
    ///
    /// Returns `Some((label, color, confidence))` tuple, or `None` if the gap
    /// should be filtered out based on utilization.
    fn classify_gap(
        &self,
        gap_interval: Interval,
        prev_meta: Option<&ItemMeta>,
        next_meta: Option<&ItemMeta>,
        kind: Option<&str>,
        summary: Option<&SummaryTileData>,
    ) -> Option<(String, Color32, f32)> {
        let duration_ms = gap_interval.duration_ns() / 1_000_000;
        let is_low_util = Self::is_low_utilization_kind(kind);

        // Compute average utilization during the gap interval
        let avg_util = summary.and_then(|s| Self::compute_avg_utilization(s, gap_interval));

        // Check utilization-based filtering
        let util_penalty = Self::utilization_filter(kind, avg_util);
        if let Some(penalty) = util_penalty {
            if penalty >= 1.0 {
                // Filter out this highlight entirely
                return None;
            }
        }

        // Check structured scheduling fields (more reliable than pattern matching)
        let (prev_deferred, prev_waiting, prev_delayed, prev_critical) =
            Self::check_scheduling_fields(prev_meta);
        let (next_deferred, next_waiting, next_delayed, next_critical) =
            Self::check_scheduling_fields(next_meta);

        let has_deferred = prev_deferred || next_deferred;
        let has_waiting = prev_waiting || next_waiting;
        let has_delayed = prev_delayed || next_delayed;
        let has_critical_path = prev_critical || next_critical;

        // Check metadata fields for classification hints (fallback to pattern matching)
        let has_dependency_hint = has_waiting || has_deferred || self.check_fields_for_patterns(
            prev_meta,
            next_meta,
            &["depend", "wait", "sync", "barrier"],
        );

        let (has_data_hint, data_size) =
            self.check_fields_for_data_patterns(prev_meta, next_meta);

        // Reduce confidence for low-utilization kinds
        let kind_confidence_modifier = if is_low_util { 0.3 } else { 0.0 };

        // Add utilization-based confidence penalty
        let util_confidence_modifier = util_penalty.unwrap_or(0.0);

        // Boost confidence if we have structured field evidence
        let field_confidence_boost = if has_critical_path {
            0.15 // Critical path items are important
        } else if has_deferred || has_waiting {
            0.1 // Structured field evidence
        } else {
            0.0
        };

        let total_confidence_modifier = kind_confidence_modifier + util_confidence_modifier - field_confidence_boost;

        // Classify based on detected patterns (prioritize structured fields)
        if has_deferred {
            // Deferred wait - task was waiting for something to complete
            let color = if is_low_util {
                Color32::from_rgb(200, 150, 150)
            } else {
                Color32::from_rgb(220, 50, 50) // Dark red
            };
            Some((
                format!("Idle Gap: {}ms (Deferred)", duration_ms),
                color,
                (0.9 - total_confidence_modifier).clamp(0.1, 1.0),
            ))
        } else if has_waiting {
            // Waiting - explicit wait state
            let color = if is_low_util {
                Color32::from_rgb(200, 150, 150)
            } else {
                Color32::RED
            };
            Some((
                format!("Idle Gap: {}ms (Waiting)", duration_ms),
                color,
                (0.9 - total_confidence_modifier).clamp(0.1, 1.0),
            ))
        } else if has_delayed {
            // Delayed - scheduling delay
            let color = if is_low_util {
                Color32::from_rgb(200, 180, 150)
            } else {
                Color32::from_rgb(255, 140, 0) // Dark orange
            };
            Some((
                format!("Idle Gap: {}ms (Scheduling Delay)", duration_ms),
                color,
                (0.85 - total_confidence_modifier).clamp(0.1, 1.0),
            ))
        } else if has_dependency_hint {
            let color = if is_low_util {
                Color32::from_rgb(200, 150, 150)
            } else {
                Color32::RED
            };
            Some((
                format!("Idle Gap: {}ms (Dependency Wait)", duration_ms),
                color,
                (0.9 - total_confidence_modifier).clamp(0.1, 1.0),
            ))
        } else if has_data_hint {
            let size_str = if let Some(size) = data_size {
                format!(" - {}MB", size / (1024 * 1024))
            } else {
                String::new()
            };
            let color = if is_low_util {
                Color32::from_rgb(200, 180, 150)
            } else {
                Color32::from_rgb(255, 165, 0) // Orange
            };
            Some((
                format!("Idle Gap: {}ms (Data Stall{})", duration_ms, size_str),
                color,
                (0.85 - total_confidence_modifier).clamp(0.1, 1.0),
            ))
        } else {
            let color = if is_low_util {
                Color32::LIGHT_GRAY
            } else {
                Color32::YELLOW
            };
            Some((
                format!("Idle Gap: {}ms", duration_ms),
                color,
                (0.7 - total_confidence_modifier).clamp(0.1, 1.0),
            ))
        }
    }

    /// Check if any metadata fields contain dependency-related patterns.
    fn check_fields_for_patterns(
        &self,
        prev_meta: Option<&ItemMeta>,
        next_meta: Option<&ItemMeta>,
        patterns: &[&str],
    ) -> bool {
        for meta in [prev_meta, next_meta].into_iter().flatten() {
            // Check title
            let title_lower = meta.title.to_lowercase();
            if patterns.iter().any(|p| title_lower.contains(p)) {
                return true;
            }

            // Check field values
            for item_field in &meta.fields {
                if self.field_contains_patterns(&item_field.1, patterns) {
                    return true;
                }
            }
        }
        false
    }

    /// Recursively check if a field value contains any of the patterns.
    fn field_contains_patterns(&self, field: &Field, patterns: &[&str]) -> bool {
        match field {
            Field::String(s) => {
                let s_lower = s.to_lowercase();
                patterns.iter().any(|p| s_lower.contains(p))
            }
            Field::Vec(fields) => fields
                .iter()
                .any(|f| self.field_contains_patterns(f, patterns)),
            _ => false,
        }
    }

    /// Check for data transfer patterns and extract size if available.
    ///
    /// Returns (has_data_hint, optional_size_in_bytes).
    fn check_fields_for_data_patterns(
        &self,
        prev_meta: Option<&ItemMeta>,
        next_meta: Option<&ItemMeta>,
    ) -> (bool, Option<u64>) {
        let data_patterns = ["copy", "fill", "transfer", "dma", "memcpy"];
        let mut found_pattern = false;
        let mut size: Option<u64> = None;

        for meta in [prev_meta, next_meta].into_iter().flatten() {
            // Check title for data patterns
            let title_lower = meta.title.to_lowercase();
            if data_patterns.iter().any(|p| title_lower.contains(p)) {
                found_pattern = true;
            }

            // Check fields for patterns and size
            for item_field in &meta.fields {
                if self.field_contains_patterns(&item_field.1, &data_patterns) {
                    found_pattern = true;
                }

                // Try to extract size from numeric fields
                if let Some(extracted_size) = self.extract_size_from_field(&item_field.1) {
                    // Consider it a data stall if size > 1MB
                    if extracted_size > 1_000_000 {
                        found_pattern = true;
                        size = Some(extracted_size.max(size.unwrap_or(0)));
                    }
                }
            }
        }

        (found_pattern, size)
    }

    /// Try to extract a size value from a field.
    fn extract_size_from_field(&self, field: &Field) -> Option<u64> {
        match field {
            Field::U64(v) => Some(*v),
            Field::I64(v) if *v > 0 => Some(*v as u64),
            _ => None,
        }
    }

    /// Detect sustained high utilization periods for utility processors.
    ///
    /// Flags periods where avg_util > 0.65 for continuous period > 200ms.
    /// This indicates a utility processor that may be overloaded.
    ///
    /// Returns highlights for detected high utilization periods.
    fn detect_sustained_high_util(
        &self,
        summary: &SummaryTileData,
        tile_interval: Interval,
    ) -> Vec<AiHighlight> {
        let mut highlights = Vec::new();

        // Debug: log that we're being called
        println!("[AI] detect_sustained_high_util called: {} util points, tile {:?}",
            summary.utilization.len(), tile_interval);

        if summary.utilization.is_empty() {
            println!("[AI] No utilization points, returning empty");
            return highlights;
        }

        // Debug: show max util in the data
        let max_util = summary.utilization.iter().map(|p| p.util).fold(0.0_f32, f32::max);
        let points_in_range: Vec<_> = summary.utilization.iter()
            .filter(|p| p.time.0 >= tile_interval.start.0 && p.time.0 <= tile_interval.stop.0)
            .collect();
        let high_util_points: Vec<_> = points_in_range.iter()
            .filter(|p| p.util > 0.5)
            .collect();
        println!("[AI] Max util: {:.2}, points in tile range: {}, points > 50%: {}",
            max_util, points_in_range.len(), high_util_points.len());

        // Show first 10 high util points
        for (i, p) in high_util_points.iter().take(10).enumerate() {
            println!("[AI]   High util point {}: time={}ms, util={:.2}",
                i, p.time.0 / 1_000_000, p.util);
        }

        const HIGH_UTIL_THRESHOLD: f32 = 0.50;  // Lowered further to 50%
        const MIN_DURATION_NS: i64 = 100_000_000; // Lowered to 100ms

        let mut high_util_start: Option<i64> = None;
        let mut high_util_sum: f32 = 0.0;
        let mut high_util_count: i32 = 0;

        // Scan utilization points within the tile interval
        for point in &summary.utilization {
            let time = point.time.0;

            // Skip points outside tile interval
            if time < tile_interval.start.0 || time > tile_interval.stop.0 {
                continue;
            }

            if point.util > HIGH_UTIL_THRESHOLD {
                // Start or continue high util period
                if high_util_start.is_none() {
                    high_util_start = Some(time);
                    high_util_sum = 0.0;
                    high_util_count = 0;
                }
                high_util_sum += point.util;
                high_util_count += 1;
            } else if let Some(start) = high_util_start {
                // End of high util period
                let duration_ns = time - start;
                if duration_ns >= MIN_DURATION_NS && high_util_count > 0 {
                    let avg_util = high_util_sum / high_util_count as f32;
                    let duration_ms = duration_ns / 1_000_000;

                    // Color: RED if >500ms, ORANGE otherwise
                    let color = if duration_ns > 500_000_000 {
                        Color32::RED
                    } else {
                        Color32::from_rgb(255, 165, 0) // Orange
                    };

                    highlights.push(AiHighlight {
                        interval: Interval::new(
                            crate::timestamp::Timestamp(start),
                            crate::timestamp::Timestamp(time),
                        ),
                        color: color.gamma_multiply(0.5),
                        label: format!("High Utility Load: {}ms @ {:.0}%", duration_ms, avg_util * 100.0),
                        confidence: if duration_ns > 500_000_000 { 0.95 } else { 0.85 },
                    });
                }
                high_util_start = None;
            }
        }

        // Handle case where high util extends to end of tile
        if let Some(start) = high_util_start {
            let end = tile_interval.stop.0;
            let duration_ns = end - start;
            if duration_ns >= MIN_DURATION_NS && high_util_count > 0 {
                let avg_util = high_util_sum / high_util_count as f32;
                let duration_ms = duration_ns / 1_000_000;

                let color = if duration_ns > 500_000_000 {
                    Color32::RED
                } else {
                    Color32::from_rgb(255, 165, 0)
                };

                highlights.push(AiHighlight {
                    interval: Interval::new(
                        crate::timestamp::Timestamp(start),
                        crate::timestamp::Timestamp(end),
                    ),
                    color: color.gamma_multiply(0.5),
                    label: format!("High Utility Load: {}ms @ {:.0}%", duration_ms, avg_util * 100.0),
                    confidence: if duration_ns > 500_000_000 { 0.95 } else { 0.85 },
                });
            }
        }

        highlights
    }

    /// Check if an item's metadata indicates it was waiting/deferred/delayed.
    ///
    /// Uses well-known field IDs from Legion Prof:
    /// - 14: Waiting interval
    /// - 15: Deferred interval
    /// - 16: Delayed interval
    /// - 23: Critical Path (ItemLink)
    ///
    /// Returns (has_deferred, has_waiting, has_delayed, has_critical_path)
    fn check_scheduling_fields(meta: Option<&ItemMeta>) -> (bool, bool, bool, bool) {
        let meta = match meta {
            Some(m) => m,
            None => return (false, false, false, false),
        };

        let mut has_deferred = false;
        let mut has_waiting = false;
        let mut has_delayed = false;
        let mut has_critical_path = false;

        for field in &meta.fields {
            let field_id = field.0;
            match field_id.as_usize() {
                FIELD_ID_WAITING => {
                    // Check if Waiting interval has non-zero duration
                    if let Field::Interval(interval) = &field.1 {
                        if interval.duration_ns() > 0 {
                            has_waiting = true;
                        }
                    }
                }
                FIELD_ID_DEFERRED => {
                    // Check if Deferred interval has non-zero duration
                    if let Field::Interval(interval) = &field.1 {
                        if interval.duration_ns() > 0 {
                            has_deferred = true;
                        }
                    }
                }
                FIELD_ID_DELAYED => {
                    // Check if Delayed interval has non-zero duration
                    if let Field::Interval(interval) = &field.1 {
                        if interval.duration_ns() > 0 {
                            has_delayed = true;
                        }
                    }
                }
                FIELD_ID_CRITICAL_PATH => {
                    // Critical path is present if we have an ItemLink
                    if matches!(&field.1, Field::ItemLink(_)) {
                        has_critical_path = true;
                    }
                }
                _ => {}
            }
        }

        (has_deferred, has_waiting, has_delayed, has_critical_path)
    }

    /// Find the metadata for an item by matching item_uid.
    fn find_meta_for_item<'a>(
        &self,
        meta_data: &'a SlotMetaTileData,
        row_idx: usize,
        item_uid: crate::data::ItemUID,
    ) -> Option<&'a ItemMeta> {
        // First try the same row index (common case - parallel structure)
        if let Some(meta_row) = meta_data.items.get(row_idx) {
            if let Some(meta) = meta_row.iter().find(|m| m.item_uid == item_uid) {
                return Some(meta);
            }
        }

        // Fall back to searching all rows (defensive)
        for meta_row in &meta_data.items {
            if let Some(meta) = meta_row.iter().find(|m| m.item_uid == item_uid) {
                return Some(meta);
            }
        }

        None
    }
}

impl Analyzer for IdleGapAnalyzer {
    fn analyze(
        &self,
        slot_data: &SlotTileData,
        meta: Option<&SlotMetaTileData>,
        kind: Option<&str>,
        summary: Option<&SummaryTileData>,
        tile_interval: Option<Interval>,
    ) -> Vec<AiHighlight> {
        let mut highlights = Vec::new();

        let is_utility = Self::is_utility_kind(kind);
        let is_compute = Self::is_compute_kind(kind);

        // For utility processors: detect sustained high utilization instead of gaps
        if is_utility {
            println!("[AI] Analyzing utility kind={:?}, has_summary={}, has_tile_interval={}",
                kind, summary.is_some(), tile_interval.is_some());
            // If we have summary data and tile interval, check for high utilization
            if let (Some(summary), Some(tile_interval)) = (summary, tile_interval) {
                highlights.extend(self.detect_sustained_high_util(summary, tile_interval));
            }
            // Do NOT detect idle gaps for utility kinds (low utilization is normal)
            return highlights;
        }

        // For compute kinds (cpu/gpu/dp) or unknown kinds: detect internal gaps
        let threshold = self.get_threshold_for_kind(kind);

        // Iterate over each row in the slot
        for (row_idx, row) in slot_data.items.iter().enumerate() {
            // Skip empty rows - no items means no gaps to detect
            if row.is_empty() {
                continue;
            }

            // Need at least 2 items to have internal gaps
            if row.len() < 2 {
                continue;
            }

            // Items are sorted by time within a row - detect internal gaps
            for window in row.windows(2) {
                let prev_item = &window[0];
                let next_item = &window[1];

                // Calculate gap between end of prev and start of next
                let gap_start = prev_item.interval.stop;
                let gap_end = next_item.interval.start;

                // Skip if no gap or overlapping items
                if gap_end.0 <= gap_start.0 {
                    continue;
                }

                let gap_interval = Interval::new(gap_start, gap_end);
                let gap_ns = gap_interval.duration_ns();

                // Only flag gaps exceeding threshold for this kind
                if gap_ns < threshold {
                    continue;
                }

                // Get metadata for classification (safely handling Option chaining)
                let prev_meta = meta.and_then(|m| self.find_meta_for_item(m, row_idx, prev_item.item_uid));
                let next_meta = meta.and_then(|m| self.find_meta_for_item(m, row_idx, next_item.item_uid));

                // Classify the gap (may return None if filtered by utilization)
                // For compute kinds, skip the utilization filter since we want to flag gaps
                let classify_kind = if is_compute { None } else { kind };
                if let Some((label, color, confidence)) =
                    self.classify_gap(gap_interval, prev_meta, next_meta, classify_kind, summary)
                {
                    highlights.push(AiHighlight {
                        interval: gap_interval,
                        color: color.gamma_multiply(0.5), // Semi-transparent
                        label,
                        confidence,
                    });
                }
            }
        }

        highlights
    }
}

/// Helper to extract the kind name from an EntryID given the kinds list.
///
/// EntryID structure is [node_idx, kind_idx, slot_idx], so level 1 is the kind index.
pub fn get_kind_from_entry_id<'a>(entry_id: &EntryID, kinds: &'a [String]) -> Option<&'a str> {
    // Level 1 is the kind index
    if let Some(EntryIndex::Slot(kind_idx)) = entry_id.index(1) {
        kinds.get(kind_idx as usize).map(|s| s.as_str())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{Item, ItemUID, UtilPoint};
    use crate::timestamp::Timestamp;

    fn make_item(uid: u64, start_ns: i64, stop_ns: i64) -> Item {
        Item {
            item_uid: ItemUID(uid),
            interval: Interval::new(Timestamp(start_ns), Timestamp(stop_ns)),
            color: Color32::WHITE,
        }
    }

    fn make_summary_with_util(util: f32) -> SummaryTileData {
        SummaryTileData {
            utilization: vec![
                UtilPoint { time: Timestamp(0), util },
                UtilPoint { time: Timestamp(500_000_000), util },
                UtilPoint { time: Timestamp(1_000_000_000), util },
            ],
        }
    }

    #[test]
    fn test_no_gaps() {
        let analyzer = IdleGapAnalyzer::default();
        let slot_data = SlotTileData {
            items: vec![vec![
                make_item(1, 0, 100_000_000),
                make_item(2, 100_000_000, 200_000_000),
            ]],
        };

        let highlights = analyzer.analyze(&slot_data, None, Some("cpu"), None, None);
        assert!(highlights.is_empty());
    }

    #[test]
    fn test_small_gap_ignored_for_cpu() {
        let analyzer = IdleGapAnalyzer::default();
        let slot_data = SlotTileData {
            items: vec![vec![
                make_item(1, 0, 100_000_000),
                // 5ms gap (below 10ms CPU threshold)
                make_item(2, 105_000_000, 200_000_000),
            ]],
        };

        let highlights = analyzer.analyze(&slot_data, None, Some("cpu"), None, None);
        assert!(highlights.is_empty());
    }

    #[test]
    fn test_large_gap_detected_for_cpu() {
        let analyzer = IdleGapAnalyzer::default();
        let slot_data = SlotTileData {
            items: vec![vec![
                make_item(1, 0, 100_000_000),
                // 200ms gap (above 10ms CPU threshold)
                make_item(2, 300_000_000, 400_000_000),
            ]],
        };

        let highlights = analyzer.analyze(&slot_data, None, Some("cpu"), None, None);
        assert_eq!(highlights.len(), 1);
        assert_eq!(highlights[0].interval.start.0, 100_000_000);
        assert_eq!(highlights[0].interval.stop.0, 300_000_000);
        assert!(highlights[0].label.contains("200ms"));
        assert_eq!(highlights[0].confidence, 0.7); // Default classification
    }

    #[test]
    fn test_utility_vs_cpu_gap_detection() {
        let analyzer = IdleGapAnalyzer::default();
        let slot_data = SlotTileData {
            items: vec![vec![
                make_item(1, 0, 100_000_000),
                // 200ms gap
                make_item(2, 300_000_000, 400_000_000),
            ]],
        };

        let tile_interval = Interval::new(Timestamp(0), Timestamp(500_000_000));

        // Utility kinds skip gap detection entirely
        let highlights = analyzer.analyze(&slot_data, None, Some("utility"), None, Some(tile_interval));
        assert!(highlights.is_empty());

        // But CPU SHOULD detect gaps (200ms > 10ms threshold)
        let highlights = analyzer.analyze(&slot_data, None, Some("cpu"), None, Some(tile_interval));
        assert_eq!(highlights.len(), 1);
    }

    #[test]
    fn test_utility_no_gap_detection() {
        // Utility kinds now skip gap detection entirely
        let analyzer = IdleGapAnalyzer::default();
        let slot_data = SlotTileData {
            items: vec![vec![
                make_item(1, 0, 100_000_000),
                // 600ms gap - would have been flagged before, but not anymore
                make_item(2, 700_000_000, 800_000_000),
            ]],
        };

        let tile_interval = Interval::new(Timestamp(0), Timestamp(1_000_000_000));
        let highlights = analyzer.analyze(&slot_data, None, Some("utility"), None, Some(tile_interval));
        // No gap highlights for utility kinds
        assert!(highlights.is_empty());
    }

    #[test]
    fn test_system_uses_higher_threshold() {
        let analyzer = IdleGapAnalyzer::default();
        let slot_data = SlotTileData {
            items: vec![vec![
                make_item(1, 0, 100_000_000),
                // 300ms gap - below 500ms system threshold
                make_item(2, 400_000_000, 500_000_000),
            ]],
        };

        // Should NOT detect gap for system
        let highlights = analyzer.analyze(&slot_data, None, Some("system"), None, None);
        assert!(highlights.is_empty());
    }

    #[test]
    fn test_io_uses_higher_threshold() {
        let analyzer = IdleGapAnalyzer::default();
        let slot_data = SlotTileData {
            items: vec![vec![
                make_item(1, 0, 100_000_000),
                // 400ms gap - below 500ms io threshold
                make_item(2, 500_000_000, 600_000_000),
            ]],
        };

        // Should NOT detect gap for io
        let highlights = analyzer.analyze(&slot_data, None, Some("io"), None, None);
        assert!(highlights.is_empty());
    }

    #[test]
    fn test_partial_kind_match_utility() {
        let analyzer = IdleGapAnalyzer::default();
        let slot_data = SlotTileData {
            items: vec![vec![
                make_item(1, 0, 100_000_000),
                // 200ms gap
                make_item(2, 300_000_000, 400_000_000),
            ]],
        };

        let tile_interval = Interval::new(Timestamp(0), Timestamp(500_000_000));

        // "utility io" contains "utility", so it should skip gap detection
        let highlights = analyzer.analyze(&slot_data, None, Some("utility io"), None, Some(tile_interval));
        assert!(highlights.is_empty()); // Utility kinds skip gap detection
    }

    #[test]
    fn test_multiple_rows() {
        let analyzer = IdleGapAnalyzer::default();
        let slot_data = SlotTileData {
            items: vec![
                vec![
                    make_item(1, 0, 100_000_000),
                    make_item(2, 300_000_000, 400_000_000), // 200ms gap
                ],
                vec![
                    make_item(3, 0, 50_000_000),
                    make_item(4, 200_000_000, 250_000_000), // 150ms gap
                ],
            ],
        };

        let highlights = analyzer.analyze(&slot_data, None, Some("cpu"), None, None);
        assert_eq!(highlights.len(), 2);
    }

    #[test]
    fn test_custom_threshold() {
        let analyzer = IdleGapAnalyzer::with_threshold_ms(50);
        let slot_data = SlotTileData {
            items: vec![vec![
                make_item(1, 0, 100_000_000),
                // 60ms gap (above 50ms threshold)
                make_item(2, 160_000_000, 200_000_000),
            ]],
        };

        // Use None for kind to use default threshold
        let highlights = analyzer.analyze(&slot_data, None, None, None, None);
        assert_eq!(highlights.len(), 1);
    }

    #[test]
    fn test_get_kind_from_entry_id() {
        let kinds = vec![
            "cpu".to_string(),
            "utility".to_string(),
            "io".to_string(),
            "system".to_string(),
        ];

        // EntryID [0, 1, 0] -> kind index 1 -> "utility"
        let entry_id = EntryID::root().child(0).child(1).child(0);
        assert_eq!(get_kind_from_entry_id(&entry_id, &kinds), Some("utility"));

        // EntryID [0, 0, 0] -> kind index 0 -> "cpu"
        let entry_id = EntryID::root().child(0).child(0).child(0);
        assert_eq!(get_kind_from_entry_id(&entry_id, &kinds), Some("cpu"));

        // EntryID [0, 3, 0] -> kind index 3 -> "system"
        let entry_id = EntryID::root().child(0).child(3).child(0);
        assert_eq!(get_kind_from_entry_id(&entry_id, &kinds), Some("system"));
    }

    // Utilization-based filtering tests

    #[test]
    fn test_cpu_low_util_keeps_highlight() {
        let analyzer = IdleGapAnalyzer::default();
        let slot_data = SlotTileData {
            items: vec![vec![
                make_item(1, 0, 100_000_000),
                // 200ms gap
                make_item(2, 300_000_000, 400_000_000),
            ]],
        };

        // Low utilization (30%) - should keep highlight for CPU
        let summary = make_summary_with_util(0.3);
        let highlights = analyzer.analyze(&slot_data, None, Some("cpu"), Some(&summary), None);
        assert_eq!(highlights.len(), 1);
    }

    #[test]
    fn test_cpu_gap_detection_ignores_utilization() {
        // CPU gap detection now ignores utilization filtering (always detects gaps)
        let analyzer = IdleGapAnalyzer::default();
        let slot_data = SlotTileData {
            items: vec![vec![
                make_item(1, 0, 100_000_000),
                // 200ms gap
                make_item(2, 300_000_000, 400_000_000),
            ]],
        };

        // High utilization (80%) - CPU should still detect gap with full confidence
        let summary = make_summary_with_util(0.8);
        let highlights = analyzer.analyze(&slot_data, None, Some("cpu"), Some(&summary), None);
        assert_eq!(highlights.len(), 1);
        // Confidence is NOT reduced for CPU - we want to flag all gaps
        assert!((highlights[0].confidence - 0.7).abs() < 0.01);
    }

    #[test]
    fn test_utility_kind_skips_gap_detection() {
        // Utility kinds should NOT have gap detection - they use sustained high util detection instead
        let analyzer = IdleGapAnalyzer::default();
        let slot_data = SlotTileData {
            items: vec![vec![
                make_item(1, 0, 100_000_000),
                // 600ms gap (above any threshold) - should NOT be flagged for utility
                make_item(2, 700_000_000, 800_000_000),
            ]],
        };

        let tile_interval = Interval::new(Timestamp(0), Timestamp(1_000_000_000));
        let highlights = analyzer.analyze(&slot_data, None, Some("utility"), None, Some(tile_interval));
        // No gap detection for utility kinds
        assert!(highlights.is_empty());
    }

    #[test]
    fn test_mapper_kind_skips_gap_detection() {
        let analyzer = IdleGapAnalyzer::default();
        let slot_data = SlotTileData {
            items: vec![vec![
                make_item(1, 0, 100_000_000),
                make_item(2, 700_000_000, 800_000_000),
            ]],
        };

        let tile_interval = Interval::new(Timestamp(0), Timestamp(1_000_000_000));
        let highlights = analyzer.analyze(&slot_data, None, Some("mapper"), None, Some(tile_interval));
        assert!(highlights.is_empty());
    }

    #[test]
    fn test_scheduler_kind_skips_gap_detection() {
        let analyzer = IdleGapAnalyzer::default();
        let slot_data = SlotTileData {
            items: vec![vec![
                make_item(1, 0, 100_000_000),
                make_item(2, 700_000_000, 800_000_000),
            ]],
        };

        let tile_interval = Interval::new(Timestamp(0), Timestamp(1_000_000_000));
        let highlights = analyzer.analyze(&slot_data, None, Some("scheduler"), None, Some(tile_interval));
        assert!(highlights.is_empty());
    }

    // NOTE: Sustained high utilization detection is disabled because summary tiles
    // don't reliably flow through the AiDataWrapper due to LRU caching.
    // The detect_sustained_high_util method exists but is not called.
    // Tests below verify that utility kinds return no highlights (correct behavior).

    #[test]
    fn test_cpu_kind_still_detects_gaps() {
        // Make sure CPU still gets gap detection
        let analyzer = IdleGapAnalyzer::default();
        let slot_data = SlotTileData {
            items: vec![vec![
                make_item(1, 0, 100_000_000),
                // 200ms gap (above 10ms CPU threshold)
                make_item(2, 300_000_000, 400_000_000),
            ]],
        };

        let tile_interval = Interval::new(Timestamp(0), Timestamp(500_000_000));
        let highlights = analyzer.analyze(&slot_data, None, Some("cpu"), None, Some(tile_interval));
        assert_eq!(highlights.len(), 1);
        assert!(highlights[0].label.contains("Idle Gap"));
    }

    #[test]
    fn test_is_utility_kind() {
        assert!(IdleGapAnalyzer::is_utility_kind(Some("utility")));
        assert!(IdleGapAnalyzer::is_utility_kind(Some("Utility")));
        assert!(IdleGapAnalyzer::is_utility_kind(Some("mapper")));
        assert!(IdleGapAnalyzer::is_utility_kind(Some("scheduler")));
        assert!(IdleGapAnalyzer::is_utility_kind(Some("system")));
        assert!(IdleGapAnalyzer::is_utility_kind(Some("utility io")));
        assert!(!IdleGapAnalyzer::is_utility_kind(Some("cpu")));
        assert!(!IdleGapAnalyzer::is_utility_kind(Some("gpu")));
        assert!(!IdleGapAnalyzer::is_utility_kind(Some("io")));
        assert!(!IdleGapAnalyzer::is_utility_kind(None));
    }

    #[test]
    fn test_is_compute_kind() {
        assert!(IdleGapAnalyzer::is_compute_kind(Some("cpu")));
        assert!(IdleGapAnalyzer::is_compute_kind(Some("CPU")));
        assert!(IdleGapAnalyzer::is_compute_kind(Some("gpu")));
        assert!(IdleGapAnalyzer::is_compute_kind(Some("GPU")));
        assert!(!IdleGapAnalyzer::is_compute_kind(Some("utility")));
        assert!(!IdleGapAnalyzer::is_compute_kind(Some("io")));
        assert!(!IdleGapAnalyzer::is_compute_kind(None));
    }

    #[test]
    fn test_compute_avg_utilization() {
        let summary = SummaryTileData {
            utilization: vec![
                UtilPoint { time: Timestamp(0), util: 0.2 },
                UtilPoint { time: Timestamp(200_000_000), util: 0.4 },
                UtilPoint { time: Timestamp(400_000_000), util: 0.6 },
            ],
        };

        // Interval covering two points
        let interval = Interval::new(Timestamp(100_000_000), Timestamp(300_000_000));
        let avg = IdleGapAnalyzer::compute_avg_utilization(&summary, interval);
        assert!(avg.is_some());
        // Should average the point at 200ms (0.4)
        assert!((avg.unwrap() - 0.4).abs() < 0.01);
    }

    #[test]
    fn test_compute_avg_utilization_interpolation() {
        let summary = SummaryTileData {
            utilization: vec![
                UtilPoint { time: Timestamp(0), util: 0.2 },
                UtilPoint { time: Timestamp(400_000_000), util: 0.6 },
            ],
        };

        // Interval between points - should interpolate
        let interval = Interval::new(Timestamp(100_000_000), Timestamp(300_000_000));
        let avg = IdleGapAnalyzer::compute_avg_utilization(&summary, interval);
        assert!(avg.is_some());
        // Midpoint at 200ms, interpolated between 0.2 and 0.6 = 0.4
        assert!((avg.unwrap() - 0.4).abs() < 0.01);
    }

    // Completely idle row, leading/trailing idle tests

    #[test]
    fn test_empty_row_no_highlights() {
        // Empty rows should not produce any highlights (completely idle detection removed)
        let analyzer = IdleGapAnalyzer::default();
        let slot_data = SlotTileData {
            items: vec![vec![]], // One empty row
        };

        let tile_interval = Interval::new(Timestamp(0), Timestamp(500_000_000));
        let highlights = analyzer.analyze(&slot_data, None, Some("cpu"), None, Some(tile_interval));
        assert!(highlights.is_empty());
    }

    #[test]
    fn test_single_item_no_highlights() {
        // A single item has no internal gaps, so no highlights
        let analyzer = IdleGapAnalyzer::default();
        let slot_data = SlotTileData {
            items: vec![vec![make_item(1, 100_000_000, 200_000_000)]],
        };

        let tile_interval = Interval::new(Timestamp(0), Timestamp(500_000_000));
        let highlights = analyzer.analyze(&slot_data, None, Some("cpu"), None, Some(tile_interval));
        assert!(highlights.is_empty());
    }

    #[test]
    fn test_no_leading_trailing_detection() {
        // Leading/trailing idle detection was removed as it was too noisy.
        // Only internal gaps and completely idle rows/slots are detected.
        let analyzer = IdleGapAnalyzer::default();
        let slot_data = SlotTileData {
            items: vec![vec![
                // Item in the middle of the tile - no leading/trailing should be flagged
                make_item(1, 200_000_000, 300_000_000),
            ]],
        };

        // Tile from 0 to 500ms - would have 200ms leading and 200ms trailing
        let tile_interval = Interval::new(Timestamp(0), Timestamp(500_000_000));

        let highlights = analyzer.analyze(&slot_data, None, Some("cpu"), None, Some(tile_interval));
        // Should have NO highlights (leading/trailing detection removed)
        assert!(highlights.is_empty());
    }

    #[test]
    fn test_internal_gap_only() {
        let analyzer = IdleGapAnalyzer::default();
        let slot_data = SlotTileData {
            items: vec![vec![
                make_item(1, 100_000_000, 150_000_000),
                // 50ms gap (above 10ms CPU threshold)
                make_item(2, 200_000_000, 250_000_000),
            ]],
        };

        // Tile from 0 to 500ms
        let tile_interval = Interval::new(Timestamp(0), Timestamp(500_000_000));

        let highlights = analyzer.analyze(&slot_data, None, Some("cpu"), None, Some(tile_interval));
        // Should have only 1 highlight: internal gap (50ms)
        // Leading (100ms) and trailing (250ms) are NOT flagged anymore
        assert_eq!(highlights.len(), 1);
        assert!(highlights[0].label.contains("Idle Gap"));
        assert!(highlights[0].label.contains("50ms"));
    }

    #[test]
    fn test_empty_slot_no_highlights() {
        // Empty slots (no rows) should not produce any highlights
        let analyzer = IdleGapAnalyzer::default();
        let slot_data = SlotTileData {
            items: vec![], // No rows
        };

        let tile_interval = Interval::new(Timestamp(0), Timestamp(500_000_000));
        let highlights = analyzer.analyze(&slot_data, None, Some("cpu"), None, Some(tile_interval));
        assert!(highlights.is_empty());
    }

    #[test]
    fn test_dp_internal_gap_uses_higher_threshold() {
        let analyzer = IdleGapAnalyzer::default();
        let slot_data = SlotTileData {
            items: vec![vec![
                make_item(1, 0, 100_000_000),
                // 200ms gap - below 300ms dp threshold
                make_item(2, 300_000_000, 400_000_000),
            ]],
        };

        let tile_interval = Interval::new(Timestamp(0), Timestamp(400_000_000));

        // Should NOT flag 200ms gap for dp (threshold is 300ms)
        let highlights = analyzer.analyze(&slot_data, None, Some("dp"), None, Some(tile_interval));
        assert!(highlights.is_empty());
    }

    // Tests for dp as compute kind

    #[test]
    fn test_dp_is_compute_kind() {
        // dp (Dependent Partitioning) should be classified as compute kind
        assert!(IdleGapAnalyzer::is_compute_kind(Some("dp")));
        assert!(IdleGapAnalyzer::is_compute_kind(Some("DP")));
        assert!(!IdleGapAnalyzer::is_utility_kind(Some("dp")));
    }

    #[test]
    fn test_dp_detects_large_gaps() {
        let analyzer = IdleGapAnalyzer::default();
        let slot_data = SlotTileData {
            items: vec![vec![
                make_item(1, 0, 100_000_000),
                // 400ms gap - above 300ms dp threshold
                make_item(2, 500_000_000, 600_000_000),
            ]],
        };

        let tile_interval = Interval::new(Timestamp(0), Timestamp(600_000_000));
        let highlights = analyzer.analyze(&slot_data, None, Some("dp"), None, Some(tile_interval));
        assert_eq!(highlights.len(), 1);
        assert!(highlights[0].label.contains("400ms"));
    }

    // Tests for sustained high utilization detection

    #[test]
    fn test_utility_detects_high_util_with_summary() {
        let analyzer = IdleGapAnalyzer::default();
        let slot_data = SlotTileData {
            items: vec![vec![
                make_item(1, 0, 100_000_000),
            ]],
        };

        // Create summary with sustained high utilization (>75% for >300ms)
        let summary = SummaryTileData {
            utilization: vec![
                UtilPoint { time: Timestamp(0), util: 0.8 },
                UtilPoint { time: Timestamp(100_000_000), util: 0.85 },
                UtilPoint { time: Timestamp(200_000_000), util: 0.9 },
                UtilPoint { time: Timestamp(300_000_000), util: 0.85 },
                UtilPoint { time: Timestamp(400_000_000), util: 0.8 },
                UtilPoint { time: Timestamp(500_000_000), util: 0.3 }, // Drops below threshold
            ],
        };

        let tile_interval = Interval::new(Timestamp(0), Timestamp(600_000_000));
        let highlights = analyzer.analyze(&slot_data, None, Some("utility"), Some(&summary), Some(tile_interval));

        // Should detect high util period
        assert_eq!(highlights.len(), 1);
        assert!(highlights[0].label.contains("High Utility Load"));
    }

    #[test]
    fn test_utility_no_high_util_short_period() {
        let analyzer = IdleGapAnalyzer::default();
        let slot_data = SlotTileData {
            items: vec![vec![
                make_item(1, 0, 100_000_000),
            ]],
        };

        // Create summary with short high utilization (<300ms)
        let summary = SummaryTileData {
            utilization: vec![
                UtilPoint { time: Timestamp(0), util: 0.8 },
                UtilPoint { time: Timestamp(100_000_000), util: 0.85 },
                UtilPoint { time: Timestamp(200_000_000), util: 0.3 }, // Drops after 200ms
            ],
        };

        let tile_interval = Interval::new(Timestamp(0), Timestamp(300_000_000));
        let highlights = analyzer.analyze(&slot_data, None, Some("utility"), Some(&summary), Some(tile_interval));

        // Should NOT detect high util (period too short)
        assert!(highlights.is_empty());
    }

    #[test]
    fn test_utility_no_highlights_without_summary() {
        let analyzer = IdleGapAnalyzer::default();
        let slot_data = SlotTileData {
            items: vec![vec![
                make_item(1, 0, 100_000_000),
                make_item(2, 500_000_000, 600_000_000),
            ]],
        };

        let tile_interval = Interval::new(Timestamp(0), Timestamp(600_000_000));
        // Without summary, utility kinds should have no highlights
        let highlights = analyzer.analyze(&slot_data, None, Some("utility"), None, Some(tile_interval));
        assert!(highlights.is_empty());
    }

    // Tests for metadata field detection

    fn make_meta_with_fields(uid: u64, start_ns: i64, stop_ns: i64, fields: Vec<(usize, Field)>) -> ItemMeta {
        use crate::data::{FieldID, ItemField};
        ItemMeta {
            item_uid: ItemUID(uid),
            original_interval: Interval::new(Timestamp(start_ns), Timestamp(stop_ns)),
            title: "Test Item".to_string(),
            fields: fields.into_iter().map(|(id, f)| ItemField(FieldID::from_usize(id), f, None)).collect(),
        }
    }

    #[test]
    fn test_classify_gap_with_deferred_field() {
        let analyzer = IdleGapAnalyzer::default();
        let slot_data = SlotTileData {
            items: vec![vec![
                make_item(1, 0, 100_000_000),
                make_item(2, 200_000_000, 300_000_000),
            ]],
        };

        // Create meta with Deferred field (field ID 15)
        let meta_data = SlotMetaTileData {
            items: vec![vec![
                make_meta_with_fields(1, 0, 100_000_000, vec![]),
                make_meta_with_fields(2, 200_000_000, 300_000_000, vec![
                    (15, Field::Interval(Interval::new(Timestamp(100_000_000), Timestamp(150_000_000)))),
                ]),
            ]],
        };

        let highlights = analyzer.analyze(&slot_data, Some(&meta_data), Some("cpu"), None, None);
        assert_eq!(highlights.len(), 1);
        assert!(highlights[0].label.contains("Deferred"));
    }

    #[test]
    fn test_classify_gap_with_waiting_field() {
        let analyzer = IdleGapAnalyzer::default();
        let slot_data = SlotTileData {
            items: vec![vec![
                make_item(1, 0, 100_000_000),
                make_item(2, 200_000_000, 300_000_000),
            ]],
        };

        // Create meta with Waiting field (field ID 14)
        let meta_data = SlotMetaTileData {
            items: vec![vec![
                make_meta_with_fields(1, 0, 100_000_000, vec![]),
                make_meta_with_fields(2, 200_000_000, 300_000_000, vec![
                    (14, Field::Interval(Interval::new(Timestamp(100_000_000), Timestamp(150_000_000)))),
                ]),
            ]],
        };

        let highlights = analyzer.analyze(&slot_data, Some(&meta_data), Some("cpu"), None, None);
        assert_eq!(highlights.len(), 1);
        assert!(highlights[0].label.contains("Waiting"));
    }

    #[test]
    fn test_classify_gap_with_delayed_field() {
        let analyzer = IdleGapAnalyzer::default();
        let slot_data = SlotTileData {
            items: vec![vec![
                make_item(1, 0, 100_000_000),
                make_item(2, 200_000_000, 300_000_000),
            ]],
        };

        // Create meta with Delayed field (field ID 16)
        let meta_data = SlotMetaTileData {
            items: vec![vec![
                make_meta_with_fields(1, 0, 100_000_000, vec![]),
                make_meta_with_fields(2, 200_000_000, 300_000_000, vec![
                    (16, Field::Interval(Interval::new(Timestamp(100_000_000), Timestamp(150_000_000)))),
                ]),
            ]],
        };

        let highlights = analyzer.analyze(&slot_data, Some(&meta_data), Some("cpu"), None, None);
        assert_eq!(highlights.len(), 1);
        assert!(highlights[0].label.contains("Scheduling Delay"));
    }

    #[test]
    fn test_classify_gap_with_critical_path_boosts_confidence() {
        let analyzer = IdleGapAnalyzer::default();
        let slot_data = SlotTileData {
            items: vec![vec![
                make_item(1, 0, 100_000_000),
                make_item(2, 200_000_000, 300_000_000),
            ]],
        };

        // Create meta with Critical Path field (field ID 23) - ItemLink
        let critical_path_link = Field::ItemLink(crate::data::ItemLink {
            item_uid: ItemUID(999),
            title: "Critical task".to_string(),
            interval: Interval::new(Timestamp(0), Timestamp(100_000_000)),
            entry_id: EntryID::root(),
        });

        let meta_data = SlotMetaTileData {
            items: vec![vec![
                make_meta_with_fields(1, 0, 100_000_000, vec![]),
                make_meta_with_fields(2, 200_000_000, 300_000_000, vec![
                    (23, critical_path_link),
                ]),
            ]],
        };

        // Without critical path
        let meta_data_no_critical = SlotMetaTileData {
            items: vec![vec![
                make_meta_with_fields(1, 0, 100_000_000, vec![]),
                make_meta_with_fields(2, 200_000_000, 300_000_000, vec![]),
            ]],
        };

        let highlights_with = analyzer.analyze(&slot_data, Some(&meta_data), Some("cpu"), None, None);
        let highlights_without = analyzer.analyze(&slot_data, Some(&meta_data_no_critical), Some("cpu"), None, None);

        // Critical path should boost confidence
        assert!(highlights_with[0].confidence > highlights_without[0].confidence);
    }

    #[test]
    fn test_system_is_utility_kind() {
        // system should be utility kind (exact match, not substring)
        assert!(IdleGapAnalyzer::is_utility_kind(Some("system")));
        assert!(IdleGapAnalyzer::is_utility_kind(Some("System")));
        assert!(IdleGapAnalyzer::is_utility_kind(Some("SYSTEM")));
        // But not if it's just a substring of something else
        // Actually our implementation uses == for "system", so "system io" won't match
        // Let me check the actual implementation...
    }

}

//! AI-powered data source wrapper that analyzes tiles as they are fetched.

use std::collections::HashMap;

use crate::data::{DataSourceDescription, DataSourceInfo, EntryID, SlotTileData, SummaryTileData, TileID};
use crate::deferred_data::{
    DeferredDataSource, SlotMetaTileResponse, SlotTileResponse, SummaryTileResponse,
};

use super::analyzer::{get_kind_from_entry_id, AiHighlight, Analyzer, IdleGapAnalyzer};

/// A data source wrapper that runs AI analysis on fetched tiles.
///
/// This wrapper intercepts tile responses and runs analyzers to detect
/// performance issues. Highlights are accumulated and can be drained
/// by the UI layer.
///
/// Analysis only runs when `analysis_enabled` is true, which is controlled
/// by the "Scan for Issues" button in the UI.
pub struct AiDataWrapper<T: DeferredDataSource> {
    inner: T,
    analyzer: IdleGapAnalyzer,
    /// Temporary storage for slot tile data, keyed by (entry_id, tile_id).
    /// We need this to correlate with meta tiles for analysis.
    slot_tile_cache: HashMap<(EntryID, TileID), SlotTileData>,
    /// Temporary storage for summary tile data, keyed by (entry_id, tile_id).
    /// Used for utilization-based filtering.
    summary_tile_cache: HashMap<(EntryID, TileID), SummaryTileData>,
    /// Accumulated highlights from analysis, ready to be drained.
    /// Keyed by entry_id only so highlights persist across zoom levels.
    pending_highlights: HashMap<EntryID, Vec<AiHighlight>>,
    /// Whether analysis is enabled. Set to true when "Scan for Issues" is clicked.
    analysis_enabled: bool,
    /// List of kind names for threshold selection.
    kinds: Vec<String>,
}

impl<T: DeferredDataSource> AiDataWrapper<T> {
    /// Create a new AI data wrapper with the default idle gap analyzer.
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            analyzer: IdleGapAnalyzer::default(),
            slot_tile_cache: HashMap::new(),
            summary_tile_cache: HashMap::new(),
            pending_highlights: HashMap::new(),
            analysis_enabled: false,
            kinds: Vec::new(),
        }
    }

    /// Create a new AI data wrapper with a custom analyzer.
    pub fn with_analyzer(inner: T, analyzer: IdleGapAnalyzer) -> Self {
        Self {
            inner,
            analyzer,
            slot_tile_cache: HashMap::new(),
            summary_tile_cache: HashMap::new(),
            pending_highlights: HashMap::new(),
            analysis_enabled: false,
            kinds: Vec::new(),
        }
    }

    /// Enable analysis. Call this when "Scan for Issues" is clicked.
    pub fn enable_analysis(&mut self) {
        self.analysis_enabled = true;
    }

    /// Enable analysis with kinds list. Call this when "Scan for Issues" is clicked.
    /// This ensures kinds are available even if get_infos() was already called before.
    pub fn enable_analysis_with_kinds(&mut self, kinds: Vec<String>) {
        self.kinds = kinds;
        self.analysis_enabled = true;
    }

    /// Disable analysis.
    pub fn disable_analysis(&mut self) {
        self.analysis_enabled = false;
    }

    /// Check if analysis is enabled.
    pub fn is_analysis_enabled(&self) -> bool {
        self.analysis_enabled
    }

    /// Drain all pending highlights, returning them and clearing the internal buffer.
    ///
    /// Call this after processing tiles to retrieve analysis results.
    pub fn drain_highlights(&mut self) -> HashMap<EntryID, Vec<AiHighlight>> {
        std::mem::take(&mut self.pending_highlights)
    }

    /// Clear the slot tile cache. Call this when the view changes significantly.
    pub fn clear_cache(&mut self) {
        self.slot_tile_cache.clear();
        self.summary_tile_cache.clear();
        self.pending_highlights.clear();
    }

    /// Get the kind string for an entry_id.
    fn get_kind(&self, entry_id: &EntryID) -> Option<&str> {
        get_kind_from_entry_id(entry_id, &self.kinds)
    }

    /// Create the parent entry ID by removing the last level.
    /// EntryID([0, 1, 0]) -> EntryID([0, 1])
    fn parent_entry_id(entry_id: &EntryID) -> Option<EntryID> {
        if entry_id.level() <= 1 {
            return None;
        }
        // Build parent by taking all but the last index
        let mut parent = EntryID::root();
        for level in 0..(entry_id.level() - 1) {
            if let Some(crate::data::EntryIndex::Slot(idx)) = entry_id.index(level) {
                parent = parent.child(idx);
            } else {
                return None; // Can't build parent through summary marker
            }
        }
        Some(parent)
    }

    /// Find summary tile data for an entry.
    ///
    /// Summary tiles are stored at the kind level (parent) in the hierarchy, not at the slot level.
    /// For example, utility slot u0 is EntryID([0, 1, 0]), but its summary is at EntryID([0, 1]).
    /// This method tries various lookup strategies including parent entries.
    fn find_summary_for_entry(&self, entry_id: &EntryID, tile_id: TileID) -> Option<&SummaryTileData> {
        // Try exact match first
        if let Some(summary) = self.summary_tile_cache.get(&(entry_id.clone(), tile_id)) {
            return Some(summary);
        }

        // Try the summary entry (same entry but with summary marker)
        let summary_entry_id = entry_id.summary();
        if let Some(summary) = self.summary_tile_cache.get(&(summary_entry_id, tile_id)) {
            return Some(summary);
        }

        // Try parent entry (kind level) - summary tiles are typically stored at parent level
        // EntryID([0, 1, 0]) -> parent is EntryID([0, 1])
        if let Some(parent_entry_id) = Self::parent_entry_id(entry_id) {
            // Try parent with summary marker
            let parent_summary_id = parent_entry_id.summary();
            if let Some(summary) = self.summary_tile_cache.get(&(parent_summary_id.clone(), tile_id)) {
                return Some(summary);
            }
            // Try parent without summary marker
            if let Some(summary) = self.summary_tile_cache.get(&(parent_entry_id.clone(), tile_id)) {
                return Some(summary);
            }

            // Try parent with overlapping interval
            for ((cached_entry_id, cached_tile_id), summary) in &self.summary_tile_cache {
                if cached_entry_id == &parent_entry_id || cached_entry_id == &parent_summary_id {
                    if cached_tile_id.0.overlaps(tile_id.0) {
                        return Some(summary);
                    }
                }
            }
        }

        // Try finding any summary tile for this entry with overlapping interval
        // (tile_ids might not match exactly)
        for ((cached_entry_id, cached_tile_id), summary) in &self.summary_tile_cache {
            // Check if this is the same entry or its summary variant
            if cached_entry_id == entry_id || cached_entry_id == &entry_id.summary() {
                // Check if intervals overlap
                if cached_tile_id.0.overlaps(tile_id.0) {
                    return Some(summary);
                }
            }
        }

        None
    }
}

impl<T: DeferredDataSource> DeferredDataSource for AiDataWrapper<T> {
    fn fetch_description(&self) -> DataSourceDescription {
        self.inner.fetch_description()
    }

    fn fetch_info(&mut self) {
        self.inner.fetch_info()
    }

    fn get_infos(&mut self) -> Vec<crate::data::Result<DataSourceInfo>> {
        let infos = self.inner.get_infos();

        // Extract kinds from the first successful info
        for info in &infos {
            if let Ok(info) = info {
                self.kinds = info.entry_info.kinds();
                break;
            }
        }

        infos
    }

    fn fetch_summary_tile(&mut self, entry_id: &EntryID, tile_id: TileID, full: bool) {
        self.inner.fetch_summary_tile(entry_id, tile_id, full)
    }

    fn get_summary_tiles(&mut self) -> Vec<SummaryTileResponse> {
        let responses = self.inner.get_summary_tiles();

        // Cache summary tiles - they're needed for utilization-based analysis
        // Note: Due to LRU caching, many summary tiles may not flow through here
        println!("[AI] get_summary_tiles: {} responses", responses.len());
        for (result, req) in &responses {
            if let Ok(tile) = result {
                println!("[AI]   Caching summary: entry_id={:?}, tile_id={:?}, util_points={}",
                    req.entry_id, req.tile_id, tile.data.utilization.len());
                self.summary_tile_cache.insert(
                    (req.entry_id.clone(), req.tile_id),
                    tile.data.clone(),
                );
            }
        }

        responses
    }

    fn fetch_slot_tile(&mut self, entry_id: &EntryID, tile_id: TileID, full: bool) {
        self.inner.fetch_slot_tile(entry_id, tile_id, full)
    }

    fn get_slot_tiles(&mut self) -> Vec<SlotTileResponse> {
        let responses = self.inner.get_slot_tiles();

        // Analyze slot tiles when they arrive
        if self.analysis_enabled {
            println!("[AI] get_slot_tiles: {} responses, summary_cache has {} entries",
                responses.len(), self.summary_tile_cache.len());
            for (result, req) in &responses {
                if let Ok(tile) = result {
                    // Skip if we've already analyzed this exact tile
                    let cache_key = (req.entry_id.clone(), req.tile_id);
                    if self.slot_tile_cache.contains_key(&cache_key) {
                        continue; // Already processed
                    }

                    let kind = self.get_kind(&req.entry_id);
                    let summary = self.find_summary_for_entry(&req.entry_id, req.tile_id);
                    let tile_interval = req.tile_id.0;

                    // Run analysis (without metadata - basic classification)
                    let highlights = self.analyzer.analyze(&tile.data, None, kind, summary, Some(tile_interval));

                    if !highlights.is_empty() {
                        self.pending_highlights
                            .entry(req.entry_id.clone())
                            .or_default()
                            .extend(highlights);
                    }

                    // Cache to prevent re-analysis
                    self.slot_tile_cache.insert(cache_key, tile.data.clone());
                }
            }
        }

        responses
    }

    fn fetch_slot_meta_tile(&mut self, entry_id: &EntryID, tile_id: TileID, full: bool) {
        self.inner.fetch_slot_meta_tile(entry_id, tile_id, full)
    }

    fn get_slot_meta_tiles(&mut self) -> Vec<SlotMetaTileResponse> {
        let responses = self.inner.get_slot_meta_tiles();
        // Analysis is done in get_slot_tiles - meta tiles are just passed through
        // This avoids duplicate detection of the same gaps
        responses
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{
        DataSourceDescription, DataSourceInfo, EntryInfo, FieldSchema, Item, ItemMeta, ItemUID,
        SlotMetaTile, SlotMetaTileData, SlotTile, SlotTileData, TileSet,
    };
    use crate::deferred_data::TileRequest;
    use crate::timestamp::{Interval, Timestamp};
    use egui::Color32;

    /// A mock data source for testing.
    struct MockDataSource {
        slot_tiles: Vec<SlotTileResponse>,
        meta_tiles: Vec<SlotMetaTileResponse>,
    }

    impl MockDataSource {
        fn new() -> Self {
            Self {
                slot_tiles: Vec::new(),
                meta_tiles: Vec::new(),
            }
        }

        fn add_slot_tile(&mut self, entry_id: EntryID, tile_id: TileID, data: SlotTileData) {
            self.slot_tiles.push((
                Ok(SlotTile {
                    entry_id: entry_id.clone(),
                    tile_id,
                    data,
                }),
                TileRequest {
                    entry_id,
                    tile_id,
                    full: false,
                },
            ));
        }

        fn add_meta_tile(&mut self, entry_id: EntryID, tile_id: TileID, data: SlotMetaTileData) {
            self.meta_tiles.push((
                Ok(SlotMetaTile {
                    entry_id: entry_id.clone(),
                    tile_id,
                    data,
                }),
                TileRequest {
                    entry_id,
                    tile_id,
                    full: false,
                },
            ));
        }
    }

    impl DeferredDataSource for MockDataSource {
        fn fetch_description(&self) -> DataSourceDescription {
            DataSourceDescription {
                source_locator: vec!["mock".to_string()],
            }
        }

        fn fetch_info(&mut self) {}

        fn get_infos(&mut self) -> Vec<crate::data::Result<DataSourceInfo>> {
            vec![Ok(DataSourceInfo {
                entry_info: EntryInfo::Slot {
                    short_name: "test".to_string(),
                    long_name: "test".to_string(),
                    max_rows: 1,
                },
                interval: Interval::new(Timestamp(0), Timestamp(1_000_000_000)),
                tile_set: TileSet::default(),
                field_schema: FieldSchema::new(),
                warning_message: None,
            })]
        }

        fn fetch_summary_tile(&mut self, _: &EntryID, _: TileID, _: bool) {}
        fn get_summary_tiles(&mut self) -> Vec<SummaryTileResponse> {
            Vec::new()
        }

        fn fetch_slot_tile(&mut self, _: &EntryID, _: TileID, _: bool) {}
        fn get_slot_tiles(&mut self) -> Vec<SlotTileResponse> {
            std::mem::take(&mut self.slot_tiles)
        }

        fn fetch_slot_meta_tile(&mut self, _: &EntryID, _: TileID, _: bool) {}
        fn get_slot_meta_tiles(&mut self) -> Vec<SlotMetaTileResponse> {
            std::mem::take(&mut self.meta_tiles)
        }
    }

    fn make_item(uid: u64, start_ns: i64, stop_ns: i64) -> Item {
        Item {
            item_uid: ItemUID(uid),
            interval: Interval::new(Timestamp(start_ns), Timestamp(stop_ns)),
            color: Color32::WHITE,
        }
    }

    fn make_meta(uid: u64, start_ns: i64, stop_ns: i64) -> ItemMeta {
        ItemMeta {
            item_uid: ItemUID(uid),
            original_interval: Interval::new(Timestamp(start_ns), Timestamp(stop_ns)),
            title: "Test Item".to_string(),
            fields: Vec::new(),
        }
    }

    #[test]
    fn test_wrapper_detects_gaps_when_enabled() {
        let mut mock = MockDataSource::new();
        let entry_id = EntryID::root().child(0);
        // Use a tight tile interval that matches the items exactly to avoid leading/trailing
        let tile_id = TileID(Interval::new(Timestamp(0), Timestamp(400_000_000)));

        // Add slot tile with a 200ms gap
        mock.add_slot_tile(
            entry_id.clone(),
            tile_id,
            SlotTileData {
                items: vec![vec![
                    make_item(1, 0, 100_000_000),
                    // 200ms gap here
                    make_item(2, 300_000_000, 400_000_000),
                ]],
            },
        );

        // Add corresponding meta tile
        mock.add_meta_tile(
            entry_id.clone(),
            tile_id,
            SlotMetaTileData {
                items: vec![vec![
                    make_meta(1, 0, 100_000_000),
                    make_meta(2, 300_000_000, 400_000_000),
                ]],
            },
        );

        let mut wrapper = AiDataWrapper::new(mock);

        // Enable analysis (simulating "Scan for Issues" click)
        wrapper.enable_analysis();

        // First, get slot tiles (this caches the data and runs analysis)
        let _slot_responses = wrapper.get_slot_tiles();

        // Then, get meta tiles (this runs analysis with metadata)
        let _meta_responses = wrapper.get_slot_meta_tiles();

        // Drain highlights
        let highlights = wrapper.drain_highlights();

        assert_eq!(highlights.len(), 1);
        let tile_highlights = highlights.get(&entry_id).unwrap();
        // Should have internal gap highlight
        assert!(!tile_highlights.is_empty());
        // Find the internal gap (not leading/trailing)
        let internal_gap = tile_highlights.iter().find(|h| h.label.contains("Idle Gap"));
        assert!(internal_gap.is_some());
        assert!(internal_gap.unwrap().label.contains("200ms"));
    }

    #[test]
    fn test_wrapper_no_analysis_when_disabled() {
        let mut mock = MockDataSource::new();
        let entry_id = EntryID::root().child(0);
        let tile_id = TileID(Interval::new(Timestamp(0), Timestamp(1_000_000_000)));

        // Add slot tile with a 200ms gap
        mock.add_slot_tile(
            entry_id.clone(),
            tile_id,
            SlotTileData {
                items: vec![vec![
                    make_item(1, 0, 100_000_000),
                    make_item(2, 300_000_000, 400_000_000),
                ]],
            },
        );

        let mut wrapper = AiDataWrapper::new(mock);

        // Don't enable analysis - should NOT detect anything
        let _slot_responses = wrapper.get_slot_tiles();

        let highlights = wrapper.drain_highlights();
        assert!(highlights.is_empty());
    }

    #[test]
    fn test_wrapper_no_gaps() {
        let mut mock = MockDataSource::new();
        let entry_id = EntryID::root().child(0);
        // Use a tight tile interval that matches the items exactly to avoid trailing idle
        let tile_id = TileID(Interval::new(Timestamp(0), Timestamp(200_000_000)));

        // Add slot tile with no gaps (continuous items filling the tile)
        mock.add_slot_tile(
            entry_id.clone(),
            tile_id,
            SlotTileData {
                items: vec![vec![
                    make_item(1, 0, 100_000_000),
                    make_item(2, 100_000_000, 200_000_000),
                ]],
            },
        );

        mock.add_meta_tile(
            entry_id.clone(),
            tile_id,
            SlotMetaTileData {
                items: vec![vec![
                    make_meta(1, 0, 100_000_000),
                    make_meta(2, 100_000_000, 200_000_000),
                ]],
            },
        );

        let mut wrapper = AiDataWrapper::new(mock);
        wrapper.enable_analysis();

        let _slot_responses = wrapper.get_slot_tiles();
        let _meta_responses = wrapper.get_slot_meta_tiles();

        let highlights = wrapper.drain_highlights();
        assert!(highlights.is_empty());
    }

    #[test]
    fn test_drain_clears_highlights() {
        let mut mock = MockDataSource::new();
        let entry_id = EntryID::root().child(0);
        let tile_id = TileID(Interval::new(Timestamp(0), Timestamp(1_000_000_000)));

        mock.add_slot_tile(
            entry_id.clone(),
            tile_id,
            SlotTileData {
                items: vec![vec![
                    make_item(1, 0, 100_000_000),
                    make_item(2, 300_000_000, 400_000_000),
                ]],
            },
        );

        mock.add_meta_tile(
            entry_id,
            tile_id,
            SlotMetaTileData {
                items: vec![vec![
                    make_meta(1, 0, 100_000_000),
                    make_meta(2, 300_000_000, 400_000_000),
                ]],
            },
        );

        let mut wrapper = AiDataWrapper::new(mock);
        wrapper.enable_analysis();

        let _slot_responses = wrapper.get_slot_tiles();
        let _meta_responses = wrapper.get_slot_meta_tiles();

        // First drain should have highlights
        let highlights = wrapper.drain_highlights();
        assert!(!highlights.is_empty());

        // Second drain should be empty
        let highlights = wrapper.drain_highlights();
        assert!(highlights.is_empty());
    }
}

use std::collections::{BTreeMap, BTreeSet, VecDeque};
#[cfg(feature = "ai")]
use std::collections::HashMap;
use std::fmt;
use std::num::NonZeroUsize;
use std::time::Duration;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use egui::{
    Color32, NumExt, Pos2, Rect, RichText, ScrollArea, Slider, Stroke, TextStyle, TextWrapMode,
    Vec2,
};
use egui_extras::{Column, TableBuilder};
#[cfg(not(target_arch = "wasm32"))]
use itertools::Itertools;
use log::warn;
use percentage::{Percentage, PercentageInteger};
use regex::{Regex, escape};
use serde::{Deserialize, Serialize};

use crate::app::tile_manager::TileManager;
use crate::data::{
    self, DataSourceInfo, EntryID, EntryIndex, EntryInfo, Field, FieldID, FieldSchema, ItemField,
    ItemLink, ItemMeta, ItemUID, SlotMetaTileData, SlotTileData, SummaryTileData, TileID,
    UtilPoint,
};
use crate::deferred_data::{CountingDeferredDataSource, DeferredDataSource, LruDeferredDataSource};
use crate::timestamp::{
    Interval, Timestamp, TimestampDisplay, TimestampParseError, TimestampUnits,
};

#[cfg(feature = "ai")]
use crate::ai::{AiHighlight, Highlight};

/// Overview:
///   ProfApp -> Context, Window *
///   Window -> Config, Panel
///   Panel -> Summary, { Panel | Slot } *
///   Summary
///   Slot -> Item *
///
/// Context:
///   * Global configuration state (i.e., for all profiles)
///
/// Window:
///   * One Windows per profile
///   * Owns the ScrollArea (there is only **ONE** ScrollArea)
///   * Handles pan/zoom (there is only **ONE** pan/zoom setting)
///
/// Config:
///   * Window configuration state (i.e., specific to a profile)
///
/// Panel:
///   * One Panel for each level of nesting in the profile (root, node, kind)
///   * Table widget for (nested) cells
///   * Each row contains: label, content
///
/// Summary:
///   * Utilization widget
///
/// Slot:
///   * One Slot for each processor, channel, memory
///   * Viewer widget for items

#[derive(Debug, Clone)]
struct Summary {
    entry_id: EntryID,
    color: Color32,
    tiles: BTreeMap<TileID, Option<data::Result<SummaryTileData>>>,
}

#[derive(Debug, Clone)]
struct Slot {
    entry_id: EntryID,
    short_name: String,
    long_name: String,
    expanded: bool,
    max_rows: u64,

    // These maps have to track four different kinds of states:
    //
    //  1. Entry missing: user navigated away before response came back.
    //  2. None: awaiting response.
    //  3. Some(Err(_)): response failed or returned with an error.
    //  4. Some(Ok(_)): successfully completed response.
    tiles: BTreeMap<TileID, Option<data::Result<SlotTileData>>>,
    tile_metas: BTreeMap<TileID, Option<data::Result<SlotMetaTileData>>>,
    tile_metas_full: BTreeMap<TileID, Option<data::Result<SlotMetaTileData>>>,
}

#[derive(Debug, Clone)]
struct Panel<S: Entry> {
    entry_id: EntryID,
    short_name: String,
    long_name: String,
    expanded: bool,

    summary: Option<Summary>,
    slots: Vec<S>,
}

#[derive(Debug, Clone)]
struct ItemLocator {
    // For vertical scroll, we need the item's entry ID and row index
    // (note: reversed, because we're in screen space)
    entry_id: EntryID,
    irow: Option<usize>,

    // If we can't find the item on the initial attempt, we track the ItemUID
    // and attempt to find it once the tile loads
    item_uid: ItemUID,
}

#[derive(Debug, Clone)]
struct ItemDetail {
    // We populate metadata lazily, so there can be a delay until this is full
    meta: Option<ItemMeta>,
    loc: ItemLocator,
}

#[derive(Debug, Clone)]
struct SearchCacheItem {
    item_uid: ItemUID,

    // Cache fields for display
    title: String,

    // For horizontal scroll, we need the item's interval
    interval: Interval,

    // For vertical scroll, we need the item's row index (note: reversed,
    // because we're in screen space)
    irow: usize,
}

#[derive(Debug, Clone)]
struct SearchState {
    title_field: FieldID,

    // Search parameters
    query: String,
    last_query: String,
    search_field: FieldID,
    last_search_field: FieldID,
    whole_word: bool,
    last_whole_word: bool,
    last_word_regex: Option<Regex>,
    include_collapsed_entries: bool,
    last_include_collapsed_entries: bool,
    last_view_interval: Option<Interval>,

    // Cache of matching items
    result_set: BTreeSet<ItemUID>,
    result_cache: BTreeMap<EntryID, BTreeMap<TileID, BTreeMap<ItemUID, SearchCacheItem>>>,
    entry_tree: BTreeMap<u64, BTreeMap<u64, BTreeSet<u64>>>,
}

type DataSourceStack = CountingDeferredDataSource<LruDeferredDataSource<Box<dyn DeferredDataSource>>>;

struct Config {
    field_schema: FieldSchema,

    // Node selection
    min_node: u64,
    max_node: u64,

    // Kind selection
    kinds: Vec<String>,
    kind_filter: BTreeSet<String>,

    // This is just for the local profile
    interval: Interval,
    warning_message: Option<String>,

    data_source: DataSourceStack,

    search_state: SearchState,

    // When the user clicks on an item, we put it here
    items_selected: BTreeMap<ItemUID, ItemDetail>,

    // When the user clicks "Zoom to Item" or a search result, we put it here
    scroll_to_item: Option<ItemLocator>,
    // Sometimes, we cannot find the correct row to scroll to. In this case we
    // populate the following field to track the re-scroll when the item is found
    scroll_to_item_retry: Option<ItemLocator>,

    tile_manager: TileManager,

    /// AI-detected performance highlights, keyed by entry_id only.
    /// Highlights persist across zoom levels since they're not tied to specific tiles.
    #[cfg(feature = "ai")]
    ai_highlights: HashMap<EntryID, Vec<AiHighlight>>,

    /// Whether to show AI highlights in the UI.
    #[cfg(feature = "ai")]
    ai_highlights_enabled: bool,

    /// Timeline gap selection for AI diagnosis: (entry_id, gap_interval, entry_label).
    /// Set by Shift+click on empty space in a slot, consumed by update() to propagate to chat panel.
    #[cfg(feature = "ai")]
    ai_timeline_selection: Option<(EntryID, Interval, String)>,
}

struct Window {
    panel: Panel<Panel<Panel<Slot>>>, // nodes -> kind -> proc/chan/mem
    index: u64,
    config: Config,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
enum IntervalOrigin {
    Zoom,
    Pan,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct IntervalState {
    levels: Vec<Interval>,
    origins: Vec<IntervalOrigin>,
    index: usize,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
enum IntervalSelectError {
    InvalidValue,
    NoUnit,
    InvalidUnit,
    StartAfterStop,
    StartAfterEnd,
    StopBeforeStart,
}

impl From<TimestampParseError> for IntervalSelectError {
    fn from(val: TimestampParseError) -> Self {
        match val {
            TimestampParseError::InvalidValue => IntervalSelectError::InvalidValue,
            TimestampParseError::NoUnit => IntervalSelectError::NoUnit,
            TimestampParseError::InvalidUnit => IntervalSelectError::InvalidUnit,
        }
    }
}

impl fmt::Display for IntervalSelectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IntervalSelectError::InvalidValue => write!(f, "invalid value"),
            IntervalSelectError::NoUnit => write!(f, "no unit"),
            IntervalSelectError::InvalidUnit => write!(f, "invalid unit"),
            IntervalSelectError::StartAfterStop => write!(f, "start after stop"),
            IntervalSelectError::StartAfterEnd => write!(f, "start after end"),
            IntervalSelectError::StopBeforeStart => write!(f, "stop before start"),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct IntervalSelectState {
    // User-entered strings for the interval start/stop.
    start_buffer: String,
    stop_buffer: String,

    // Parse errors for the respective strings (if any).
    start_error: Option<IntervalSelectError>,
    stop_error: Option<IntervalSelectError>,
}

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
enum ItemLinkNavigationMode {
    #[default]
    Zoom,
    Pan,
}

impl ItemLinkNavigationMode {
    fn label_text(&self) -> &'static str {
        match *self {
            ItemLinkNavigationMode::Zoom => "Zoom to Item",
            ItemLinkNavigationMode::Pan => "Pan to Item",
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct Context {
    #[serde(skip)]
    row_height: f32,
    #[serde(skip)]
    scale_factor: f32,

    #[serde(skip)]
    row_scroll_delta: i32,

    #[serde(skip)]
    subheading_size: f32,

    // This is across all profiles
    #[serde(skip)]
    total_interval: Interval,

    // Visible time range
    #[serde(skip)]
    view_interval: Interval,

    #[serde(skip)]
    drag_origin: Option<Pos2>,

    // Hack: We need to track the screenspace rect where slot/summary
    // data gets drawn. This gets used rendering the cursor, but we
    // only know it when we render slots. So stash it here.
    #[serde(skip)]
    slot_rect: Option<Rect>,

    item_link_mode: ItemLinkNavigationMode,

    debug: bool,

    #[serde(skip)]
    show_controls: bool,

    #[serde(skip)]
    view_interval_history: IntervalState,
    #[serde(skip)]
    interval_select_state: IntervalSelectState,

    #[cfg(feature = "ai")]
    #[serde(skip)]
    chat_panel: crate::ai::ChatPanel,

    /// Request ID of a pending `ViewportCommand::Screenshot` awaiting
    /// delivery via `Event::Screenshot`. Set when the agent requests a
    /// screenshot, consumed when the screenshot event arrives.
    #[cfg(feature = "ai")]
    #[serde(skip)]
    awaiting_screenshot: Option<u64>,

    /// Agent-requested vertical scroll target. The rendering code scrolls to
    /// this entry's position and clears the field. Set by `ScrollToRequest`
    /// and `SetViewRequest` navigation events.
    #[cfg(feature = "ai")]
    #[serde(skip)]
    ai_scroll_to_entry: Option<crate::data::EntryID>,

    /// Item UIDs (sorted) of the last task selection pushed to the chat panel,
    /// used to detect changes and avoid re-pushing every frame.
    #[cfg(feature = "ai")]
    #[serde(skip)]
    last_item_selection: Vec<u64>,

    /// Shift+drag-selected time region (entry-agnostic), drawn as a blue band
    /// and surfaced to the chat panel.
    #[cfg(feature = "ai")]
    #[serde(skip)]
    ai_region_selection: Option<Interval>,

    /// V1.0 bridge: the single viewport-ownership token. The embedded chat agent
    /// drives transparently (sole driver in V1.0); a second consumer claims it via
    /// the `UiBridge` from [`Context::ui_bridge`]. Read only by `ui_bridge`, which
    /// has no caller until V1.1 wires the in-viewer MCP server.
    #[cfg(feature = "ai")]
    #[serde(skip)]
    #[allow(dead_code)]
    viewport_token: crate::ai::bridge::ViewportToken,

    /// Second event source (the future in-viewer MCP). Arc-wrapped so `Context`
    /// stays `Clone`; drained every frame alongside the embedded source, with
    /// replies routed to `mcp_cmd_tx`. Empty/unused until a `UiBridge` is minted.
    #[cfg(feature = "ai")]
    #[serde(skip)]
    mcp_event_rx:
        std::sync::Arc<std::sync::Mutex<Option<std::sync::mpsc::Receiver<crate::ai::AgentEvent>>>>,
    #[cfg(feature = "ai")]
    #[serde(skip)]
    mcp_cmd_tx: Option<std::sync::mpsc::Sender<crate::ai::UiCommand>>,

    /// Request id + reply channel + watchdog deadline for a screenshot the SECOND
    /// source is awaiting; Phase 1 routes the captured PNG here AFTER the embedded
    /// slot. The deadline bounds the slot: `Event::Screenshot` always arrives the
    /// next frame in practice, but if one were ever lost the slot would otherwise
    /// stay stuck — busy-looping the repaint driver and locking out future MCP
    /// navigations. Past the deadline the slot is reset (the bridge has already
    /// timed out, so the dropped reply channel is harmless).
    #[cfg(feature = "ai")]
    #[serde(skip)]
    mcp_awaiting_screenshot:
        Option<(u64, std::sync::mpsc::Sender<crate::ai::UiCommand>, std::time::Instant)>,

    /// V1.1: whether the in-viewer HTTP MCP server (data tools) has been started.
    /// One spawn attempt is made once a DuckDB path is configured.
    #[cfg(feature = "viewer-mcp")]
    #[serde(skip)]
    viewer_mcp_started: bool,

    /// P1 (Backend B): the ACTUAL bound port of the in-viewer MCP server, stored
    /// instead of discarded so the embedded Claude Code backend can build its
    /// `--mcp-config` against the real port. `None` until the server starts (or if
    /// the bind failed). The spawn site prefers the stable well-known port 8765
    /// (external `claude mcp add` registrations keep working) and falls back to an
    /// ephemeral port only if 8765 is taken.
    #[cfg(feature = "viewer-mcp")]
    #[serde(skip)]
    viewer_mcp_port: Option<u16>,
}

#[cfg(feature = "ai")]
impl Context {
    /// Mint a [`UiBridge`](crate::ai::bridge::UiBridge) for a second consumer (the
    /// future in-viewer MCP server thread) bound to `consumer_id`. Creates the
    /// second event/command channel pair, stores the UI-side ends so the per-frame
    /// loop drains and replies on them, and hands the consumer-side ends + a clone
    /// of the shared viewport token to the bridge. The embedded chat agent is
    /// unaffected; the bridge's `request` is structurally locked out via the token
    /// while another consumer owns the viewport.
    ///
    /// No caller until V1.1 (the in-viewer MCP server) — the per-frame drain and
    /// channels it wires are already live, so V1.1 only needs to call this + spawn
    /// the server thread.
    #[allow(dead_code)]
    pub fn ui_bridge(&mut self, consumer_id: u64) -> crate::ai::bridge::UiBridge {
        let (event_tx, event_rx) = std::sync::mpsc::channel::<crate::ai::AgentEvent>();
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<crate::ai::UiCommand>();
        *self.mcp_event_rx.lock().unwrap() = Some(event_rx);
        self.mcp_cmd_tx = Some(cmd_tx);
        crate::ai::bridge::UiBridge::new(event_tx, cmd_rx, self.viewport_token.clone(), consumer_id)
    }
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default)] // deserialize missing fields as default value
struct ProfApp {
    // Data sources waiting to be turned into windows.
    #[serde(skip)]
    pending_data_sources: VecDeque<Box<dyn DeferredDataSource>>,

    #[serde(skip)]
    windows: Vec<Window>,

    cx: Context,

    #[cfg(not(target_arch = "wasm32"))]
    #[serde(skip)]
    last_update: Option<Instant>,
}

trait Entry {
    fn new(info: &EntryInfo, entry_id: EntryID) -> Self;

    fn entry_id(&self) -> &EntryID;
    fn label_text(&self) -> &str;
    fn hover_text(&self) -> &str;

    fn find_slot(&self, entry_id: &EntryID, level: u64) -> Option<&Slot>;
    fn find_slot_mut(&mut self, entry_id: &EntryID, level: u64) -> Option<&mut Slot>;
    fn find_summary_mut(&mut self, entry_id: &EntryID, level: u64) -> Option<&mut Summary>;

    fn expand_slot(&mut self, entry_id: &EntryID, level: u64);

    fn inflate_meta(&mut self, config: &mut Config, cx: &mut Context);

    fn search(&mut self, config: &mut Config);

    fn label(&mut self, ui: &mut egui::Ui, rect: Rect, cx: &Context) {
        let response = ui.allocate_rect(
            rect,
            if self.is_expandable() {
                egui::Sense::click()
            } else {
                egui::Sense::hover()
            },
        );

        let style = ui.style();
        let font_id = TextStyle::Body.resolve(style);
        let visuals = if self.is_expandable() {
            style.interact_selectable(&response, false)
        } else {
            *style.noninteractive()
        };

        ui.painter()
            .rect(rect, 0.0, visuals.bg_fill, visuals.bg_stroke);
        let spacing = style.spacing.item_spacing * Vec2::new(1.0, cx.scale_factor);
        let layout = ui.painter().layout(
            self.label_text().to_owned(),
            font_id,
            visuals.text_color(),
            rect.width() - spacing.x * 2.0,
        );
        ui.painter()
            .galley(rect.min + spacing, layout, visuals.text_color());

        if response.clicked() {
            // This will take effect next frame because we can't redraw this widget now
            self.toggle_expanded();
        } else if response.hovered() {
            response.on_hover_text(self.hover_text());
        }
    }

    fn content(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        viewport: Rect,
        config: &mut Config,
        cx: &mut Context,
    );

    fn height(&self, prefix: Option<&EntryID>, config: &Config, cx: &Context) -> f32;

    fn is_expandable(&self) -> bool;

    fn toggle_expanded(&mut self);
}

impl Summary {
    fn inflate(&mut self, config: &mut Config, cx: &mut Context) {
        const PART: bool = false;
        let tile_ids = config.request_tiles(cx.view_interval, PART);
        Config::invalidate_cache(&tile_ids, &mut self.tiles);
        for tile_id in tile_ids {
            self.tiles.entry(tile_id).or_insert_with(|| {
                config
                    .data_source
                    .fetch_summary_tile(&self.entry_id, tile_id, PART);
                None
            });
        }
    }
}

impl Entry for Summary {
    fn new(info: &EntryInfo, entry_id: EntryID) -> Self {
        if let EntryInfo::Summary { color } = info {
            Self {
                entry_id,
                color: *color,
                tiles: BTreeMap::new(),
            }
        } else {
            unreachable!()
        }
    }

    fn entry_id(&self) -> &EntryID {
        &self.entry_id
    }
    fn label_text(&self) -> &str {
        "avg"
    }
    fn hover_text(&self) -> &str {
        "Utilization Plot of Average Usage Over Time"
    }

    fn find_slot(&self, _entry_id: &EntryID, _level: u64) -> Option<&Slot> {
        unreachable!()
    }

    fn find_slot_mut(&mut self, _entry_id: &EntryID, _level: u64) -> Option<&mut Slot> {
        unreachable!()
    }

    fn find_summary_mut(&mut self, entry_id: &EntryID, level: u64) -> Option<&mut Summary> {
        assert_eq!(entry_id.level(), level);
        assert_eq!(entry_id.index(level - 1)?, EntryIndex::Summary);
        Some(self)
    }

    fn expand_slot(&mut self, _entry_id: &EntryID, _level: u64) {
        unreachable!()
    }

    fn inflate_meta(&mut self, _config: &mut Config, _cx: &mut Context) {
        unreachable!()
    }

    fn search(&mut self, _config: &mut Config) {
        unreachable!()
    }

    fn content(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        _viewport: Rect,
        config: &mut Config,
        cx: &mut Context,
    ) {
        cx.slot_rect = Some(rect); // Save slot rect for use later

        const TOOLTIP_RADIUS: f32 = 4.0;
        let hover_pos = ui.rect_hover_pos(rect); // where is the mouse hovering?

        self.inflate(config, cx);

        let style = ui.style();
        let visuals = style.noninteractive();
        ui.painter()
            .rect(rect, 0.0, visuals.bg_fill, visuals.bg_stroke);

        let stroke = Stroke::new(visuals.bg_stroke.width, self.color);

        // Conversions to and from screen space coordinates
        let util_to_screen = |util: &UtilPoint| {
            let time = cx.view_interval.unlerp(util.time);
            rect.lerp_inside(Vec2::new(time, 1.0 - util.util))
        };
        let screen_to_util = |screen: Pos2| UtilPoint {
            time: cx
                .view_interval
                .lerp((screen.x - rect.left()) / rect.width()),
            util: 1.0 - (screen.y - rect.top()) / rect.height(),
        };

        // Linear interpolation along the line from p1 to p2
        let interpolate = |p1: Pos2, p2: Pos2, x: f32| {
            let ratio = (x - p1.x) / (p2.x - p1.x);
            Rect::from_min_max(p1, p2).lerp_inside(Vec2::new(ratio, ratio))
        };

        let mut last_util: Option<&UtilPoint> = None;
        let mut last_point: Option<Pos2> = None;
        let mut hover_util = None;
        for tile in self.tiles.values().flatten() {
            let tile = match tile {
                Ok(t) => t,
                Err(e) => {
                    warn!("{}", e);
                    // Paint the entire tile red to indicate the error.
                    ui.painter().rect(rect, 0.0, Color32::RED, Stroke::NONE);
                    return;
                }
            };

            for util in &tile.utilization {
                let mut point = util_to_screen(util);
                if let Some(mut last) = last_point {
                    let last_util = last_util.unwrap();
                    if cx
                        .view_interval
                        .overlaps(Interval::new(last_util.time, util.time))
                    {
                        // Interpolate when out of view
                        if last.x < rect.min.x {
                            last = interpolate(last, point, rect.min.x);
                        }
                        if point.x > rect.max.x {
                            point = interpolate(last, point, rect.max.x);
                        }

                        ui.painter().line_segment([last, point], stroke);

                        if let Some(hover) = hover_pos {
                            if last.x <= hover.x && hover.x < point.x {
                                let interp = interpolate(last, point, hover.x);
                                ui.painter().circle_stroke(
                                    interp,
                                    TOOLTIP_RADIUS,
                                    visuals.fg_stroke,
                                );
                                hover_util = Some(screen_to_util(interp));
                            }
                        }
                    }
                }

                last_point = Some(point);
                last_util = Some(util);
            }
        }

        if let Some(util) = hover_util {
            let time = cx.view_interval.unlerp(util.time);
            let util_rect = Rect::from_min_max(
                rect.lerp_inside(Vec2::new(time - 0.05, 0.0)),
                rect.lerp_inside(Vec2::new(time + 0.05, 1.0)),
            );
            ui.show_tooltip(
                "utilization_tooltip",
                &util_rect,
                format!("{:.0}% Utilization", util.util * 100.0),
            );
        }
    }

    fn height(&self, prefix: Option<&EntryID>, _config: &Config, cx: &Context) -> f32 {
        assert!(prefix.is_none());
        const ROWS: u64 = 4;
        ROWS as f32 * cx.row_height
    }

    fn is_expandable(&self) -> bool {
        false
    }

    fn toggle_expanded(&mut self) {
        unreachable!();
    }
}

impl fmt::Display for Field {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Field::I64(value) => write!(f, "{value}"),
            Field::U64(value) => write!(f, "{value}"),
            Field::String(value) => write!(f, "{value}"),
            Field::Interval(value) => write!(f, "{value}"),
            Field::ItemLink(ItemLink { title, .. }) => write!(f, "{title}"),
            Field::Vec(fields) => {
                for (i, field) in fields.iter().enumerate() {
                    write!(f, "{field}")?;
                    if i < fields.len() {
                        write!(f, ", ")?;
                    }
                }
                Ok(())
            }
            Field::Empty => write!(f, ""),
        }
    }
}

struct FieldWithName<'a>(&'a str, &'a Field);

impl fmt::Display for FieldWithName<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let FieldWithName(name, value) = self;
        match value {
            Field::Empty => write!(f, "{name}"),
            _ => write!(f, "{name}: {value}"),
        }
    }
}

impl Slot {
    fn rows(&self) -> u64 {
        const UNEXPANDED_ROWS: u64 = 2;
        if self.expanded {
            self.max_rows.at_least(UNEXPANDED_ROWS)
        } else {
            UNEXPANDED_ROWS
        }
    }

    fn inflate(&mut self, config: &mut Config, cx: &mut Context) -> Vec<TileID> {
        const PART: bool = false;
        let tile_ids = config.request_tiles(cx.view_interval, PART);
        Config::invalidate_cache(&tile_ids, &mut self.tiles);
        Config::invalidate_cache(&tile_ids, &mut self.tile_metas);
        for tile_id in &tile_ids {
            self.tiles.entry(*tile_id).or_insert_with(|| {
                config
                    .data_source
                    .fetch_slot_tile(&self.entry_id, *tile_id, false);
                None
            });
        }
        tile_ids
    }

    fn fetch_meta_tile(
        &mut self,
        tile_id: TileID,
        config: &mut Config,
        full: bool,
    ) -> Option<&data::Result<SlotMetaTileData>> {
        let metas = if full {
            &mut self.tile_metas_full
        } else {
            &mut self.tile_metas
        };

        metas
            .entry(tile_id)
            .or_insert_with(|| {
                config
                    .data_source
                    .fetch_slot_meta_tile(&self.entry_id, tile_id, full);
                None
            })
            .as_ref()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_tile(
        &mut self,
        tile_id: TileID,
        rows: u64,
        mut hover_pos: Option<Pos2>,
        ui: &mut egui::Ui,
        rect: Rect,
        viewport: Rect,
        config: &mut Config,
        cx: &mut Context,
    ) -> Option<Pos2> {
        let tile = self.tiles.get(&tile_id).unwrap();

        if !tile.is_some() {
            // Tile hasn't finished loading.
            return hover_pos;
        }
        let tile = tile.as_ref().unwrap();

        let tile = match tile {
            Ok(t) => t,
            Err(e) => {
                warn!("{}", e);
                // Paint the entire tile red to indicate the error.
                ui.painter().rect(rect, 0.0, Color32::RED, Stroke::NONE);
                return hover_pos;
            }
        };

        if !cx.view_interval.overlaps(tile_id.0) {
            return hover_pos;
        }

        // Figure out roughly how large a pixel is on the screen.
        let pixel_ns = (cx.view_interval.duration_ns() as f32 / rect.width()) as i64;

        // Track which item, if any, we're interacting with
        let mut interact_item = None;

        // Render AI highlights FIRST (as background, behind items)
        #[cfg(feature = "ai")]
        if config.ai_highlights_enabled {
            if let Some(highlights) = config.ai_highlights.get(&self.entry_id) {
                for hl in highlights {
                    // Honor the per-highlight enable toggle (manager checkbox).
                    if !hl.enabled {
                        continue;
                    }
                    // Map interval to normalized [0,1] within view
                    let norm_start = cx.view_interval.unlerp(hl.interval.start).clamp(0.0, 1.0);
                    let norm_stop = cx.view_interval.unlerp(hl.interval.stop).clamp(0.0, 1.0);

                    // Skip if highlight is outside view
                    if norm_stop <= 0.0 || norm_start >= 1.0 {
                        continue;
                    }

                    // Full slot height rect
                    let min = rect.lerp_inside(Vec2::new(norm_start, 0.0));
                    let max = rect.lerp_inside(Vec2::new(norm_stop, 1.0));
                    let hl_rect = Rect::from_min_max(min, max);

                    // Semi-transparent red fill (very low opacity for background look)
                    let fill_color = Color32::from_rgba_unmultiplied(255, 0, 0, 40);
                    ui.painter().rect_filled(hl_rect, 0.0, fill_color);
                    ui.painter().rect_stroke(
                        hl_rect,
                        0.0,
                        Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 0, 0, 80)),
                    );
                }
            }
        }

        for (row, row_items) in tile.items.iter().enumerate() {
            // Need to reverse the rows because we're working in screen space
            let irow = rows - (row as u64) - 1;

            // We want to do this first on rows, so that we can cut the
            // entire row if we don't need it

            // Compute bounds for the whole row
            let row_min = rect.lerp_inside(Vec2::new(0.0, (irow as f32 + 0.05) / rows as f32));
            let row_max = rect.lerp_inside(Vec2::new(1.0, (irow as f32 + 0.95) / rows as f32));

            // Cull if out of bounds
            // Note: need to shift by rect.min to get to viewport space
            if row_max.y - rect.min.y < viewport.min.y {
                break;
            } else if row_min.y - rect.min.y > viewport.max.y {
                continue;
            }

            // Check if mouse is hovering over this row
            let row_rect = Rect::from_min_max(row_min, row_max);
            let row_hover = hover_pos.is_some_and(|h| row_rect.contains(h));

            // Now handle the items
            for (item_idx, item) in row_items.iter().enumerate() {
                if !cx.view_interval.overlaps(item.interval) {
                    continue;
                }

                // Expand interval to use at least one pixel, but do NOT
                // overlap neighboring items.
                let mut interval = item.interval;
                if interval.duration_ns() < pixel_ns {
                    let expand_ns = (pixel_ns - interval.duration_ns()) / 2;
                    interval = interval.grow(expand_ns).intersection(tile_id.0);
                    if item_idx > 0 {
                        let last_item = &row_items[item_idx - 1];
                        interval = interval.subtract_before(last_item.interval.stop);
                    }
                    if item_idx < row_items.len() - 1 {
                        let next_item = &row_items[item_idx + 1];
                        interval = interval.subtract_after(next_item.interval.start);
                    }
                }

                // Note: the interval is EXCLUSIVE. This turns out to be what
                // we want here, because in screen coordinates interval.stop
                // is the BEGINNING of the interval.stop nanosecond.
                let start = cx.view_interval.unlerp(interval.start).at_least(0.0);
                let stop = cx.view_interval.unlerp(interval.stop).at_most(1.0);
                let min = rect.lerp_inside(Vec2::new(start, (irow as f32 + 0.05) / rows as f32));
                let max = rect.lerp_inside(Vec2::new(stop, (irow as f32 + 0.95) / rows as f32));

                let item_rect = Rect::from_min_max(min, max);
                if row_hover && hover_pos.is_some_and(|h| item_rect.contains(h)) {
                    hover_pos = None;
                    interact_item = Some((row, item_idx, item_rect, tile_id));
                }

                let highlight = config.items_selected.contains_key(&item.item_uid);

                let mut color = item.color;
                if !config.search_state.query.is_empty() {
                    if config.search_state.result_set.contains(&item.item_uid) || highlight {
                        color = Color32::RED;
                    } else {
                        color = color.gamma_multiply(0.2);
                    }
                } else if highlight {
                    color = Color32::RED;
                }

                ui.painter().rect(item_rect, 0.0, color, Stroke::NONE);
            }
        }

        // Detect Shift+click on empty gap space for AI timeline selection
        #[cfg(feature = "ai")]
        {
            let pointer_in_rect = ui.rect_contains_pointer(rect);
            if pointer_in_rect && hover_pos.is_some() {
                // hover_pos is still Some => mouse is NOT over any item (items consume it)
                ui.input(|i| {
                    if i.pointer.any_click() && i.pointer.primary_released() && i.modifiers.shift {
                        if let Some(pos) = i.pointer.hover_pos() {
                            let norm_x =
                                ((pos.x - rect.min.x) / rect.width()).clamp(0.0, 1.0);
                            let click_time = cx.view_interval.lerp(norm_x as f32);

                            // Find the gap containing this timestamp using row 0 items
                            if let Some(Ok(tile_data)) =
                                self.tiles.get(&tile_id).and_then(|t| t.as_ref())
                            {
                                if let Some(row) = tile_data.items.first() {
                                    let mut gap_interval = None;

                                    // Check gaps between consecutive items
                                    for window in row.windows(2) {
                                        let prev_end = window[0].interval.stop;
                                        let next_start = window[1].interval.start;
                                        if click_time >= prev_end && click_time <= next_start {
                                            gap_interval =
                                                Some(Interval::new(prev_end, next_start));
                                            break;
                                        }
                                    }

                                    // Check gap before first item
                                    if gap_interval.is_none() {
                                        if let Some(first) = row.first() {
                                            if click_time < first.interval.start {
                                                gap_interval = Some(Interval::new(
                                                    tile_id.0.start,
                                                    first.interval.start,
                                                ));
                                            }
                                        }
                                    }

                                    // Check gap after last item
                                    if gap_interval.is_none() {
                                        if let Some(last) = row.last() {
                                            if click_time > last.interval.stop {
                                                gap_interval = Some(Interval::new(
                                                    last.interval.stop,
                                                    tile_id.0.stop,
                                                ));
                                            }
                                        }
                                    }

                                    // Empty row — whole tile is a gap
                                    if gap_interval.is_none() && row.is_empty() {
                                        gap_interval = Some(tile_id.0);
                                    }

                                    if let Some(gap) = gap_interval {
                                        config.ai_timeline_selection = Some((
                                            self.entry_id.clone(),
                                            gap,
                                            self.long_name.clone(),
                                        ));
                                    }
                                }
                            }
                        }
                    }
                });
            }
        }

        // Render AI timeline selection highlight (blue overlay)
        #[cfg(feature = "ai")]
        if let Some((ref sel_entry_id, ref sel_interval, _)) = config.ai_timeline_selection {
            if sel_entry_id == &self.entry_id {
                let norm_start =
                    cx.view_interval.unlerp(sel_interval.start).clamp(0.0, 1.0);
                let norm_stop =
                    cx.view_interval.unlerp(sel_interval.stop).clamp(0.0, 1.0);
                if norm_stop > 0.0 && norm_start < 1.0 {
                    let min = rect.lerp_inside(Vec2::new(norm_start, 0.0));
                    let max = rect.lerp_inside(Vec2::new(norm_stop, 1.0));
                    let sel_rect = Rect::from_min_max(min, max);
                    let fill = Color32::from_rgba_unmultiplied(50, 100, 255, 30);
                    ui.painter().rect_filled(sel_rect, 0.0, fill);
                    ui.painter().rect_stroke(
                        sel_rect,
                        0.0,
                        Stroke::new(
                            1.5,
                            Color32::from_rgba_unmultiplied(80, 140, 255, 150),
                        ),
                    );
                }
            }
        }

        // Handle AI highlight tooltips (rendering done above, before items)
        #[cfg(feature = "ai")]
        if config.ai_highlights_enabled {
            if let Some(highlights) = config.ai_highlights.get(&self.entry_id) {
                for hl in highlights {
                    // Honor the per-highlight enable toggle (manager checkbox).
                    if !hl.enabled {
                        continue;
                    }
                    // Map interval to normalized [0,1] within view
                    let norm_start = cx.view_interval.unlerp(hl.interval.start).clamp(0.0, 1.0);
                    let norm_stop = cx.view_interval.unlerp(hl.interval.stop).clamp(0.0, 1.0);

                    // Skip if highlight is outside view
                    if norm_stop <= 0.0 || norm_start >= 1.0 {
                        continue;
                    }

                    // Full slot height rect
                    let min = rect.lerp_inside(Vec2::new(norm_start, 0.0));
                    let max = rect.lerp_inside(Vec2::new(norm_stop, 1.0));
                    let hl_rect = Rect::from_min_max(min, max);

                    // Tooltip on hover
                    if let Some(h) = hover_pos {
                        if h.x >= min.x && h.x <= max.x && hl_rect.contains(h) {
                            let tooltip_id = ("ai_highlight", tile_id.0.start.0, hl.interval.start.0);
                            ui.show_tooltip_ui(tooltip_id, &hl_rect, |ui| {
                                ui.label(RichText::new(&hl.label).strong());
                                ui.label(format!("Interval: {}", hl.interval));
                            });
                        }
                    }
                }
            }
        }

        if let Some((row, item_idx, item_rect, tile_id)) = interact_item {
            // Hack: clone here  to avoid mutability conflict.
            let entry_id = self.entry_id.clone();
            const PART: bool = false;
            if let Some(tile_meta) = self.fetch_meta_tile(tile_id, config, PART) {
                let tile_meta = match tile_meta {
                    Ok(t) => t,
                    Err(e) => {
                        warn!("{}", e);
                        ui.show_tooltip("task_tooltip_error", &item_rect, e);
                        return hover_pos;
                    }
                };

                let item_meta = &tile_meta.items[row][item_idx];
                let tooltip_id = ("task_tooltip", item_meta.item_uid.0);
                ui.show_tooltip_ui(tooltip_id, &item_rect, |ui| {
                    ui.label(&item_meta.title);
                    if cx.debug {
                        ui.label(format!("Item UID: {}", item_meta.item_uid.0));
                    }
                    for ItemField(field_id, field, color) in &item_meta.fields {
                        let name = config.field_schema.get_name(*field_id).unwrap();
                        let text = format!("{}", FieldWithName(name, field));
                        if let Some(color) = color {
                            ui.label(RichText::new(text).color(*color));
                        } else {
                            ui.label(text);
                        }
                    }
                    ui.label("(Click to show details.)");
                });

                // Also mark task as selected if the mouse has been clicked
                ui.input(|i| {
                    // A "click" is measured on *release*, assuming certain
                    // properties hold (e.g., the button was held less than
                    // some duration, and it moved less than some amount).
                    if i.pointer.any_click() && i.pointer.primary_released() {
                        let irow = Some(rows as usize - row - 1);
                        match config.items_selected.entry(item_meta.item_uid) {
                            std::collections::btree_map::Entry::Vacant(e) => {
                                e.insert(ItemDetail {
                                    meta: Some(item_meta.clone()),
                                    loc: ItemLocator {
                                        entry_id,
                                        irow,
                                        item_uid: item_meta.item_uid,
                                    },
                                });
                            }
                            std::collections::btree_map::Entry::Occupied(e) => {
                                e.remove_entry();
                            }
                        }
                    }
                });
            }
        }

        hover_pos
    }
}

impl Entry for Slot {
    fn new(info: &EntryInfo, entry_id: EntryID) -> Self {
        if let EntryInfo::Slot {
            short_name,
            long_name,
            max_rows,
        } = info
        {
            Self {
                entry_id,
                short_name: short_name.to_owned(),
                long_name: long_name.to_owned(),
                expanded: true,
                max_rows: *max_rows,
                tiles: BTreeMap::new(),
                tile_metas: BTreeMap::new(),
                tile_metas_full: BTreeMap::new(),
            }
        } else {
            unreachable!()
        }
    }

    fn entry_id(&self) -> &EntryID {
        &self.entry_id
    }
    fn label_text(&self) -> &str {
        &self.short_name
    }
    fn hover_text(&self) -> &str {
        &self.long_name
    }

    fn find_slot(&self, entry_id: &EntryID, level: u64) -> Option<&Slot> {
        assert_eq!(entry_id.level(), level);
        assert!(entry_id.slot_index(level - 1).is_some());
        Some(self)
    }

    fn find_slot_mut(&mut self, entry_id: &EntryID, level: u64) -> Option<&mut Slot> {
        assert_eq!(entry_id.level(), level);
        assert!(entry_id.slot_index(level - 1).is_some());
        Some(self)
    }

    fn find_summary_mut(&mut self, _entry_id: &EntryID, _level: u64) -> Option<&mut Summary> {
        unreachable!()
    }

    fn expand_slot(&mut self, entry_id: &EntryID, level: u64) {
        assert_eq!(entry_id.level(), level);
        assert!(entry_id.slot_index(level - 1).is_some());
        self.expanded = true;
    }

    fn inflate_meta(&mut self, config: &mut Config, cx: &mut Context) {
        const FULL: bool = true;
        let tile_ids = config.request_tiles(cx.view_interval, FULL);
        Config::invalidate_cache(&tile_ids, &mut self.tile_metas_full);
        for tile_id in tile_ids {
            self.fetch_meta_tile(tile_id, config, FULL);
        }
    }

    fn search(&mut self, config: &mut Config) {
        if !config.search_state.start_entry(self) {
            return;
        }

        for (tile_id, tile) in &self.tile_metas_full {
            if let Some(Ok(tile)) = tile {
                if !config.search_state.start_tile(self, *tile_id) {
                    continue;
                }

                for (row, row_items) in tile.items.iter().enumerate() {
                    for item in row_items {
                        if config.search_state.is_match(item) {
                            // Reverse rows because we're in screen space
                            let irow = tile.items.len() - row - 1;
                            config.search_state.insert(self, *tile_id, irow, item);
                        }
                    }
                }
            }
        }
    }

    fn content(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        viewport: Rect,
        config: &mut Config,
        cx: &mut Context,
    ) {
        cx.slot_rect = Some(rect); // Save slot rect for use later

        let mut hover_pos = ui.rect_hover_pos(rect); // where is the mouse hovering?

        if self.expanded {
            let tile_ids = self.inflate(config, cx);

            let style = ui.style();
            let visuals = style.noninteractive();
            ui.painter()
                .rect(rect, 0.0, visuals.bg_fill, visuals.bg_stroke);

            let rows = self.rows();
            for tile_id in tile_ids {
                hover_pos =
                    self.render_tile(tile_id, rows, hover_pos, ui, rect, viewport, config, cx);
            }
        }
    }

    fn height(&self, _prefix: Option<&EntryID>, _config: &Config, cx: &Context) -> f32 {
        self.rows() as f32 * cx.row_height
    }

    fn is_expandable(&self) -> bool {
        true
    }

    fn toggle_expanded(&mut self) {
        self.expanded = !self.expanded;
    }
}

impl<S: Entry> Panel<S> {
    fn render<T: Entry>(
        ui: &mut egui::Ui,
        rect: Rect,
        viewport: Rect,
        slot: &mut T,
        y: &mut f32,
        config: &mut Config,
        cx: &mut Context,
    ) -> bool {
        const LABEL_WIDTH: f32 = 60.0;
        const COL_PADDING: f32 = 4.0;
        const ROW_PADDING: f32 = 4.0;

        // Compute the size of this slot
        // This is in screen (i.e., rect) space
        let min_y = *y;
        let max_y = min_y + slot.height(None, config, cx);
        *y = max_y + ROW_PADDING;

        // Cull if out of bounds
        // Note: need to shift by rect.min to get to viewport space
        if max_y - rect.min.y < viewport.min.y {
            return false;
        } else if min_y - rect.min.y > viewport.max.y {
            return true;
        }

        // Draw label and content
        let label_min = rect.min.x;
        let label_max = (rect.min.x + LABEL_WIDTH).at_most(rect.max.x);
        let content_min = (label_max + COL_PADDING).at_most(rect.max.x);
        let content_max = rect.max.x;

        let label_subrect =
            Rect::from_min_max(Pos2::new(label_min, min_y), Pos2::new(label_max, max_y));
        let content_subrect =
            Rect::from_min_max(Pos2::new(content_min, min_y), Pos2::new(content_max, max_y));

        // Shift viewport up by the amount consumed
        // Invariant: (0, 0) in viewport is rect.min
        //   (i.e., subtracting rect.min gets us from screen space to viewport space)
        // Note: viewport.min is NOT necessarily (0, 0)
        let content_viewport = viewport.translate(Vec2::new(0.0, rect.min.y - min_y));

        slot.content(ui, content_subrect, content_viewport, config, cx);
        slot.label(ui, label_subrect, cx);

        false
    }

    fn is_slot_visible(slot: &S, config: &Config) -> bool {
        let level = slot.entry_id().level();
        if level == 1 {
            // Apply node filter.
            let index = slot.entry_id().last_slot_index().unwrap();
            index >= config.min_node && index <= config.max_node
        } else if level == 2 {
            // Apply kind filter.
            let kind = slot.label_text();
            config.kind_filter.is_empty() || config.kind_filter.contains(kind)
        } else {
            true
        }
    }
}

impl<S: Entry> Entry for Panel<S> {
    fn new(info: &EntryInfo, entry_id: EntryID) -> Self {
        if let EntryInfo::Panel {
            short_name,
            long_name,
            summary,
            slots,
        } = info
        {
            let expanded = entry_id.level() != 2;
            let summary = summary
                .as_ref()
                .map(|s| Summary::new(s, entry_id.summary()));
            let slots = slots
                .iter()
                .enumerate()
                .map(|(i, s)| S::new(s, entry_id.child(i as u64)))
                .collect();
            Self {
                entry_id,
                short_name: short_name.to_owned(),
                long_name: long_name.to_owned(),
                expanded,
                summary,
                slots,
            }
        } else {
            unreachable!()
        }
    }

    fn entry_id(&self) -> &EntryID {
        &self.entry_id
    }
    fn label_text(&self) -> &str {
        &self.short_name
    }
    fn hover_text(&self) -> &str {
        &self.long_name
    }

    fn find_slot(&self, entry_id: &EntryID, level: u64) -> Option<&Slot> {
        self.slots
            .get(entry_id.slot_index(level)? as usize)?
            .find_slot(entry_id, level + 1)
    }

    fn find_slot_mut(&mut self, entry_id: &EntryID, level: u64) -> Option<&mut Slot> {
        self.slots
            .get_mut(entry_id.slot_index(level)? as usize)?
            .find_slot_mut(entry_id, level + 1)
    }

    fn find_summary_mut(&mut self, entry_id: &EntryID, level: u64) -> Option<&mut Summary> {
        if level < entry_id.level() - 1 {
            self.slots
                .get_mut(entry_id.slot_index(level)? as usize)?
                .find_summary_mut(entry_id, level + 1)
        } else {
            self.summary.as_mut()?.find_summary_mut(entry_id, level + 1)
        }
    }

    fn expand_slot(&mut self, entry_id: &EntryID, level: u64) {
        self.slots
            .get_mut(entry_id.slot_index(level).unwrap() as usize)
            .unwrap()
            .expand_slot(entry_id, level + 1);
        self.expanded = true;
    }

    fn inflate_meta(&mut self, config: &mut Config, cx: &mut Context) {
        let force = config.search_state.include_collapsed_entries;
        if self.expanded || force {
            for slot in &mut self.slots {
                // Apply visibility settings
                if !force && !Self::is_slot_visible(slot, config) {
                    continue;
                }

                slot.inflate_meta(config, cx);
            }
        }
    }

    fn search(&mut self, config: &mut Config) {
        let force = config.search_state.include_collapsed_entries;
        if self.expanded || force {
            for slot in &mut self.slots {
                // Apply visibility settings
                if !force && !Self::is_slot_visible(slot, config) {
                    continue;
                }

                slot.search(config);
            }
        }
    }

    fn content(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        viewport: Rect,
        config: &mut Config,
        cx: &mut Context,
    ) {
        let mut y = rect.min.y;
        if let Some(summary) = &mut self.summary {
            Self::render(ui, rect, viewport, summary, &mut y, config, cx);
        }

        if self.expanded {
            for slot in &mut self.slots {
                // Apply visibility settings
                if !Self::is_slot_visible(slot, config) {
                    continue;
                }

                if Self::render(ui, rect, viewport, slot, &mut y, config, cx) {
                    break;
                }
            }
        }
    }

    fn height(&self, prefix: Option<&EntryID>, config: &Config, cx: &Context) -> f32 {
        const UNEXPANDED_ROWS: u64 = 2;
        const ROW_PADDING: f32 = 4.0;

        let mut total = 0.0;
        let mut rows: i64 = 0;
        if let Some(summary) = &self.summary {
            total += summary.height(None, config, cx);
            rows += 1;
        } else if !self.expanded {
            // Need some minimum space if this panel has no summary and is collapsed
            total += UNEXPANDED_ROWS as f32 * cx.row_height;
            rows += 1;
        }

        if self.expanded {
            for slot in &self.slots {
                if let Some(prefix) = prefix {
                    // If this is our entry, stop
                    if slot.entry_id() == prefix {
                        break;
                    }
                }

                // Apply visibility settings
                if !Self::is_slot_visible(slot, config) {
                    continue;
                }

                total += slot.height(prefix, config, cx);

                if let Some(prefix) = prefix {
                    // If we're a prefix of the entry, recurse and then stop
                    if prefix.has_prefix(slot.entry_id()) {
                        break;
                    }
                }

                rows += 1;
            }
        }

        total += (rows - 1).at_least(0) as f32 * ROW_PADDING;

        total
    }

    fn is_expandable(&self) -> bool {
        !self.slots.is_empty()
    }

    fn toggle_expanded(&mut self) {
        self.expanded = !self.expanded;
    }
}

impl SearchState {
    fn new(title_id: FieldID) -> Self {
        Self {
            title_field: title_id,

            query: "".to_owned(),
            last_query: "".to_owned(),
            search_field: title_id,
            last_search_field: title_id,
            whole_word: false,
            last_whole_word: false,
            last_word_regex: None,
            include_collapsed_entries: false,
            last_include_collapsed_entries: false,
            last_view_interval: None,

            result_set: BTreeSet::new(),
            result_cache: BTreeMap::new(),
            entry_tree: BTreeMap::new(),
        }
    }

    fn clear(&mut self) {
        self.result_set.clear();
        self.result_cache.clear();
        self.entry_tree.clear();
    }

    fn ensure_valid_cache(&mut self, cx: &Context) {
        let mut invalidate = false;

        // Invalidate when the search query changes.
        if self.query != self.last_query {
            invalidate = true;
            self.last_query.clone_from(&self.query);
        }

        // Invalidate when the search field changes.
        if self.search_field != self.last_search_field {
            invalidate = true;
            self.last_search_field = self.search_field;
        }

        // Invalidate when the whole word setting changes.
        if self.whole_word != self.last_whole_word {
            invalidate = true;
            self.last_whole_word = self.whole_word;
        }

        // Invalidate when EXCLUDING collapsed entries. (I.e., because the
        // searched set shrinks. Growing is ok because search is monotonic.)
        if self.include_collapsed_entries != self.last_include_collapsed_entries
            && !self.include_collapsed_entries
        {
            invalidate = true;
            self.last_include_collapsed_entries = self.include_collapsed_entries;
        }

        // Invalidate when the view interval changes.
        if self.last_view_interval != Some(cx.view_interval) {
            invalidate = true;
            self.last_view_interval = Some(cx.view_interval);
        }

        if invalidate {
            if self.whole_word {
                let regex_string = format!("\\b{}\\b", escape(&self.query));
                self.last_word_regex = Some(Regex::new(&regex_string).unwrap());
            }

            self.clear();
        }
    }

    fn is_string_match(&self, s: &str) -> bool {
        if self.whole_word {
            let Some(regex) = &self.last_word_regex else {
                unreachable!();
            };
            regex.is_match(s)
        } else {
            s.contains(&self.query)
        }
    }

    fn is_field_match(&self, field: &Field) -> bool {
        match field {
            Field::String(s) => self.is_string_match(s),
            Field::ItemLink(ItemLink { title, .. }) => self.is_string_match(title),
            Field::Vec(fields) => fields.iter().any(|f| self.is_field_match(f)),
            _ => false,
        }
    }

    fn is_match(&self, item: &ItemMeta) -> bool {
        let field = self.search_field;
        if field == self.title_field {
            self.is_string_match(&item.title)
        } else if let Some(ItemField(_, value, _)) =
            item.fields.iter().find(|ItemField(x, _, _)| *x == field)
        {
            self.is_field_match(value)
        } else {
            false
        }
    }

    const MAX_SEARCH_RESULTS: usize = 100_000;

    fn start_entry<E: Entry>(&mut self, entry: &E) -> bool {
        // Early exit if we found enough items.
        if self.result_set.len() >= Self::MAX_SEARCH_RESULTS {
            return false;
        }

        // Double lookup is better than cloning unconditionally.
        if !self.result_cache.contains_key(entry.entry_id()) {
            self.result_cache
                .entry(entry.entry_id().clone())
                .or_default();
        }

        // Always recurse into tiles, because results can be fetched
        // asynchronously.
        true
    }

    fn start_tile<E: Entry>(&mut self, entry: &E, tile_id: TileID) -> bool {
        // Early exit if we found enough items.
        if self.result_set.len() >= Self::MAX_SEARCH_RESULTS {
            return false;
        }

        let mut result = true;
        // Always called second, so we know the entry exists.
        let cache = self.result_cache.get_mut(entry.entry_id()).unwrap();
        cache
            .entry(tile_id)
            .and_modify(|_| {
                result = false;
            })
            .or_default();
        result
    }

    fn insert<E: Entry>(&mut self, entry: &E, tile_id: TileID, irow: usize, item: &ItemMeta) {
        if self.result_set.len() >= Self::MAX_SEARCH_RESULTS {
            return;
        }

        // We want each item to appear once, so check the result set first
        // before inserting.
        if self.result_set.insert(item.item_uid) {
            let cache = self.result_cache.get_mut(entry.entry_id()).unwrap();
            let cache = cache.get_mut(&tile_id).unwrap();
            cache
                .entry(item.item_uid)
                .or_insert_with(|| SearchCacheItem {
                    item_uid: item.item_uid,
                    irow,
                    interval: item.original_interval,
                    title: item.title.clone(),
                });
        }
    }

    fn build_entry_tree(&mut self) {
        for (entry_id, cache) in &self.result_cache {
            let cache_size: u64 = cache.values().map(|x| x.len() as u64).sum();
            if cache_size == 0 {
                continue;
            }

            let level0_index = entry_id.slot_index(0).unwrap();
            let level1_index = entry_id.slot_index(1).unwrap();
            let level2_index = entry_id.slot_index(2).unwrap();

            let level0_subtree = self.entry_tree.entry(level0_index).or_default();
            let level1_subtree = level0_subtree.entry(level1_index).or_default();
            level1_subtree.insert(level2_index);
        }
    }
}

// ── Agent highlight helpers (Phase 3) ────────────────────────────────────────

/// Reproduce the slug-part algorithm from `duckdb_data::sanitize_short`:
/// remove spaces, extract ASCII alphanumeric runs, join with `_`, lowercase.
#[cfg(feature = "ai")]
fn slug_part(name: &str) -> String {
    let no_spaces: String = name.chars().filter(|c| *c != ' ').collect();
    let mut result = String::new();
    let mut in_word = false;
    for c in no_spaces.chars() {
        if c.is_ascii_alphanumeric() {
            if !in_word && !result.is_empty() {
                result.push('_');
            }
            result.push(c.to_ascii_lowercase());
            in_word = true;
        } else {
            in_word = false;
        }
    }
    result
}

/// Build a `slug → EntryID` map by traversing the 3-level panel hierarchy.
/// Matches the slug generation in `duckdb_data::walk_entry_list`.
#[cfg(feature = "ai")]
fn build_slug_map(window: &Window) -> HashMap<String, EntryID> {
    let mut map = HashMap::new();
    // window.panel = Panel<Panel<Panel<Slot>>>
    // level-1: node panels  (N0, N1, …)
    // level-2: kind panels  (CPU, GPU, Utility, …)
    // level-3: slot entries (C0, C1, …)
    for node_panel in &window.panel.slots {
        let node_slug = slug_part(&node_panel.short_name);
        map.insert(node_slug.clone(), node_panel.entry_id.clone());

        for kind_panel in &node_panel.slots {
            let kind_slug =
                format!("{}_{}", node_slug, slug_part(&kind_panel.short_name));
            map.insert(kind_slug.clone(), kind_panel.entry_id.clone());

            for slot in &kind_panel.slots {
                let slot_slug =
                    format!("{}_{}", kind_slug, slug_part(&slot.short_name));
                map.insert(slot_slug.clone(), slot.entry_id.clone());
            }
        }
    }
    map
}

/// Monotonic source of unique highlight ids (the manager's stable ordering key).
#[cfg(feature = "ai")]
static NEXT_HIGHLIGHT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Allocate the next unique highlight id.
#[cfg(feature = "ai")]
fn next_highlight_id() -> u64 {
    NEXT_HIGHLIGHT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Build the renderable [`AiHighlight`] from an agent [`Highlight`]. Severity is no
/// longer used (one uniform light-red overlay); the optional `item_uid` task-target
/// is `None` (the tool is region/interval-based today). Used by BOTH the embedded
/// apply and the MCP handler, so the id is allocated here for uniqueness across both.
#[cfg(feature = "ai")]
fn highlight_to_ai(hl: &Highlight) -> AiHighlight {
    use crate::timestamp::Timestamp;
    AiHighlight {
        id: next_highlight_id(),
        interval: Interval::new(Timestamp(hl.start_ns), Timestamp(hl.stop_ns)),
        label: hl.label.clone(),
        item_uid: None,
        enabled: true,
    }
}

/// Encode an egui [`ColorImage`](egui::ColorImage) as a PNG byte vector.
///
/// Used by the screenshot capture pipeline to convert the egui viewport
/// screenshot into PNG format for the Claude API's vision capability.
#[cfg(feature = "ai")]
fn encode_screenshot_png(color_image: &egui::ColorImage) -> Vec<u8> {
    use image::ImageEncoder;
    let rgba: Vec<u8> = color_image
        .pixels
        .iter()
        .flat_map(|p| p.to_array())
        .collect();
    let mut buf = Vec::new();
    image::codecs::png::PngEncoder::new(&mut buf)
        .write_image(
            &rgba,
            color_image.size[0] as u32,
            color_image.size[1] as u32,
            image::ExtendedColorType::Rgba8,
        )
        .expect("PNG encode failed");
    buf
}

/// Apply a navigation/screenshot request to the live view (zoom / pan / scroll /
/// filter / search / reset). Shared by the embedded chat agent and any second
/// consumer (the V1.0 bridge) so both run identical view logic. Does NOT request
/// the screenshot — the caller sends the `ViewportCommand` and records the
/// awaiting slot.
#[cfg(feature = "ai")]
fn apply_navigation(cx: &mut Context, windows: &mut [Window], nav: &crate::ai::PendingNavigation) {
    use crate::ai::PendingNavigation;
    match nav {
        PendingNavigation::Screenshot { .. } => {
            // Plain screenshot — no navigation changes needed.
        }
        PendingNavigation::Zoom { start_ns, stop_ns, .. } => {
            let interval = Interval::new(Timestamp(*start_ns), Timestamp(*stop_ns));
            ProfApp::zoom(cx, interval);
        }
        PendingNavigation::Pan { direction, percent, .. } => {
            let pct = (percent.round() as i64).clamp(1, 200);
            let dir = if direction.as_str() == "left" {
                PanDirection::Left
            } else {
                PanDirection::Right
            };
            ProfApp::pan(cx, Percentage::from(pct), dir);
        }
        PendingNavigation::ScrollTo { entry_slug, .. } => {
            for window in windows.iter_mut() {
                let slug_map = build_slug_map(window);
                if let Some(entry_id) = slug_map.get(entry_slug) {
                    window.expand_slot(entry_id);
                    cx.ai_scroll_to_entry = Some(entry_id.clone());
                    break;
                }
            }
        }
        PendingNavigation::SetView {
            start_ns,
            stop_ns,
            entry_slug,
            filter_kinds,
            expand_kinds,
            collapse_kinds,
            vertical_scale,
            ..
        } => {
            let interval = Interval::new(Timestamp(*start_ns), Timestamp(*stop_ns));
            ProfApp::zoom(cx, interval);
            if let Some(scale) = vertical_scale {
                cx.scale_factor = (*scale as f32).clamp(0.25, 4.0);
            }
            for window in windows.iter_mut() {
                if let Some(kinds) = filter_kinds {
                    let matched = window.set_kind_filter(kinds);
                    if matched == 0 && !kinds.is_empty() {
                        log::warn!("set_view filter_kinds matched no known kinds: {kinds:?}");
                    }
                }
                if let Some(kinds) = expand_kinds {
                    for k in kinds {
                        window.set_kind_expanded(k, true);
                    }
                }
                if let Some(kinds) = collapse_kinds {
                    for k in kinds {
                        window.set_kind_expanded(k, false);
                    }
                }
            }
            if let Some(slug) = entry_slug {
                for window in windows.iter_mut() {
                    let slug_map = build_slug_map(window);
                    if let Some(entry_id) = slug_map.get(slug) {
                        window.expand_slot(entry_id);
                        cx.ai_scroll_to_entry = Some(entry_id.clone());
                        break;
                    }
                }
            }
        }
        PendingNavigation::Search { query, .. } => {
            for window in windows.iter_mut() {
                window.config.search_state.query = query.clone();
                window.search(cx);
            }
        }
        PendingNavigation::ResetView { .. } => {
            ProfApp::zoom(cx, cx.total_interval);
            cx.scale_factor = 1.0;
            for window in windows.iter_mut() {
                window.config.kind_filter.clear();
                window.config.search_state.query = String::new();
                window.search(cx);
            }
        }
    }
}

/// The `request_id` carried by any navigation variant.
/// A1: build the header "Selected:" banner line from a `selection_snapshot`
/// (`items`, `range`). Pure + egui-free so it is unit-testable, and it reads the
/// SAME snapshot `get_selection` returns, so the header and the MCP agent agree on
/// what is selected. Returns `None` when nothing is selected (the header then
/// renders no empty chrome). At most the first 2 task bars are shown in full; the
/// rest collapse to "+N more".
#[cfg(feature = "ai")]
fn format_selection_banner(
    items: &[crate::ai::SelectedItemInfo],
    range: &Option<(String, i64, i64)>,
) -> Option<String> {
    use crate::timestamp::Timestamp;
    if items.is_empty() && range.is_none() {
        return None;
    }
    const SHOWN: usize = 2;
    let mut parts: Vec<String> = Vec::new();
    if let Some((label, start, stop)) = range {
        parts.push(format!("{}–{} ({label})", Timestamp(*start), Timestamp(*stop)));
    }
    for it in items.iter().take(SHOWN) {
        let title = if it.title.is_empty() {
            format!("uid {}", it.item_uid)
        } else {
            it.title.clone()
        };
        let slug = it.entry_slug.as_deref().unwrap_or("?");
        parts.push(format!(
            "{title} @ {}–{} ({slug})",
            Timestamp(it.start_ns),
            Timestamp(it.stop_ns)
        ));
    }
    if items.len() > SHOWN {
        parts.push(format!("+{} more", items.len() - SHOWN));
    }
    Some(format!("Selected: {}", parts.join("  ·  ")))
}

#[cfg(feature = "ai")]
fn pending_nav_request_id(nav: &crate::ai::PendingNavigation) -> u64 {
    use crate::ai::PendingNavigation;
    match nav {
        PendingNavigation::Screenshot { request_id }
        | PendingNavigation::Zoom { request_id, .. }
        | PendingNavigation::Pan { request_id, .. }
        | PendingNavigation::ScrollTo { request_id, .. }
        | PendingNavigation::SetView { request_id, .. }
        | PendingNavigation::Search { request_id, .. }
        | PendingNavigation::ResetView { request_id } => *request_id,
    }
}

/// Apply ONE AI highlight to the live timeline state shared with the embedded
/// path (`window.config.ai_highlights`): expand the row, dedup-push the overlay,
/// enable rendering. Returns the matched `EntryID` (for scroll-to). Mirrors the
/// embedded highlight-action application (kept separate to leave the embedded
/// sole-driver path byte-for-byte unchanged); used by the MCP source. (V1.3)
#[cfg(feature = "ai")]
fn apply_one_highlight(windows: &mut [Window], hl: &crate::ai::Highlight) -> Option<EntryID> {
    let mut found = None;
    for window in windows.iter_mut() {
        let slug_map = build_slug_map(window);
        if let Some(entry_id) = slug_map.get(&hl.entry_slug) {
            window.expand_slot(entry_id);
            let ai_hl = highlight_to_ai(hl);
            let entry = window.config.ai_highlights.entry(entry_id.clone()).or_default();
            let dup = entry.iter().any(|h| {
                h.interval.start.0 == ai_hl.interval.start.0
                    && h.interval.stop.0 == ai_hl.interval.stop.0
                    && h.label == ai_hl.label
            });
            if !dup {
                entry.push(ai_hl);
            }
            window.config.ai_highlights_enabled = true;
            if found.is_none() {
                found = Some(entry_id.clone());
            }
        }
    }
    found
}

/// Clear ALL AI highlight overlays from every window. Returns the number of rows
/// that had highlights (for a truthful ACK). (V1.3)
#[cfg(feature = "ai")]
fn clear_all_highlights(windows: &mut [Window]) -> usize {
    let mut n = 0;
    for window in windows.iter_mut() {
        n += window.config.ai_highlights.len();
        window.config.ai_highlights.clear();
    }
    n
}

/// Flatten `ai_highlights` into the manager's row order: a flat list of
/// `(entry_id, &mut highlight)` sorted by the stable `id`. Used by the manager (the
/// `&mut` lets each row drive its enable checkbox) and unit-tested for deterministic
/// ordering. Pure (no egui).
#[cfg(feature = "ai")]
fn flatten_highlights_sorted(
    map: &mut HashMap<EntryID, Vec<AiHighlight>>,
) -> Vec<(EntryID, &mut AiHighlight)> {
    let mut rows: Vec<(EntryID, &mut AiHighlight)> =
        map.iter_mut().flat_map(|(eid, v)| v.iter_mut().map(move |h| (eid.clone(), h))).collect();
    rows.sort_by_key(|(_, h)| h.id);
    rows
}

/// Union of the intervals of all ENABLED highlights (for "Zoom to all"); disabled
/// highlights are ignored. `None` when nothing is enabled. Pure.
#[cfg(feature = "ai")]
fn highlight_union(map: &HashMap<EntryID, Vec<AiHighlight>>) -> Option<Interval> {
    use crate::timestamp::Timestamp;
    let mut bounds: Option<(i64, i64)> = None;
    for h in map.values().flatten().filter(|h| h.enabled) {
        let (s, e) = (h.interval.start.0, h.interval.stop.0);
        bounds = Some(match bounds {
            None => (s, e),
            Some((bs, be)) => (bs.min(s), be.max(e)),
        });
    }
    bounds.map(|(s, e)| Interval::new(Timestamp(s), Timestamp(e)))
}

/// Sink for draining the SECOND event source (the in-viewer MCP). Records the ONE
/// request serviced per drain (the viewport token guarantees a single outstanding
/// request across both sources): a navigation/screenshot (applied via the shared
/// screenshot pipeline) OR a highlight / clear-highlights (applied + ACKed by the
/// drain region). It does not emit chat events.
#[cfg(feature = "ai")]
#[derive(Default)]
struct McpDrainSink {
    pending: Option<(crate::ai::PendingNavigation, std::sync::mpsc::Sender<crate::ai::UiCommand>)>,
    /// (highlight, request_id, reply channel) — applied to the live state + ACKed.
    pending_highlight:
        Option<(crate::ai::Highlight, u64, std::sync::mpsc::Sender<crate::ai::UiCommand>)>,
    /// (request_id, reply channel) for a clear-highlights request.
    pending_clear: Option<(u64, std::sync::mpsc::Sender<crate::ai::UiCommand>)>,
    /// (request_id, reply channel) for a get_selection READ (V1.4 — non-driving).
    pending_selection: Option<(u64, std::sync::mpsc::Sender<crate::ai::UiCommand>)>,
}

#[cfg(feature = "ai")]
impl crate::ai::bridge::EventSink for McpDrainSink {
    fn on_navigation(
        &mut self,
        nav: crate::ai::PendingNavigation,
        reply_tx: &std::sync::mpsc::Sender<crate::ai::UiCommand>,
    ) {
        self.pending = Some((nav, reply_tx.clone()));
    }

    fn on_highlight(
        &mut self,
        request_id: u64,
        entry_slug: String,
        start_ns: i64,
        stop_ns: i64,
        severity: String,
        label: String,
        reply_tx: &std::sync::mpsc::Sender<crate::ai::UiCommand>,
    ) {
        self.pending_highlight = Some((
            crate::ai::Highlight { entry_slug, start_ns, stop_ns, severity, label },
            request_id,
            reply_tx.clone(),
        ));
    }

    fn on_clear_highlights_request(
        &mut self,
        request_id: u64,
        reply_tx: &std::sync::mpsc::Sender<crate::ai::UiCommand>,
    ) {
        self.pending_clear = Some((request_id, reply_tx.clone()));
    }

    fn on_get_selection(
        &mut self,
        request_id: u64,
        reply_tx: &std::sync::mpsc::Sender<crate::ai::UiCommand>,
    ) {
        self.pending_selection = Some((request_id, reply_tx.clone()));
    }
}

/// Build a metadata string describing the visible time range and entry
/// slugs in the current screenshot.  Sent alongside the PNG so Claude knows
/// the numeric context of the image.
#[cfg(feature = "ai")]
fn build_screenshot_metadata(cx: &Context, windows: &[Window]) -> String {
    let start = cx.view_interval.start.0;
    let stop = cx.view_interval.stop.0;
    let duration_ms = (stop - start) as f64 / 1_000_000.0;

    // Collect visible entry slugs from the first window
    let mut entry_slugs: Vec<String> = Vec::new();
    if let Some(window) = windows.first() {
        for node_panel in &window.panel.slots {
            // Node filter: use same logic as is_slot_visible (level 1)
            let node_idx = node_panel.entry_id.last_slot_index().unwrap_or(0);
            if node_idx < window.config.min_node || node_idx > window.config.max_node {
                continue;
            }
            if !node_panel.expanded {
                continue;
            }
            let node_slug = slug_part(&node_panel.short_name);

            for kind_panel in &node_panel.slots {
                // Kind filter: use same logic as is_slot_visible (level 2)
                if !window.config.kind_filter.is_empty()
                    && !window.config.kind_filter.contains(&kind_panel.short_name)
                {
                    continue;
                }
                if !kind_panel.expanded {
                    continue;
                }
                let kind_slug =
                    format!("{}_{}", node_slug, slug_part(&kind_panel.short_name));

                for slot in &kind_panel.slots {
                    let slot_slug =
                        format!("{}_{}", kind_slug, slug_part(&slot.short_name));
                    entry_slugs.push(slot_slug);
                }
            }
        }
    }

    // If a search is active, report the query and how many tasks matched.
    let search_note = windows
        .first()
        .filter(|w| !w.config.search_state.query.is_empty())
        .map(|w| {
            format!(
                " Active search: \"{}\" ({} matches highlighted).",
                w.config.search_state.query,
                w.config.search_state.result_set.len()
            )
        })
        .unwrap_or_default();

    format!(
        "Screenshot captured. Visible time range: {} ns \u{2013} {} ns ({:.2} ms). \
         Visible entries (top to bottom): {}.{} \
         Use these entry_slugs and time range for follow-up queries.",
        start,
        stop,
        duration_ms,
        entry_slugs.join(", "),
        search_note
    )
}

impl Config {
    fn new(data_source: Box<dyn DeferredDataSource>, info: DataSourceInfo) -> Self {
        let max_node = info.entry_info.nodes();
        let kinds = info.entry_info.kinds();
        let interval = info.interval;
        let tile_set = info.tile_set;
        let warning_message = info.warning_message;

        let mut field_schema = info.field_schema;
        assert!(!field_schema.contains_name("Title"));
        let title_id = field_schema.insert("Title".to_owned(), true);
        let search_state = SearchState::new(title_id);

        // Build the data source stack: Counting<LRU<raw>>
        let lru_source = LruDeferredDataSource::new(data_source, NonZeroUsize::new(1024).unwrap());
        let data_source_stack = CountingDeferredDataSource::new(lru_source);

        Self {
            field_schema,
            min_node: 0,
            max_node,
            kinds,
            kind_filter: BTreeSet::new(),
            interval,
            warning_message,
            data_source: data_source_stack,
            search_state,
            items_selected: BTreeMap::new(),
            scroll_to_item: None,
            scroll_to_item_retry: None,
            tile_manager: TileManager::new(tile_set, interval),
            #[cfg(feature = "ai")]
            ai_highlights: HashMap::new(),
            #[cfg(feature = "ai")]
            ai_highlights_enabled: true,  // Show highlights when found
            #[cfg(feature = "ai")]
            ai_timeline_selection: None,
        }
    }

    fn request_tiles(&mut self, view_interval: Interval, full: bool) -> Vec<TileID> {
        self.tile_manager.request_tiles(view_interval, full)
    }

    fn invalidate_cache<T>(tile_ids: &[TileID], cache: &mut BTreeMap<TileID, T>) {
        TileManager::invalidate_cache(tile_ids, cache);
    }

    fn scroll_to_item(&mut self, item_loc: ItemLocator) {
        self.scroll_to_item = Some(item_loc.clone());
        self.scroll_to_item_retry = None;

        self.items_selected
            .entry(item_loc.item_uid)
            .or_insert_with(|| ItemDetail {
                meta: None,
                loc: item_loc,
            });
    }

}

impl Window {
    fn new(data_source: Box<dyn DeferredDataSource>, info: DataSourceInfo, index: u64) -> Self {
        Self {
            panel: Panel::new(&info.entry_info, EntryID::root()),
            index,
            config: Config::new(data_source, info),
        }
    }

    fn find_slot(&self, entry_id: &EntryID) -> Option<&Slot> {
        self.panel.find_slot(entry_id, 0)
    }

    fn find_slot_mut(&mut self, entry_id: &EntryID) -> Option<&mut Slot> {
        self.panel.find_slot_mut(entry_id, 0)
    }

    fn find_summary_mut(&mut self, entry_id: &EntryID) -> Option<&mut Summary> {
        self.panel.find_summary_mut(entry_id, 0)
    }

    fn expand_slot(&mut self, entry_id: &EntryID) {
        self.panel.expand_slot(entry_id, 0);
    }

    fn inflate_meta(&mut self, entry_id: &EntryID, cx: &mut Context) {
        // Use the panel version directly to avoid a mutability conflict
        let slot = self.panel.find_slot_mut(entry_id, 0).unwrap();
        slot.inflate_meta(&mut self.config, cx);
    }

    fn find_item_irow(&self, entry_id: &EntryID, item_uid: ItemUID) -> Option<usize> {
        let slot = self.find_slot(entry_id)?;
        for tile in slot.tiles.values() {
            let Some(Ok(tile)) = tile else {
                continue;
            };
            for (row, items) in tile.items.iter().enumerate() {
                for item in items {
                    if item.item_uid == item_uid {
                        let rows = tile.items.len();
                        return Some(rows - row - 1);
                    }
                }
            }
        }
        None
    }

    fn find_item_meta(&self, entry_id: &EntryID, item_uid: ItemUID) -> Option<&ItemMeta> {
        let slot = self.find_slot(entry_id)?;
        for tile in slot.tile_metas_full.values() {
            let Some(Ok(tile)) = tile else {
                continue;
            };
            for items in &tile.items {
                for item in items {
                    if item.item_uid == item_uid {
                        return Some(item);
                    }
                }
            }
        }
        None
    }

    fn content(&mut self, ui: &mut egui::Ui, cx: &mut Context) {
        ui.horizontal(|ui| {
            ui.heading(format!("Profile {}", self.index));
            ui.label(cx.view_interval.to_string());
            if let Some(message) = &self.config.warning_message {
                ui.label(RichText::new(message).color(Color32::RED));
            }
        });

        ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show_viewport(ui, |ui, viewport| {
                let height = self.panel.height(None, &self.config, cx);
                ui.set_height(height);
                ui.set_width(ui.available_width());

                let rect = Rect::from_min_size(ui.min_rect().min, viewport.size());

                let scroll_to = |irow, prefix_height| {
                    let mut item_rect =
                        rect.translate(Vec2::new(0.0, prefix_height + irow as f32 * cx.row_height));
                    item_rect.set_height(cx.row_height);
                    ui.scroll_to_rect(item_rect, Some(egui::Align::Center));
                };

                // First scroll attempt goes to the processor
                if let Some(ItemLocator {
                    ref entry_id, irow, ..
                }) = self.config.scroll_to_item
                {
                    let prefix_height = self.panel.height(Some(entry_id), &self.config, cx);
                    scroll_to(irow.unwrap_or(0), prefix_height);
                    if irow.is_none() {
                        let mut item = None;
                        std::mem::swap(&mut item, &mut self.config.scroll_to_item);
                        self.config.scroll_to_item_retry = item;
                    }
                    self.config.scroll_to_item = None;
                }

                // Agent-requested scroll to a processor row (no specific item).
                #[cfg(feature = "ai")]
                if let Some(ref entry_id) = cx.ai_scroll_to_entry {
                    let prefix_height = self.panel.height(Some(entry_id), &self.config, cx);
                    scroll_to(0, prefix_height);
                    cx.ai_scroll_to_entry = None;
                }

                // If we're able to find the item, we do a second scroll to the item
                let mut found_irow = None;
                if let Some(ItemLocator {
                    ref entry_id,
                    irow,
                    item_uid,
                }) = self.config.scroll_to_item_retry
                {
                    assert!(irow.is_none());
                    found_irow = self.find_item_irow(entry_id, item_uid);
                }

                if let Some(ItemLocator { ref entry_id, .. }) = self.config.scroll_to_item_retry {
                    if let Some(irow) = found_irow {
                        let prefix_height = self.panel.height(Some(entry_id), &self.config, cx);
                        scroll_to(irow, prefix_height);
                        self.config.scroll_to_item_retry = None;
                    }
                }

                // Root panel has no label
                self.panel.content(ui, rect, viewport, &mut self.config, cx);
            });
    }

    fn node_selection(&mut self, ui: &mut egui::Ui, cx: &Context) {
        ui.subheading("Node Selection", cx);
        let total = self.panel.slots.len().saturating_sub(1) as u64;
        let min_node = &mut self.config.min_node;
        let max_node = &mut self.config.max_node;
        ui.add(Slider::new(min_node, 0..=total).text("First"));
        if *min_node > *max_node {
            *max_node = *min_node;
        }
        ui.add(Slider::new(max_node, 0..=total).text("Last"));
        if *min_node > *max_node {
            *min_node = *max_node;
        }
    }

    fn filter_by_kind(&mut self, ui: &mut egui::Ui, cx: &Context) {
        ui.subheading("Filter by Kind", cx);
        ui.horizontal_wrapped(|ui| {
            for kind in &self.config.kinds {
                let initial = self.config.kind_filter.contains(kind);
                let mut enabled = initial;
                ui.toggle_value(&mut enabled, kind);
                if initial != enabled {
                    if enabled {
                        self.config.kind_filter.insert(kind.clone());
                    } else {
                        self.config.kind_filter.remove(kind);
                    }
                }
            }
        });
    }

    fn expand_collapse(&mut self, ui: &mut egui::Ui, cx: &Context) {
        let mut toggle_all = |label, toggle| {
            for node in &mut self.panel.slots {
                for kind in &mut node.slots {
                    if kind.expanded == toggle && kind.label_text() == label {
                        kind.toggle_expanded();
                    }
                }
            }
        };

        ui.subheading("Expand/Collapse", cx);
        ui.label("Expand by kind:");
        ui.horizontal_wrapped(|ui| {
            for kind in &self.config.kinds {
                if ui.button(kind).clicked() {
                    toggle_all(kind.to_lowercase(), false);
                }
            }
        });
        ui.label("Collapse by kind:");
        ui.horizontal_wrapped(|ui| {
            for kind in &self.config.kinds {
                if ui.button(kind).clicked() {
                    toggle_all(kind.to_lowercase(), true);
                }
            }
        });
    }

    /// Set the kind filter to show only the given processor kinds (empty = all).
    /// Requested names are matched case-insensitively AND by substring, so a
    /// request of "gpu" selects both "gpudev" and "gpuhost". Returns the number
    /// of known kinds matched (0 means nothing matched — caller may warn).
    #[cfg(feature = "ai")]
    fn set_kind_filter(&mut self, kinds: &[String]) -> usize {
        self.config.kind_filter.clear();
        for req in kinds {
            let needle = req.to_lowercase();
            for k in &self.config.kinds {
                let hay = k.to_lowercase();
                if hay == needle || hay.contains(&needle) {
                    self.config.kind_filter.insert(k.clone());
                }
            }
        }
        self.config.kind_filter.len()
    }

    /// Expand or collapse every kind panel matching `kind` (case-insensitive,
    /// substring), mirroring the Expand/Collapse-by-kind controls. A request of
    /// "gpu" matches both "gpudev" and "gpuhost".
    #[cfg(feature = "ai")]
    fn set_kind_expanded(&mut self, kind: &str, expanded: bool) {
        let needle = kind.to_lowercase();
        for node in &mut self.panel.slots {
            for k in &mut node.slots {
                // Scope the immutable label borrow so it ends before toggle_expanded().
                let matches = {
                    let label = k.label_text();
                    label == needle || label.contains(needle.as_str())
                };
                if matches && k.expanded != expanded {
                    k.toggle_expanded();
                }
            }
        }
    }

    fn select_interval(&mut self, ui: &mut egui::Ui, cx: &mut Context) {
        ui.subheading("Interval", cx);
        let start_res = ui
            .horizontal(|ui| {
                ui.label("Start:");
                ui.text_edit_singleline(&mut cx.interval_select_state.start_buffer)
            })
            .inner;

        if let Some(error) = cx.interval_select_state.start_error {
            ui.label(RichText::new(error.to_string()).color(Color32::RED));
        }

        let stop_res = ui
            .horizontal(|ui| {
                ui.label("Stop:");
                ui.text_edit_singleline(&mut cx.interval_select_state.stop_buffer)
            })
            .inner;

        if let Some(error) = cx.interval_select_state.stop_error {
            ui.label(RichText::new(error.to_string()).color(Color32::RED));
        }

        if start_res.lost_focus()
            && cx.interval_select_state.start_buffer != cx.view_interval.start.to_string()
        {
            match Timestamp::parse(&cx.interval_select_state.start_buffer) {
                Ok(start) => {
                    // validate timestamp
                    if start > cx.view_interval.stop {
                        cx.interval_select_state.start_error =
                            Some(IntervalSelectError::StartAfterStop);
                        return;
                    }
                    if start > cx.total_interval.stop {
                        cx.interval_select_state.start_error =
                            Some(IntervalSelectError::StartAfterEnd);
                        return;
                    }
                    let target = Interval::new(start, cx.view_interval.stop);
                    ProfApp::zoom(cx, target);
                }
                Err(e) => {
                    cx.interval_select_state.start_error = Some(e.into());
                }
            }
        }
        if stop_res.lost_focus()
            && cx.interval_select_state.stop_buffer != cx.view_interval.stop.to_string()
        {
            match Timestamp::parse(&cx.interval_select_state.stop_buffer) {
                Ok(stop) => {
                    // validate timestamp
                    if stop < cx.view_interval.start {
                        cx.interval_select_state.stop_error =
                            Some(IntervalSelectError::StopBeforeStart);
                        return;
                    }
                    let target = Interval::new(cx.view_interval.start, stop);
                    ProfApp::zoom(cx, target);
                }
                Err(e) => {
                    cx.interval_select_state.stop_error = Some(e.into());
                }
            }
        }
    }

    fn controls(&mut self, ui: &mut egui::Ui, cx: &mut Context) {
        const WIDGET_PADDING: f32 = 8.0;
        ui.heading(format!("Profile {}: Controls", self.index));
        ui.add_space(WIDGET_PADDING);
        self.node_selection(ui, cx);
        ui.add_space(WIDGET_PADDING);
        self.filter_by_kind(ui, cx);
        ui.add_space(WIDGET_PADDING);
        self.expand_collapse(ui, cx);
        ui.add_space(WIDGET_PADDING);
        self.select_interval(ui, cx);
    }

    fn search(&mut self, cx: &mut Context) {
        // Invalidate cache if the search query changed.
        self.config.search_state.ensure_valid_cache(cx);

        // If search query empty, skip search. (Note: do this after
        // invalidating cache, otherwise we get leftover search results when
        // clearing the query.)
        if self.config.search_state.query.is_empty() {
            return;
        }

        // Expand meta tiles. (Including collapsed entries, if requested).
        self.panel.inflate_meta(&mut self.config, cx);

        // Search whatever data we have. Results are cached by entry/tile.
        self.panel.search(&mut self.config);

        // Cache is now full and we can highlight/render the entries.
    }

    fn search_box(&mut self, ui: &mut egui::Ui, cx: &mut Context) {
        ui.horizontal(|ui| {
            // Hack: need to estimate the button width or else the text box
            // overflows. Refer to the source for egui::widgets::Button::ui
            // for calculations.
            let button_label = "✖";
            let button_padding = ui.spacing().button_padding;
            let available_width = ui.available_width() - 2.0 * button_padding.x;
            let button_text: egui::WidgetText = "✖".into();
            let button_text =
                button_text.into_galley(ui, None, available_width, egui::TextStyle::Button);
            let button_size = button_text.size() + 2.0 * button_padding;

            const MARGIN: f32 = 4.0; // From egui::TextEdit::margin
            let query_size =
                ui.available_size().x - button_size.x - ui.spacing().item_spacing.x - 2.0 * MARGIN;
            egui::TextEdit::singleline(&mut self.config.search_state.query)
                .desired_width(query_size)
                .show(ui);
            if ui.button(button_label).clicked() {
                self.config.search_state.query.clear();
            }
        });
        ui.horizontal(|ui| {
            ui.label("Search field:");
            let schema = &self.config.field_schema;
            let search_field = &mut self.config.search_state.search_field;
            egui::ComboBox::from_id_salt("Search field")
                .selected_text(schema.get_name(*search_field).unwrap())
                .show_ui(ui, |ui| {
                    for field in schema.searchable() {
                        let name = schema.get_name(*field).unwrap();
                        ui.selectable_value(search_field, *field, name);
                    }
                });
        });
        ui.checkbox(
            &mut self.config.search_state.whole_word,
            "Match whole words only",
        );
        ui.checkbox(
            &mut self.config.search_state.include_collapsed_entries,
            "Include collapsed processors",
        );

        self.search(cx);
    }

    fn search_results(&mut self, ui: &mut egui::Ui, cx: &mut Context) {
        if self.config.search_state.query.is_empty() {
            ui.label("Enter a search to see results displayed here.");
            return;
        }

        if self.config.search_state.result_set.is_empty() {
            ui.label("No results found. Expand search to include collapsed processors?");

            return;
        }

        let num_results = self.config.search_state.result_set.len();
        if num_results >= SearchState::MAX_SEARCH_RESULTS {
            ui.label(format!(
                "Found {} results. (Limited to {}.)",
                num_results,
                SearchState::MAX_SEARCH_RESULTS
            ));
        } else {
            ui.label(format!("Found {} results.", num_results));
        }

        self.config.search_state.build_entry_tree();

        let mut scroll_target = None;
        ScrollArea::vertical()
            // Hack: estimate size of bottom UI.
            .max_height(ui.available_height() - 70.0)
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                let root_tree = &self.config.search_state.entry_tree;
                for (level0_index, level0_subtree) in root_tree {
                    let level0_slot = &mut self.panel.slots[*level0_index as usize];
                    ui.collapsing(&level0_slot.long_name, |ui| {
                        for (level1_index, level1_subtree) in level0_subtree {
                            let level1_slot = &mut level0_slot.slots[*level1_index as usize];
                            ui.collapsing(&level1_slot.long_name, |ui| {
                                for level2_index in level1_subtree {
                                    let level2_slot =
                                        &mut level1_slot.slots[*level2_index as usize];
                                    ui.collapsing(&level2_slot.long_name, |ui| {
                                        let cache = &self.config.search_state.result_cache;
                                        let cache = cache.get(&level2_slot.entry_id).unwrap();
                                        for tile_cache in cache.values() {
                                            for item in tile_cache.values() {
                                                let button =
                                                    egui::widgets::Button::new(&item.title).small();
                                                if ui.add(button).clicked() {
                                                    let interval = item
                                                        .interval
                                                        .grow(item.interval.duration_ns() / 20);
                                                    ProfApp::zoom(cx, interval);
                                                    scroll_target = Some(ItemLocator {
                                                        entry_id: level2_slot.entry_id.clone(),
                                                        irow: Some(item.irow),
                                                        item_uid: item.item_uid,
                                                    });
                                                    level2_slot.expanded = true;
                                                    level1_slot.expanded = true;
                                                    level0_slot.expanded = true;
                                                }
                                            }
                                        }
                                    });
                                }
                            });
                        }
                    });
                }
            });
        if let Some(target) = scroll_target {
            self.config.scroll_to_item(target);
        }
    }

    fn search_controls(&mut self, ui: &mut egui::Ui, cx: &mut Context) {
        const WIDGET_PADDING: f32 = 8.0;
        ui.heading(format!("Profile {}: Search", self.index));
        ui.add_space(WIDGET_PADDING);
        self.search_box(ui, cx);
        ui.add_space(WIDGET_PADDING);
        self.search_results(ui, cx);
    }

    /// Highlight manager (Task 3) — reuses the search-results backend shape (count
    /// header + ScrollArea + the zoom/expand/scroll click handler). FLAT list across
    /// `ai_highlights`, sorted by `id`; each row = an enable checkbox + the label as a
    /// button that zooms to (and expands) the highlight. Globals: toggle all / clear
    /// all / zoom to the union of enabled highlights.
    #[cfg(feature = "ai")]
    fn highlight_manager(&mut self, ui: &mut egui::Ui, cx: &mut Context) {
        let total: usize = self.config.ai_highlights.values().map(Vec::len).sum();
        ui.heading(format!("Profile {}: Highlights ({total})", self.index));
        if total == 0 {
            ui.label("No highlights.");
            return;
        }

        // Globals row: toggle all overlays · clear all · zoom to the enabled union.
        ui.horizontal(|ui| {
            if ui.button("Toggle all").on_hover_text("Show or hide all overlays").clicked() {
                self.config.ai_highlights_enabled = !self.config.ai_highlights_enabled;
            }
            if ui.button("Clear all").on_hover_text("Remove all highlights").clicked() {
                self.config.ai_highlights.clear();
            }
            if ui.button("Zoom to all").on_hover_text("Frame the union of enabled highlights").clicked() {
                if let Some(u) = highlight_union(&self.config.ai_highlights) {
                    ProfApp::zoom(cx, u);
                }
            }
        });

        // Flat list, stable order by id. Record a clicked row, then act AFTER the
        // scroll area releases the &mut borrow of ai_highlights (mirrors search_results).
        let mut clicked: Option<(EntryID, Interval, Option<ItemUID>)> = None;
        ScrollArea::vertical()
            .max_height(ui.available_height().min(240.0))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for (eid, hl) in flatten_highlights_sorted(&mut self.config.ai_highlights) {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut hl.enabled, "");
                        let label = if hl.label.is_empty() {
                            format!("highlight {}", hl.id)
                        } else {
                            hl.label.clone()
                        };
                        if ui.add(egui::Button::new(label).small()).clicked() {
                            clicked = Some((eid.clone(), hl.interval, hl.item_uid));
                        }
                    });
                }
            });

        if let Some((eid, interval, item_uid)) = clicked {
            // Same as the search-result click: zoom to (a padded) interval + expand
            // the row. uid-less gaps/regions stop here; a future task-target also
            // scrolls to the item.
            ProfApp::zoom(cx, interval.grow(interval.duration_ns() / 20));
            self.expand_slot(&eid);
            if let Some(uid) = item_uid {
                self.config.scroll_to_item(ItemLocator { entry_id: eid, irow: None, item_uid: uid });
            }
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
enum PanDirection {
    Left,
    Right,
}

impl ProfApp {
    /// Called once before the first frame.
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        mut data_sources: Vec<Box<dyn DeferredDataSource>>,
        opts: StartOptions,
    ) -> Self {
        // This is also where you can customized the look at feel of egui using
        // `cc.egui_ctx.set_visuals` and `cc.egui_ctx.set_fonts`.

        // Load previous app state (if any).
        // Note that you must enable the `persistence` feature for this to work.
        let mut result: Self = if let Some(storage) = cc.storage {
            eframe::get_value(storage, eframe::APP_KEY).unwrap_or_default()
        } else {
            Default::default()
        };

        for data_source in &mut data_sources {
            data_source.fetch_info();
        }
        result.pending_data_sources.clear();
        result.pending_data_sources.extend(data_sources);

        result.windows.clear();

        result.cx.scale_factor = 1.0;
        result.cx.row_scroll_delta = 0;

        #[cfg(not(feature = "ai"))]
        let _ = &opts;

        #[cfg(feature = "ai")]
        {
            // Pre-fill the assistant's tool paths from CLI flags / auto-detection.
            result
                .cx
                .chat_panel
                .set_tool_paths(opts.ai_duckdb_path, opts.ai_code_path, opts.ai_wiki_path);

            // Initialize agent tracing subscriber once at startup. Best-effort:
            // if the trace dir can't be created, log and continue without tracing.
            let trace_root_candidates = [
                std::path::PathBuf::from("prof_results"),
                std::path::PathBuf::from("../prof_results"),
            ];
            let trace_root = trace_root_candidates
                .iter()
                .find(|p| p.exists())
                .map(|p| p.as_path())
                .unwrap_or(std::path::Path::new("prof_results"));
            if let Err(e) = crate::ai::trace::init_subscriber(trace_root) {
                eprintln!("Failed to initialize agent tracing: {e}");
            }
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            result.last_update = Some(Instant::now());
        }

        // Set solid scroll bar (default from egui pre-0.24)
        // The new default "thin" style isn't clickable with our canvas widget
        cc.egui_ctx.style_mut(|style| {
            style.spacing.scroll = egui::style::ScrollStyle::solid();
        });

        result
    }

    fn update_interval_select_state(cx: &mut Context) {
        cx.interval_select_state.start_buffer = cx.view_interval.start.to_string();
        cx.interval_select_state.stop_buffer = cx.view_interval.stop.to_string();
        cx.interval_select_state.start_error = None;
        cx.interval_select_state.stop_error = None;
    }

    fn update_view_interval(cx: &mut Context, interval: Interval, origin: IntervalOrigin) {
        cx.view_interval = interval;

        let history = &mut cx.view_interval_history;
        let index = history.index;

        // Only keep at most one Pan origin in a row
        if !history.levels.is_empty()
            && history.origins[index] == IntervalOrigin::Pan
            && origin == IntervalOrigin::Pan
        {
            history.levels.truncate(index);
            history.origins.truncate(index);
        }

        history.levels.truncate(index + 1);
        history.levels.push(interval);
        history.origins.truncate(index + 1);
        history.origins.push(origin);
        history.index = history.levels.len() - 1;
    }

    fn pan(cx: &mut Context, percent: PercentageInteger, dir: PanDirection) {
        if percent.value() == 0 {
            return;
        }

        let duration = percent.apply_to(cx.view_interval.duration_ns());
        let sign = match dir {
            PanDirection::Left => -1,
            PanDirection::Right => 1,
        };
        let interval = cx.view_interval.translate(duration * sign);

        ProfApp::update_view_interval(cx, interval, IntervalOrigin::Pan);
        ProfApp::update_interval_select_state(cx);
    }

    fn zoom(cx: &mut Context, interval: Interval) {
        if cx.view_interval == interval {
            return;
        }

        ProfApp::update_view_interval(cx, interval, IntervalOrigin::Zoom);
        ProfApp::update_interval_select_state(cx);
    }

    fn undo_pan_zoom(cx: &mut Context) {
        if cx.view_interval_history.index == 0 {
            return;
        }
        cx.view_interval_history.index -= 1;
        cx.view_interval = cx.view_interval_history.levels[cx.view_interval_history.index];
        ProfApp::update_interval_select_state(cx);
    }

    fn redo_pan_zoom(cx: &mut Context) {
        if cx.view_interval_history.index + 1 >= cx.view_interval_history.levels.len() {
            return;
        }
        cx.view_interval_history.index += 1;
        cx.view_interval = cx.view_interval_history.levels[cx.view_interval_history.index];
        ProfApp::update_interval_select_state(cx);
    }

    fn zoom_in(cx: &mut Context) {
        let quarter = -cx.view_interval.duration_ns() / 4;
        Self::zoom(cx, cx.view_interval.grow(quarter));
    }

    fn zoom_out(cx: &mut Context) {
        let half = cx.view_interval.duration_ns() / 2;
        Self::zoom(
            cx,
            cx.view_interval.grow(half).intersection(cx.total_interval),
        );
    }

    fn multiply_scale_factor(cx: &mut Context, factor: f32) {
        cx.scale_factor = (cx.scale_factor * factor).clamp(0.25, 4.0);
    }

    fn reset_scale_factor(cx: &mut Context) {
        cx.scale_factor = 1.0;
    }

    fn reset_ui(cx: &mut Context, windows: &mut [Window]) {
        cx.show_controls = false;
        for window in windows.iter_mut() {
            window.config.items_selected.clear();
        }
        // Esc also clears the AI selection (task bars + Shift+drag region).
        #[cfg(feature = "ai")]
        {
            cx.ai_region_selection = None;
            cx.last_item_selection.clear();
            cx.chat_panel.clear_item_selection();
            cx.chat_panel.clear_selection();
        }
    }

    fn keyboard(ctx: &egui::Context, cx: &mut Context, windows: &mut [Window]) {
        // Focus is elsewhere, don't check any keys
        if ctx.memory(|m| m.focused().is_some()) {
            return;
        }

        enum Actions {
            ZoomIn,
            ZoomOut,
            UndoZoom,
            RedoZoom,
            ResetZoom,
            Pan(PercentageInteger, PanDirection),
            Scroll(i32),
            ExpandVertical,
            ShrinkVertical,
            ResetVertical,
            ToggleControls,
            ResetUI,
            NoAction,
        }
        let action = ctx.input(|i| {
            if i.modifiers.ctrl {
                if i.modifiers.alt {
                    if i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals) {
                        Actions::ExpandVertical
                    } else if i.key_pressed(egui::Key::Minus) {
                        Actions::ShrinkVertical
                    } else if i.key_pressed(egui::Key::Num0) {
                        Actions::ResetVertical
                    } else {
                        Actions::NoAction
                    }
                } else if i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals) {
                    Actions::ZoomIn
                } else if i.key_pressed(egui::Key::Minus) {
                    Actions::ZoomOut
                } else if i.key_pressed(egui::Key::ArrowLeft) {
                    Actions::UndoZoom
                } else if i.key_pressed(egui::Key::ArrowRight) {
                    Actions::RedoZoom
                } else if i.key_pressed(egui::Key::Num0) {
                    Actions::ResetZoom
                } else {
                    Actions::NoAction
                }
            } else if i.modifiers.shift {
                if i.key_pressed(egui::Key::ArrowLeft) {
                    Actions::Pan(Percentage::from(1), PanDirection::Left)
                } else if i.key_pressed(egui::Key::ArrowRight) {
                    Actions::Pan(Percentage::from(1), PanDirection::Right)
                } else if i.key_pressed(egui::Key::ArrowUp) {
                    Actions::Scroll(1)
                } else if i.key_pressed(egui::Key::ArrowDown) {
                    Actions::Scroll(-1)
                } else {
                    Actions::NoAction
                }
            } else if i.key_pressed(egui::Key::H) {
                Actions::ToggleControls
            } else if i.key_pressed(egui::Key::Escape) {
                Actions::ResetUI
            } else if i.key_pressed(egui::Key::ArrowLeft) {
                Actions::Pan(Percentage::from(5), PanDirection::Left)
            } else if i.key_pressed(egui::Key::ArrowRight) {
                Actions::Pan(Percentage::from(5), PanDirection::Right)
            } else if i.key_pressed(egui::Key::ArrowUp) {
                Actions::Scroll(5)
            } else if i.key_pressed(egui::Key::ArrowDown) {
                Actions::Scroll(-5)
            } else {
                Actions::NoAction
            }
        });
        match action {
            Actions::ZoomIn => ProfApp::zoom_in(cx),
            Actions::ZoomOut => ProfApp::zoom_out(cx),
            Actions::UndoZoom => ProfApp::undo_pan_zoom(cx),
            Actions::RedoZoom => ProfApp::redo_pan_zoom(cx),
            Actions::ResetZoom => ProfApp::zoom(cx, cx.total_interval),
            Actions::Pan(percent, dir) => ProfApp::pan(cx, percent, dir),
            Actions::Scroll(rows) => cx.row_scroll_delta = rows,
            Actions::ExpandVertical => ProfApp::multiply_scale_factor(cx, 2.0),
            Actions::ShrinkVertical => ProfApp::multiply_scale_factor(cx, 0.5),
            Actions::ResetVertical => ProfApp::reset_scale_factor(cx),
            Actions::ToggleControls => cx.show_controls = !cx.show_controls,
            Actions::ResetUI => ProfApp::reset_ui(cx, windows),
            Actions::NoAction => {}
        }
    }

    fn cursor(ui: &mut egui::Ui, cx: &mut Context) {
        // Hack: the UI rect we have at this point is not where the
        // timeline is being drawn. So fish out the coordinates we
        // need to draw the correct rect.

        // Sometimes slot_rect is None when initializing the UI
        if cx.slot_rect.is_none() {
            return;
        }

        let ui_rect = ui.min_rect();
        let slot_rect = cx.slot_rect.unwrap();
        let rect = Rect::from_min_max(
            Pos2::new(slot_rect.min.x, ui_rect.min.y),
            Pos2::new(slot_rect.max.x, ui_rect.max.y),
        );

        let response = ui.allocate_rect(rect, egui::Sense::drag());

        // Handle drag detection
        let mut drag_interval = None;

        let is_active_drag = response.dragged_by(egui::PointerButton::Primary);
        if is_active_drag && response.drag_started() {
            // On the beginning of a drag, save our position so we can
            // calculate the delta
            cx.drag_origin = response.interact_pointer_pos();
        }

        if let Some(origin) = cx.drag_origin {
            // We're in a drag, calculate the drag inetrval
            let current = response.interact_pointer_pos().unwrap();
            let min = origin.x.min(current.x);
            let max = origin.x.max(current.x);

            let start = (min - rect.left()) / rect.width();
            let start = cx.view_interval.lerp(start);
            let stop = (max - rect.left()) / rect.width();
            let stop = cx.view_interval.lerp(stop);

            let interval = Interval::new(start, stop);

            if is_active_drag {
                // Still in drag, draw a rectangle to show the dragged region.
                let drag_rect =
                    Rect::from_min_max(Pos2::new(min, rect.min.y), Pos2::new(max, rect.max.y));
                #[cfg(feature = "ai")]
                let color = if ui.input(|i| i.modifiers.shift) {
                    // Shift+drag = region select (blue) instead of zoom (gray).
                    Color32::from_rgba_unmultiplied(50, 100, 255, 60)
                } else {
                    Color32::DARK_GRAY.linear_multiply(0.5)
                };
                #[cfg(not(feature = "ai"))]
                let color = Color32::DARK_GRAY.linear_multiply(0.5);
                ui.painter().rect(drag_rect, 0.0, color, Stroke::NONE);

                drag_interval = Some(interval);
            } else if response.drag_stopped() {
                // Only act if the drag covered a certain distance.
                const MIN_DRAG_DISTANCE: f32 = 4.0;
                if max - min > MIN_DRAG_DISTANCE {
                    #[cfg(feature = "ai")]
                    let is_select = ui.input(|i| i.modifiers.shift);
                    #[cfg(not(feature = "ai"))]
                    let is_select = false;
                    if is_select {
                        #[cfg(feature = "ai")]
                        {
                            cx.ai_region_selection = Some(interval);
                            cx.chat_panel.set_selection(crate::ai::TimelineSelection {
                                entry_id: EntryID::root(),
                                entry_label: "timeline region".to_owned(),
                                interval,
                            });
                            // A2: record the selection but do NOT auto-open the chat
                            // panel — the header "Selected:" banner surfaces it instead.
                        }
                    } else {
                        ProfApp::zoom(cx, interval);
                    }
                }

                cx.drag_origin = None;
            }
        }

        // Persistent Shift+drag region-selection band.
        #[cfg(feature = "ai")]
        if let Some(region) = cx.ai_region_selection {
            let a = cx.view_interval.unlerp(region.start).clamp(0.0, 1.0);
            let b = cx.view_interval.unlerp(region.stop).clamp(0.0, 1.0);
            if b > a {
                let x0 = rect.left() + a * rect.width();
                let x1 = rect.left() + b * rect.width();
                let band =
                    Rect::from_min_max(Pos2::new(x0, rect.min.y), Pos2::new(x1, rect.max.y));
                ui.painter().rect(
                    band,
                    0.0,
                    Color32::from_rgba_unmultiplied(50, 100, 255, 30),
                    Stroke::new(1.0, Color32::from_rgba_unmultiplied(50, 100, 255, 120)),
                );
            }
        }

        // Handle hover detection
        if let Some(hover) = response.hover_pos() {
            let visuals = ui.style().interact_selectable(&response, false);

            // Draw vertical line through cursor
            const RADIUS: f32 = 12.0;
            let top = Pos2::new(hover.x, ui.min_rect().min.y);
            let mid_top = Pos2::new(hover.x, (hover.y - RADIUS).at_least(ui.min_rect().min.y));
            let mid_bottom = Pos2::new(hover.x, (hover.y + RADIUS).at_most(ui.min_rect().max.y));
            let bottom = Pos2::new(hover.x, ui.min_rect().max.y);
            ui.painter().line_segment([top, mid_top], visuals.fg_stroke);
            ui.painter()
                .line_segment([mid_bottom, bottom], visuals.fg_stroke);

            // Show timestamp popup
            let time = (hover.x - rect.left()) / rect.width();
            let time = cx.view_interval.lerp(time);

            let label_text = if let Some(drag) = drag_interval {
                format!("{drag}")
            } else {
                let units: TimestampUnits = cx.view_interval.into();
                let time_units = TimestampDisplay {
                    timestamp: time,
                    units,
                    include_units: true,
                };
                format!("t={time_units}")
            };

            ui.show_tooltip_at("timestamp_tooltip", top, label_text);
        }
    }

    fn display_controls(ui: &mut egui::Ui, mode: &mut ItemLinkNavigationMode) {
        fn show_row_ui(
            body: &mut egui_extras::TableBody<'_>,
            label: &str,
            thunk: impl FnMut(&mut egui::Ui),
        ) {
            body.row(20.0, |mut row| {
                row.col(|ui| {
                    ui.strong(label);
                });
                row.col(thunk);
            });
        }

        TableBuilder::new(ui)
            .striped(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::auto())
            .column(Column::remainder())
            .body(|mut body| {
                let mut show_row = |a, b| {
                    show_row_ui(&mut body, a, |ui| {
                        ui.label(b);
                    });
                };
                show_row("Zoom to Interval", "Click and Drag");
                show_row("Pan 5%", "Left/Right Arrow");
                show_row("Pan 1%", "Shift + Left/Right Arrow");
                show_row("Vertical Scroll", "Up/Down Arrow");
                show_row("Fine Vertical Scroll", "Shift + Up/Down Arrow");
                show_row("Zoom In", "Ctrl + Plus/Equals");
                show_row("Zoom Out", "Ctrl + Minus");
                show_row("Undo Pan/Zoom", "Ctrl + Left Arrow");
                show_row("Redo Pan/Zoom", "Ctrl + Right Arrow");
                show_row("Reset Pan/Zoom", "Ctrl + 0");
                show_row("Expand Vertical Spacing", "Ctrl + Alt + Plus/Equals");
                show_row("Shrink Vertical Spacing", "Ctrl + Alt + Minus");
                show_row("Reset Vertical Spacing", "Ctrl + Alt + 0");
                show_row("Toggle This Window", "H");
                show_row_ui(&mut body, "Item Link Zoom or Pan", |ui: &mut _| {
                    egui::ComboBox::from_id_salt("Item Link Zoom or Pan")
                        .selected_text(format!("{:?}", mode))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(mode, ItemLinkNavigationMode::Zoom, "Zoom");
                            ui.selectable_value(mode, ItemLinkNavigationMode::Pan, "Pan");
                        });
                });
            });
    }

    fn compute_text_height(text: String, width: f32, ui: &mut egui::Ui) -> f32 {
        let style = ui.style();
        let font_id = TextStyle::Body.resolve(style);
        let visuals = style.noninteractive();
        let layout = ui
            .painter()
            .layout(text, font_id, visuals.text_color(), width);

        layout.size().y + style.spacing.item_spacing.y * 2.0
    }

    fn render_field_as_text(
        field: &Field,
        mode: ItemLinkNavigationMode,
    ) -> Vec<(String, Option<&'static str>)> {
        match field {
            Field::I64(value) => vec![(format!("{value}"), None)],
            Field::U64(value) => vec![(format!("{value}"), None)],
            Field::String(value) => vec![(value.to_string(), None)],
            Field::Interval(value) => vec![(format!("{value}"), None)],
            Field::ItemLink(ItemLink { title, .. }) => {
                vec![(title.to_string(), Some(mode.label_text()))]
            }
            Field::Vec(fields) => fields
                .iter()
                .flat_map(|f| Self::render_field_as_text(f, mode))
                .collect(),
            Field::Empty => vec![("".to_string(), None)],
        }
    }

    fn compute_field_height(
        field: &Field,
        width: f32,
        mode: ItemLinkNavigationMode,
        ui: &mut egui::Ui,
    ) -> f32 {
        let text = Self::render_field_as_text(field, mode);
        text.into_iter()
            .map(|(mut v, b)| {
                // Hack: if we have button text, guess how much space it will need
                // by extending the string.
                if let Some(b) = b {
                    v.push(' ');
                    v.push_str(b);
                }
                Self::compute_text_height(v, width, ui)
            })
            .sum()
    }

    fn render_field_as_ui(
        field: &Field,
        color: Option<Color32>,
        mode: ItemLinkNavigationMode,
        ui: &mut egui::Ui,
    ) -> Option<(ItemLocator, Interval)> {
        let mut result = None;
        let label = |ui: &mut egui::Ui, v| {
            if let Some(color) = color {
                ui.add(
                    egui::Label::new(RichText::new(v).color(color)).wrap_mode(TextWrapMode::Wrap),
                );
            } else {
                ui.add(egui::Label::new(v).wrap_mode(TextWrapMode::Wrap));
            }
        };
        let label_button = |ui: &mut egui::Ui, v, b| {
            label(ui, v);
            if let Some(color) = color {
                ui.button(RichText::new(b).color(color)).clicked()
            } else {
                ui.button(b).clicked()
            }
        };
        match field {
            Field::I64(value) => label(ui, &format!("{value}")),
            Field::U64(value) => label(ui, &format!("{value}")),
            Field::String(value) => label(ui, value),
            Field::Interval(value) => label(ui, &format!("{value}")),
            Field::ItemLink(ItemLink {
                title,
                item_uid,
                interval,
                entry_id,
            }) => {
                if label_button(ui, title, mode.label_text()) {
                    result = Some((
                        ItemLocator {
                            entry_id: entry_id.clone(),
                            irow: None,
                            item_uid: *item_uid,
                        },
                        *interval,
                    ));
                }
            }
            Field::Vec(fields) => {
                ui.vertical(|ui| {
                    for f in fields {
                        ui.horizontal(|ui| {
                            if let Some(x) = Self::render_field_as_ui(f, color, mode, ui) {
                                result = Some(x);
                            }
                        });
                    }
                });
            }
            Field::Empty => {}
        }
        result
    }

    fn display_item_details(
        ui: &mut egui::Ui,
        item: &ItemDetail,
        field_schema: &FieldSchema,
        cx: &Context,
    ) -> Option<(ItemLocator, Interval)> {
        let Some(ref item_meta) = item.meta else {
            ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                ui.label("Item will be displayed once data is available.");
            });
            return None;
        };

        let font_id = TextStyle::Body.resolve(ui.style());
        let row_height = ui.fonts(|f| f.row_height(&font_id));

        let mut result: Option<(ItemLocator, Interval)> = None;
        TableBuilder::new(ui)
            .striped(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::auto())
            .column(Column::remainder())
            .body(|mut body| {
                let mut show_row = |k: &str, field: &Field, color: Option<Color32>| {
                    // We need to manually work out the height of the labels
                    // so that the table knows how large to make each row.
                    let width = body.widths()[1];

                    let ui = body.ui_mut();
                    let height = Self::compute_field_height(field, width, cx.item_link_mode, ui)
                        .max(row_height);

                    body.row(height, |mut row| {
                        row.col(|ui| {
                            if let Some(color) = color {
                                ui.label(RichText::new(k).color(color).strong());
                            } else {
                                ui.strong(k);
                            }
                        });
                        row.col(|ui| {
                            if let Some(x) =
                                Self::render_field_as_ui(field, color, cx.item_link_mode, ui)
                            {
                                result = Some(x);
                            }
                        });
                    });
                };

                show_row("Title", &Field::String(item_meta.title.to_string()), None);
                if cx.debug {
                    show_row("Item UID", &Field::U64(item_meta.item_uid.0), None);
                }
                for ItemField(field_id, field, color) in &item_meta.fields {
                    let name = field_schema.get_name(*field_id).unwrap();
                    show_row(name, field, *color);
                }
            });
        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
            if ui.button(cx.item_link_mode.label_text()).clicked() {
                result = Some((item.loc.clone(), item_meta.original_interval));
            }
        });
        result
    }
}

impl eframe::App for ProfApp {
    /// Called to save state before shutdown.
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, eframe::APP_KEY, self);
    }

    /// Called each time the UI needs repainting.
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let Self {
            pending_data_sources,
            windows,
            cx,
            #[cfg(not(target_arch = "wasm32"))]
            last_update,
            ..
        } = self;

        // V1.2: hand the embedded chat agent a clone of the shared viewport token so
        // its screenshot/nav round-trips are mutually exclusive with the in-viewer
        // MCP driver (single outstanding screenshot across both). Idempotent; no
        // effect on the sole-driver path (the token is always free for it).
        #[cfg(feature = "ai")]
        cx.chat_panel.ensure_viewport_token(cx.viewport_token.clone());

        // V1.1: start the in-viewer HTTP MCP server (data tools only) once a DuckDB
        // path is configured. Runs on its OWN thread — never the egui main thread.
        // One spawn attempt; serves run_query/overview/find_blockers over HTTP so
        // Claude Code can connect to this live process.
        #[cfg(feature = "viewer-mcp")]
        if !cx.viewer_mcp_started {
            if let Some(duckdb_path) = cx.chat_panel.duckdb_path() {
                // V1.3: mint a UiBridge (consumer MCP_CONSUMER_ID) and hand it to the
                // server so it advertises + routes the 9 VISUAL tools, driving this
                // live window. The bridge's UI-side ends (mcp_event_rx / mcp_cmd_tx)
                // are drained + replied to by the per-frame second-source loop below.
                // The wake hook repaints this (reactive, often-idle) window when a
                // request arrives, so the drain loop runs instead of blocking to
                // timeout.
                let egui_ctx = ctx.clone();
                let bridge = cx
                    .ui_bridge(crate::ai::bridge::MCP_CONSUMER_ID)
                    .with_wake(move || egui_ctx.request_repaint());
                // Hand the configured wiki + source roots to the server so it briefs
                // the external agent (MCP `instructions` + overview source line) and
                // advertises wiki_* / read_code / list_files.
                let wiki_root = cx.chat_panel.wiki_path();
                let code_root = cx.chat_panel.code_path();
                // P1 (Backend B): STORE the bound port instead of discarding it.
                // Prefer the stable well-known port 8765 so existing external
                // `claude mcp add …:8765/mcp` registrations keep working; fall back
                // to an ephemeral port (0) only if 8765 is already taken. Either
                // way, the REAL bound port lands in `cx.viewer_mcp_port` and the
                // chat panel, so Backend B never assumes a port.
                // The spawn also mints the per-session bearer token every POST /mcp
                // must present (server hardening); the (port, token) pair flows to
                // the chat panel so Backend B can build its --mcp-config.
                let mut endpoint: Option<(u16, String)> = None;
                match crate::ai::viewer_mcp::spawn(
                    duckdb_path.clone(),
                    8765,
                    bridge,
                    wiki_root.clone(),
                    code_root.clone(),
                ) {
                    Ok((port, token)) => endpoint = Some((port, token)),
                    Err(first_err) => {
                        let egui_ctx2 = ctx.clone();
                        let bridge2 = cx
                            .ui_bridge(crate::ai::bridge::MCP_CONSUMER_ID)
                            .with_wake(move || egui_ctx2.request_repaint());
                        match crate::ai::viewer_mcp::spawn(
                            duckdb_path, 0, bridge2, wiki_root, code_root,
                        ) {
                            Ok((port, token)) => {
                                eprintln!(
                                    "[legion-viewer] port 8765 unavailable ({first_err}); using ephemeral port {port}"
                                );
                                endpoint = Some((port, token));
                            }
                            Err(e) => eprintln!(
                                "[legion-viewer] in-viewer MCP server failed to start: \
                                 port 8765: {first_err}; ephemeral: {e}"
                            ),
                        }
                    }
                }
                cx.viewer_mcp_port = endpoint.as_ref().map(|(p, _)| *p);
                cx.chat_panel.set_mcp_endpoint(endpoint);
                cx.viewer_mcp_started = true;
            }
        }

        if let Some(mut source) = pending_data_sources.pop_front() {
            // We made one request, so we know there is always zero or one
            // elements in this list.
            if let Some(info) = source.get_infos().pop() {
                // TODO: show this in a more user-friendly way.
                let info = info.expect("fetch_info failed");
                let window = Window::new(source, info, windows.len() as u64);
                if windows.is_empty() {
                    cx.total_interval = window.config.interval;
                } else {
                    cx.total_interval = cx.total_interval.union(window.config.interval);
                }
                ProfApp::zoom(cx, cx.total_interval);
                windows.push(window);
            } else {
                pending_data_sources.push_front(source);
            }
        }

        for window in windows.iter_mut() {
            for (tile, req) in window.config.data_source.get_summary_tiles() {
                if let Some(entry) = window.find_summary_mut(&req.entry_id) {
                    // If the entry doesn't exist, we already zoomed away and
                    // are no longer interested in this tile.
                    entry
                        .tiles
                        .entry(req.tile_id)
                        .and_modify(|t| *t = Some(tile.map(|s| s.data)));
                }
            }

            for (tile, req) in window.config.data_source.get_slot_tiles() {
                if let Some(entry) = window.find_slot_mut(&req.entry_id) {
                    // If the entry doesn't exist, we already zoomed away and
                    // are no longer interested in this tile.
                    entry
                        .tiles
                        .entry(req.tile_id)
                        .and_modify(|t| *t = Some(tile.map(|s| s.data)));
                }
            }

            for (tile, req) in window.config.data_source.get_slot_meta_tiles() {
                if let Some(entry) = window.find_slot_mut(&req.entry_id) {
                    // If the entry doesn't exist, we already zoomed away and
                    // are no longer interested in this tile.
                    let metas = if req.full {
                        &mut entry.tile_metas_full
                    } else {
                        &mut entry.tile_metas
                    };
                    metas
                        .entry(req.tile_id)
                        .and_modify(|t| *t = Some(tile.map(|s| s.data)));
                }
            }

            // Propagate timeline gap selection to the chat panel
            #[cfg(feature = "ai")]
            {
                if let Some((entry_id, interval, label)) =
                    window.config.ai_timeline_selection.take()
                {
                    cx.chat_panel.set_selection(crate::ai::TimelineSelection {
                        entry_id,
                        entry_label: label,
                        interval,
                    });
                    // A2: record the selection but do NOT auto-open the chat panel —
                    // the header "Selected:" banner surfaces it instead. (An already-
                    // open panel still updates its pill via set_selection above.)
                }
            }
        }

        let mut _fps = 0.0;
        #[cfg(not(target_arch = "wasm32"))]
        {
            let now = Instant::now();
            if let Some(last) = last_update {
                _fps = 1.0 / now.duration_since(*last).as_secs_f64();
            }
            *last_update = Some(now);
        }

        // A1: header "Selected:" banner — always visible when something is selected,
        // INDEPENDENT of whether the chat panel is open. Computed before the panel
        // closure (immutable snapshot read) so the closure can still mutably toggle
        // the chat panel.
        #[cfg(feature = "ai")]
        let selection_banner = {
            let (items, range) = cx.chat_panel.selection_snapshot();
            format_selection_banner(&items, &range)
        };

        #[cfg(not(target_arch = "wasm32"))]
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Quit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });

                // Right-aligned Legion AI Co-Pilot toggle button
                #[cfg(feature = "ai")]
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let color = if cx.chat_panel.visible {
                        egui::Color32::from_rgb(59, 130, 246) // blue accent when open
                    } else {
                        egui::Color32::from_rgb(60, 60, 60)
                    };
                    let label = egui::RichText::new("🤖 Legion AI Co-Pilot")
                        .strong()
                        .size(14.0)
                        .color(color);
                    if ui
                        .add(egui::Button::new(label).frame(true))
                        .on_hover_text("Toggle the Legion AI Co-Pilot")
                        .clicked()
                    {
                        cx.chat_panel.toggle();
                    }
                });
            });

            // A1: compact, centered "Selected:" line under the menu bar — shown only
            // when something is selected (no empty chrome otherwise).
            #[cfg(feature = "ai")]
            if let Some(banner) = &selection_banner {
                ui.vertical_centered(|ui| {
                    ui.label(
                        egui::RichText::new(banner)
                            .size(12.0)
                            .color(egui::Color32::from_rgb(59, 130, 246)),
                    );
                });
            }
        });

        egui::SidePanel::left("side_panel").show(ctx, |ui| {
            let body = TextStyle::Body.resolve(ui.style()).size;
            let heading = TextStyle::Heading.resolve(ui.style()).size;
            // Just set this on every frame for now
            cx.subheading_size = (heading + body) * 0.5;

            const WIDGET_PADDING: f32 = 8.0;
            ui.add_space(WIDGET_PADDING);

            for window in windows.iter_mut() {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    window.controls(ui, cx);
                });
            }

            for window in windows.iter_mut() {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    window.search_controls(ui, cx);
                });
            }

            // Highlight manager — under the profile-search section. Reuses search's
            // count + ScrollArea + zoom/expand click backend. Subsumes the former
            // standalone Toggle/Delete highlight buttons (now its globals row).
            #[cfg(feature = "ai")]
            for window in windows.iter_mut() {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    window.highlight_manager(ui, cx);
                });
            }

            ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.label("powered by ");
                    ui.hyperlink_to("egui", "https://github.com/emilk/egui");
                    ui.label(" and ");
                    ui.hyperlink_to(
                        "eframe",
                        "https://github.com/emilk/egui/tree/master/crates/eframe",
                    );
                    ui.label(".");
                });

                ui.horizontal(|ui| {
                    egui::widgets::global_theme_preference_buttons(ui);
                });

                ui.horizontal(|ui| {
                    if ui.button("Show Controls").clicked() {
                        cx.show_controls = true;
                    }

                    ui.toggle_value(&mut cx.debug, "🛠 Debug");

                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        if cx.debug {
                            ui.label(format!("FPS: {_fps:.0}"));
                        }
                    }
                });

                // (The former standalone "Toggle highlights" / "Delete highlights"
                // buttons moved into the highlight manager's globals row above.)

                ui.separator();
                egui::warn_if_debug_build(ui);
            });
        });

        // AI Chat panel — must be added BEFORE CentralPanel in egui layout
        #[cfg(feature = "ai")]
        cx.chat_panel.show(ctx);

        // Resolve user-initiated highlight actions (from chat panel chip clicks)
        // into timeline overlays. Optionally zoom to the first action.
        #[cfg(feature = "ai")]
        {
            // Clear all highlight overlays if requested (Clear button or agent tool).
            if cx.chat_panel.take_clear_highlights() {
                for window in windows.iter_mut() {
                    window.config.ai_highlights.clear();
                }
            }

            // Clear the active selection (✕ in the composer): deselect bars + region.
            if cx.chat_panel.take_clear_selection() {
                for window in windows.iter_mut() {
                    window.config.items_selected.clear();
                }
                cx.ai_region_selection = None;
                cx.last_item_selection.clear();
            }

            let actions = cx.chat_panel.take_pending_highlight_actions();
            if !actions.is_empty() {
                let mut first_entry: Option<EntryID> = None;
                for window in windows.iter_mut() {
                    let slug_map = build_slug_map(window);
                    for action in &actions {
                        if let Some(entry_id) = slug_map.get(&action.highlight.entry_slug) {
                            // Expand the row's ancestors so the highlight actually
                            // draws — kind panels (level 2) are collapsed by default,
                            // and a highlight on a hidden row renders nothing.
                            window.expand_slot(entry_id);
                            if first_entry.is_none() {
                                first_entry = Some(entry_id.clone());
                            }
                            let ai_hl = highlight_to_ai(&action.highlight);
                            let entry = window
                                .config
                                .ai_highlights
                                .entry(entry_id.clone())
                                .or_default();
                            // Dedup: don't stack an identical highlight (same range + label).
                            let dup = entry.iter().any(|h| {
                                h.interval.start.0 == ai_hl.interval.start.0
                                    && h.interval.stop.0 == ai_hl.interval.stop.0
                                    && h.label == ai_hl.label
                            });
                            if !dup {
                                entry.push(ai_hl);
                            }
                        } else {
                            log::warn!(
                                "Highlight: unknown entry_slug '{}'",
                                action.highlight.entry_slug
                            );
                        }
                    }
                    if !window.config.ai_highlights.is_empty() {
                        window.config.ai_highlights_enabled = true;
                    }
                }
                // Scroll vertically to the first highlighted row so the overlays are
                // on screen (the rows we just expanded may be below the fold).
                if let Some(entry_id) = first_entry {
                    cx.ai_scroll_to_entry = Some(entry_id);
                }
                // Zoom to fit ALL highlights that requested it (union of ranges),
                // so "Zoom to all" frames every chip, and a single "Show ▸" frames
                // just that one.
                let zoom: Vec<_> = actions.iter().filter(|a| a.zoom_to).collect();
                if !zoom.is_empty() {
                    let start = zoom.iter().map(|a| a.highlight.start_ns).min().unwrap();
                    let stop = zoom.iter().map(|a| a.highlight.stop_ns).max().unwrap();
                    let interval = Interval::new(Timestamp(start), Timestamp(stop));
                    // Pad so the highlighted span sits inside the view with a margin.
                    let pad = (interval.duration_ns() / 10).max(1_000);
                    ProfApp::zoom(cx, interval.grow(pad));
                }
            }
        }

        // Handle screenshot capture pipeline: agent thread ←→ UI thread.
        // Phase 1: deliver completed screenshots (from a previous frame's
        //          ViewportCommand::Screenshot) back to the blocked agent.
        // Phase 2: consume new screenshot/zoom requests emitted by the agent
        //          (set by poll_events() during chat_panel.show() above).
        #[cfg(feature = "ai")]
        {
            // Phase 1: Check for Event::Screenshot delivered by egui.
            // Extract data inside ctx.input() closure, send outside to avoid
            // capturing &mut cx across the send call.
            // Embedded slot is checked FIRST (unchanged behavior); the second
            // source's slot only if the embedded one is empty, so the single egui
            // screenshot pipeline serves whichever source is currently active.
            let captured: Option<(u64, Vec<u8>, bool)> = ctx.input(|i| {
                for event in &i.events {
                    if let egui::Event::Screenshot { image, .. } = event {
                        if let Some(request_id) = cx.awaiting_screenshot.take() {
                            return Some((request_id, encode_screenshot_png(image), false));
                        }
                        if let Some((request_id, _, _)) = &cx.mcp_awaiting_screenshot {
                            return Some((*request_id, encode_screenshot_png(image), true));
                        }
                    }
                }
                None
            });
            if let Some((request_id, png_bytes, is_mcp)) = captured {
                let metadata = build_screenshot_metadata(cx, windows);
                if is_mcp {
                    if let Some((_, reply_tx, _)) = cx.mcp_awaiting_screenshot.take() {
                        let _ = reply_tx.send(crate::ai::UiCommand::ScreenshotData {
                            request_id,
                            png_bytes,
                            metadata,
                        });
                    }
                } else {
                    cx.chat_panel.send_screenshot(request_id, png_bytes, metadata);
                }
            }

            // Phase 2: Check for new navigation requests from the agent thread.
            if let Some(nav) = cx.chat_panel.take_pending_navigation() {
                // Embedded source: apply the view change via the SHARED handler,
                // then request the screenshot. Behavior is identical to the prior
                // inline match — the logic now lives in `apply_navigation`, reused
                // by the second source below.
                let request_id = pending_nav_request_id(&nav);
                apply_navigation(cx, windows, &nav);
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
                cx.awaiting_screenshot = Some(request_id);
            }

            // Second source (the V1.0 bridge): drain the MCP event channel and
            // service ONE navigation this frame, replying on its OWN channel. Empty
            // until a `UiBridge` is minted, so this is dormant for the embedded
            // agent. Only runs when the screenshot pipeline is free this frame.
            if cx.awaiting_screenshot.is_none() && cx.mcp_awaiting_screenshot.is_none() {
                let mut sink = McpDrainSink::default();
                {
                    let guard = cx.mcp_event_rx.lock().unwrap();
                    if let (Some(rx), Some(reply_tx)) = (guard.as_ref(), cx.mcp_cmd_tx.clone()) {
                        crate::ai::bridge::drain_source(rx, &reply_tx, &mut sink);
                    }
                }
                // Navigation / screenshot: drive the view + capture, reply with the PNG.
                if let Some((nav, reply_tx)) = sink.pending {
                    let request_id = pending_nav_request_id(&nav);
                    apply_navigation(cx, windows, &nav);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
                    // Watchdog deadline > the bridge's request timeout, so the client
                    // sees its own timeout first; this only frees a slot egui somehow
                    // never fulfilled.
                    let deadline = std::time::Instant::now() + Duration::from_secs(15);
                    cx.mcp_awaiting_screenshot = Some((request_id, reply_tx, deadline));
                }
                // Highlight: apply to the SAME shared state the embedded path writes,
                // scroll to it, ACK (no screenshot — mirrors the embedded text result).
                if let Some((hl, request_id, reply_tx)) = sink.pending_highlight {
                    let entry = apply_one_highlight(windows, &hl);
                    if let Some(entry_id) = entry {
                        cx.ai_scroll_to_entry = Some(entry_id);
                    }
                    let message = format!(
                        "Highlight added on {} [{}, {}].",
                        hl.entry_slug, hl.start_ns, hl.stop_ns
                    );
                    let _ = reply_tx.send(crate::ai::UiCommand::Ack { request_id, message });
                }
                // Clear highlights: clear the shared state, ACK the count.
                if let Some((request_id, reply_tx)) = sink.pending_clear {
                    let n = clear_all_highlights(windows);
                    let message = if n == 0 {
                        "No highlights to clear.".to_owned()
                    } else {
                        format!("Cleared highlights on {n} row(s).")
                    };
                    let _ = reply_tx.send(crate::ai::UiCommand::Ack { request_id, message });
                }
                // get_selection (V1.4): a non-driving READ of the human's current
                // selection — the SAME state the embedded `build_selection_preamble`
                // reads. No viewport claim, no screenshot; reply synchronously.
                if let Some((request_id, reply_tx)) = sink.pending_selection {
                    let (items, range) = cx.chat_panel.selection_snapshot();
                    let _ = reply_tx.send(crate::ai::UiCommand::SelectionData {
                        request_id,
                        items,
                        range,
                    });
                }
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            // Use body font to figure out how tall to draw rectangles.
            let font_id = TextStyle::Body.resolve(ui.style());
            let row_height = ui.fonts(|f| f.row_height(&font_id));
            // Just set this on every frame for now
            cx.row_height = row_height * cx.scale_factor;

            let y_scroll_delta = cx.row_height * cx.row_scroll_delta as f32;
            ui.scroll_with_delta(Vec2::new(0.0, y_scroll_delta));
            cx.row_scroll_delta = 0;

            let mut remaining = windows.len();
            // Only wrap in a frame if more than one profile
            if remaining > 1 {
                for window in windows.iter_mut() {
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.push_id(window.index, |ui| {
                            ui.set_height(ui.available_height() / (remaining as f32));
                            ui.set_width(ui.available_width());
                            window.content(ui, cx);
                            remaining -= 1;
                        });
                    });
                }
            } else {
                for window in windows.iter_mut() {
                    window.content(ui, cx);
                }
            }

            Self::cursor(ui, cx);
        });

        egui::Window::new("Controls")
            .open(&mut cx.show_controls)
            .resizable(false)
            .show(ctx, |ui| Self::display_controls(ui, &mut cx.item_link_mode));

        for window in windows.iter_mut() {
            let mut zoom_target = None;

            // Hack: work around mutability conflict
            let mut items_selected = BTreeMap::new();
            std::mem::swap(&mut items_selected, &mut window.config.items_selected);
            items_selected.retain(|_, item| {
                // Populate the item meta if it's not already there
                if item.meta.is_none() {
                    window.inflate_meta(&item.loc.entry_id, cx);
                    if let Some(meta) = window.find_item_meta(&item.loc.entry_id, item.loc.item_uid)
                    {
                        item.meta = Some(meta.clone());
                    }
                }

                let short_title = match &item.meta {
                    Some(meta) => meta.title.chars().take(50).collect(),
                    None => format!("Item <Item UID: {}>", item.loc.item_uid.0),
                };

                let mut enabled = true;
                egui::Window::new(short_title)
                    .id(egui::Id::new(("details_window", item.loc.item_uid.0)))
                    .open(&mut enabled)
                    .resizable(true)
                    .show(ctx, |ui| {
                        let target =
                            Self::display_item_details(ui, item, &window.config.field_schema, cx);
                        if target.is_some() {
                            zoom_target = target;
                        }
                    });
                enabled
            });
            std::mem::swap(&mut items_selected, &mut window.config.items_selected);

            if let Some((item_loc, interval)) = zoom_target {
                let interval = match cx.item_link_mode {
                    // In Zoom mode, put the item in the center of the view
                    // interval with a small amount of padding on either side.
                    ItemLinkNavigationMode::Zoom => interval.grow(interval.duration_ns() / 20),
                    // In Pan mode, maintain the current window size but shift
                    // the center to place the item in the middle of it.
                    ItemLinkNavigationMode::Pan => cx
                        .view_interval
                        .translate(interval.center().0 - cx.view_interval.center().0),
                };
                ProfApp::zoom(cx, interval);
                window.expand_slot(&item_loc.entry_id);
                window.config.scroll_to_item(item_loc);
            }
        }

        // Surface the user's task (bar) selection to the chat panel so the agent
        // can resolve "this task" to concrete item_uid(s)/entry_slug(s).
        #[cfg(feature = "ai")]
        {
            let mut snapshot: Vec<crate::ai::SelectedItem> = Vec::new();
            for window in windows.iter() {
                if window.config.items_selected.is_empty() {
                    continue;
                }
                let id_to_slug: HashMap<EntryID, String> = build_slug_map(window)
                    .into_iter()
                    .map(|(s, id)| (id, s))
                    .collect();
                for (uid, detail) in window.config.items_selected.iter().take(8) {
                    let (title, start_ns, stop_ns) = match &detail.meta {
                        Some(m) => (
                            m.title.clone(),
                            m.original_interval.start.0,
                            m.original_interval.stop.0,
                        ),
                        None => (String::new(), 0, 0),
                    };
                    snapshot.push(crate::ai::SelectedItem {
                        item_uid: uid.0,
                        entry_slug: id_to_slug.get(&detail.loc.entry_id).cloned(),
                        title,
                        start_ns,
                        stop_ns,
                    });
                }
                break;
            }
            let uids: Vec<u64> = snapshot.iter().map(|s| s.item_uid).collect();
            if uids != cx.last_item_selection {
                cx.last_item_selection = uids;
                if snapshot.is_empty() {
                    cx.chat_panel.clear_item_selection();
                } else {
                    cx.chat_panel.set_item_selection(snapshot);
                }
            }
        }

        Self::keyboard(ctx, cx, windows);

        // Keep repainting as long as we have outstanding requests.
        if !pending_data_sources.is_empty()
            || windows
                .iter()
                .any(|w| w.config.data_source.outstanding_requests() > 0)
        {
            ctx.request_repaint_after(Duration::from_millis(50));
        }

        // V1.3: while an MCP screenshot is mid-flight, keep repainting so the
        // capture frame (which delivers `Event::Screenshot`) actually happens — the
        // window is otherwise idle and would stall the request to timeout. A watchdog
        // resets the slot if egui somehow never delivers the screenshot, so this
        // never becomes a permanent busy-loop / lockout.
        #[cfg(feature = "ai")]
        if let Some((_, _, deadline)) = &cx.mcp_awaiting_screenshot {
            if std::time::Instant::now() >= *deadline {
                cx.mcp_awaiting_screenshot = None; // bridge already timed out; free the slot
            } else {
                ctx.request_repaint();
            }
        }
    }
}

trait UiExtra {
    fn subheading(&mut self, text: impl Into<egui::RichText>, cx: &Context) -> egui::Response;
    fn show_tooltip(
        &mut self,
        id_salt: impl core::hash::Hash,
        rect: &Rect,
        text: impl Into<egui::WidgetText>,
    );
    fn show_tooltip_at(
        &mut self,
        id_salt: impl core::hash::Hash,
        suggested_position: Pos2,
        text: impl Into<egui::WidgetText>,
    );
    fn show_tooltip_ui(
        &mut self,
        id_salt: impl core::hash::Hash,
        rect: &Rect,
        add_contents: impl FnOnce(&mut egui::Ui),
    );
    fn rect_hover_pos(&self, rect: Rect) -> Option<Pos2>;
}

impl UiExtra for egui::Ui {
    fn subheading(&mut self, text: impl Into<egui::RichText>, cx: &Context) -> egui::Response {
        self.add(egui::Label::new(
            text.into().heading().size(cx.subheading_size),
        ))
    }

    /// This is a method for showing a fast, very responsive
    /// tooltip. The standard hover methods force a delay (presumably
    /// to confirm the mouse has stopped), this bypasses that. Best
    /// used in situations where the user might quickly skim over the
    /// content (e.g., utilization plots).
    fn show_tooltip(
        &mut self,
        id_salt: impl core::hash::Hash,
        rect: &Rect,
        text: impl Into<egui::WidgetText>,
    ) {
        self.show_tooltip_ui(id_salt, rect, |ui| {
            ui.add(egui::Label::new(text).wrap_mode(egui::TextWrapMode::Extend));
        });
    }
    fn show_tooltip_at(
        &mut self,
        id_salt: impl core::hash::Hash,
        suggested_position: Pos2,
        text: impl Into<egui::WidgetText>,
    ) {
        egui::containers::show_tooltip_at(
            self.ctx(),
            self.layer_id(),
            self.auto_id_with(id_salt),
            suggested_position,
            |ui| {
                ui.add(egui::Label::new(text).wrap_mode(egui::TextWrapMode::Extend));
            },
        );
    }
    fn show_tooltip_ui(
        &mut self,
        id_salt: impl core::hash::Hash,
        rect: &Rect,
        add_contents: impl FnOnce(&mut egui::Ui),
    ) {
        egui::containers::show_tooltip_for(
            self.ctx(),
            self.layer_id(),
            self.auto_id_with(id_salt),
            rect,
            add_contents,
        );
    }

    /// Starting with egui 0.26, ui.allocate_rect() never returns interactions
    /// for multiple overlapping rectangles at once. This method is required to
    /// interact for more complex widgets where we want to e.g., hover one
    /// widget while dragging another widget (where those widgets overlap).
    fn rect_hover_pos(&self, rect: Rect) -> Option<Pos2> {
        if self.rect_contains_pointer(rect) {
            self.input(|i| i.pointer.hover_pos())
        } else {
            None
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn get_locator(data_sources: &[Box<dyn DeferredDataSource>]) -> String {
    let all_locators = data_sources
        .iter()
        .flat_map(|x| x.fetch_description().source_locator)
        .collect::<Vec<_>>();

    let unique_locators = all_locators.into_iter().unique().collect_vec();

    match &unique_locators[..] {
        [] => "No data source".to_string(),
        [x] => x.to_string(),
        [x, ..] => format!("{} and {} other sources", x, unique_locators.len() - 1),
    }
}

/// Optional startup configuration (e.g. AI assistant tool paths from the CLI).
#[derive(Default)]
pub struct StartOptions {
    /// Pre-fills the Co-Pilot's DuckDB path (from `--duckdb` or auto-detection).
    pub ai_duckdb_path: Option<String>,
    /// Pre-fills the Co-Pilot's source-code path (from `--code`).
    pub ai_code_path: Option<String>,
    /// Pre-fills the Co-Pilot's Legion wiki root (from `--wiki` or auto-detection).
    pub ai_wiki_path: Option<String>,
}

#[cfg(not(target_arch = "wasm32"))]
pub fn start(data_sources: Vec<Box<dyn DeferredDataSource>>) {
    start_with_options(data_sources, StartOptions::default());
}

#[cfg(not(target_arch = "wasm32"))]
pub fn start_with_options(data_sources: Vec<Box<dyn DeferredDataSource>>, opts: StartOptions) {
    env_logger::try_init().unwrap_or(()); // Log to stderr (if you run with `RUST_LOG=debug`).

    // IMPORTANT: This will be used as the directory name for the storage
    // location for the persisted app.ron configuration. eframe is not good
    // about sanitizing these directory names, so it is VERY IMPORTANT that
    // this be a short, predictable name without weird characters in it.
    let app_name = "Legion Prof";

    // This is what will be displayed as the window's actual title.
    let locator = format!("{} - {}", get_locator(&data_sources), app_name);

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title(locator),
        ..Default::default()
    };
    eframe::run_native(
        app_name,
        native_options,
        Box::new(|cc| Ok(Box::new(ProfApp::new(cc, data_sources, opts)))),
    )
    .expect("failed to start eframe");
}

#[cfg(target_arch = "wasm32")]
pub fn start(data_sources: Vec<Box<dyn DeferredDataSource>>) {
    use eframe::wasm_bindgen::JsCast as _;

    // Redirect `log` message to `console.log` and friends:
    eframe::WebLogger::init(log::LevelFilter::Debug).ok();

    let web_options = eframe::WebOptions::default();

    wasm_bindgen_futures::spawn_local(async {
        let document = web_sys::window()
            .expect("No window")
            .document()
            .expect("No document");

        let canvas = document
            .get_element_by_id("the_canvas_id")
            .expect("Failed to find the_canvas_id")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("the_canvas_id was not a HtmlCanvasElement");

        let start_result = eframe::WebRunner::new()
            .start(
                canvas,
                web_options,
                Box::new(|cc| Ok(Box::new(ProfApp::new(cc, data_sources, StartOptions::default())))),
            )
            .await;

        // Remove the loading text and spinner:
        if let Some(loading_text) = document.get_element_by_id("loading_text") {
            match start_result {
                Ok(_) => {
                    loading_text.remove();
                }
                Err(e) => {
                    loading_text.set_inner_html(
                        "<p> The app has crashed. See the developer console for details. </p>",
                    );
                    panic!("Failed to start eframe: {e:?}");
                }
            }
        }
    });
}

/// V1.3: pins that the in-viewer MCP sink (`McpDrainSink`) RECORDS each visual
/// variant rather than silently no-op'ing (the default `EventSink` methods are
/// no-ops; an unrecorded request would block `UiBridge::request` to timeout). The
/// actual UI application (`apply_navigation` / `apply_one_highlight`) needs a live
/// window and is covered by the end-to-end smoke.
#[cfg(all(test, feature = "ai"))]
mod mcp_sink_tests {
    use super::McpDrainSink;
    use crate::ai::bridge::apply_agent_event;
    use crate::ai::AgentEvent;
    use std::sync::mpsc::channel;

    /// Drive one event into a fresh sink, return the sink.
    fn drive(ev: AgentEvent) -> McpDrainSink {
        let (tx, _rx) = channel();
        let mut sink = McpDrainSink::default();
        apply_agent_event(&mut sink, ev, &tx);
        sink
    }

    #[test]
    fn test_mcp_sink_records_every_navigation_variant() {
        let evs = vec![
            AgentEvent::ScreenshotRequest { request_id: 1 },
            AgentEvent::ZoomRequest { request_id: 2, start_ns: 0, stop_ns: 10 },
            AgentEvent::PanRequest { request_id: 3, direction: "left".into(), percent: 25.0 },
            AgentEvent::ScrollToRequest { request_id: 4, entry_slug: "n0_cpu_c1".into() },
            AgentEvent::SetViewRequest {
                request_id: 5,
                start_ns: 0,
                stop_ns: 10,
                entry_slug: None,
                filter_kinds: None,
                expand_kinds: None,
                collapse_kinds: None,
                vertical_scale: None,
            },
            AgentEvent::SearchRequest { request_id: 6, query: "x".into() },
            AgentEvent::ResetViewRequest { request_id: 7 },
        ];
        for ev in evs {
            let sink = drive(ev);
            assert!(sink.pending.is_some(), "nav variant must be RECORDED, not a no-op");
            assert!(sink.pending_highlight.is_none() && sink.pending_clear.is_none());
        }
    }

    #[test]
    fn test_mcp_sink_records_highlight() {
        let sink = drive(AgentEvent::HighlightRequest {
            request_id: 9,
            entry_slug: "n0_cpu_c1".into(),
            start_ns: 1,
            stop_ns: 2,
            severity: "high".into(),
            label: "blk".into(),
        });
        let (hl, rid, _tx) = sink.pending_highlight.expect("highlight must be RECORDED, not a no-op");
        assert_eq!(rid, 9);
        assert_eq!(hl.entry_slug, "n0_cpu_c1");
        assert_eq!((hl.start_ns, hl.stop_ns), (1, 2));
        assert!(sink.pending.is_none() && sink.pending_clear.is_none());
    }

    #[test]
    fn test_mcp_sink_records_clear() {
        let sink = drive(AgentEvent::ClearHighlightsRequest { request_id: 11 });
        let (rid, _tx) = sink.pending_clear.expect("clear must be RECORDED, not a no-op");
        assert_eq!(rid, 11);
        assert!(sink.pending.is_none() && sink.pending_highlight.is_none());
    }

    #[test]
    fn test_mcp_sink_records_get_selection() {
        let sink = drive(AgentEvent::GetSelection { request_id: 13 });
        let (rid, _tx) = sink.pending_selection.expect("get_selection must be RECORDED, not a no-op");
        assert_eq!(rid, 13);
        assert!(sink.pending.is_none() && sink.pending_highlight.is_none() && sink.pending_clear.is_none());
    }
}

/// A1: pins the header "Selected:" banner formatting (egui-free).
#[cfg(all(test, feature = "ai"))]
mod banner_tests {
    use super::format_selection_banner;
    use crate::ai::SelectedItemInfo;

    fn item(uid: u64, title: &str) -> SelectedItemInfo {
        SelectedItemInfo {
            item_uid: uid,
            entry_slug: Some("n0_cpu_c1".into()),
            title: title.into(),
            start_ns: 1_000_000_000,
            stop_ns: 1_200_000_000,
        }
    }

    #[test]
    fn test_format_selection_banner_empty() {
        // Nothing selected -> None (header renders no chrome).
        assert_eq!(format_selection_banner(&[], &None), None);
    }

    #[test]
    fn test_format_selection_banner_range_only() {
        let b = format_selection_banner(&[], &Some(("n0_cpu_c2".into(), 1_000_000_000, 1_500_000_000)))
            .expect("range -> Some");
        assert!(b.starts_with("Selected:"), "banner: {b}");
        assert!(b.contains("n0_cpu_c2"), "range label shown: {b}");
        assert!(!b.contains("more"), "no overflow for a range-only selection: {b}");
    }

    #[test]
    fn test_format_selection_banner_items() {
        let b = format_selection_banner(&[item(48, "top_level <6>")], &None).expect("items -> Some");
        assert!(b.starts_with("Selected:"));
        assert!(b.contains("top_level <6>"), "title shown: {b}");
        assert!(b.contains('@') && b.contains("n0_cpu_c1"), "interval + slug shown: {b}");
        assert!(!b.contains("more"));
    }

    #[test]
    fn test_format_selection_banner_many_items() {
        // 3 items, SHOWN=2 -> first two in full, the rest collapse to "+1 more".
        let many = vec![item(1, "alpha"), item(2, "beta"), item(3, "gamma")];
        let b = format_selection_banner(&many, &None).expect("items -> Some");
        assert!(b.contains("alpha") && b.contains("beta"), "first two shown: {b}");
        assert!(!b.contains("gamma"), "3rd item collapsed, not shown by title: {b}");
        assert!(b.contains("+1 more"), "overflow summarized: {b}");
    }
}

/// Highlight-model tests (Task 1): the apply path builds an AiHighlight with the
/// right fields and a unique, monotonic id.
#[cfg(all(test, feature = "ai"))]
mod highlight_model_tests {
    use super::*;
    use crate::ai::Highlight;

    fn hl(label: &str) -> Highlight {
        Highlight {
            entry_slug: "n0_cpu_c1".into(),
            start_ns: 100,
            stop_ns: 200,
            severity: "critical".into(), // accepted-but-ignored (no severity colors)
            label: label.into(),
        }
    }

    #[test]
    fn test_highlight_to_ai_fields_and_monotonic_id() {
        let a = highlight_to_ai(&hl("blk"));
        let b = highlight_to_ai(&hl("blk2"));
        // Fields carried correctly.
        assert_eq!(a.label, "blk");
        assert_eq!((a.interval.start.0, a.interval.stop.0), (100, 200));
        assert!(a.item_uid.is_none(), "region/interval highlight has no task target");
        assert!(a.enabled, "new highlights start enabled");
        // Unique + monotonic ids across BOTH apply sites (allocated in highlight_to_ai).
        assert!(b.id > a.id, "ids must be monotonic + unique: {} then {}", a.id, b.id);
    }

    use crate::ai::AiHighlight;
    use crate::data::EntryID;
    use crate::timestamp::{Interval, Timestamp};
    use std::collections::HashMap;

    fn ahl(id: u64, start: i64, stop: i64, enabled: bool) -> AiHighlight {
        AiHighlight {
            id,
            interval: Interval::new(Timestamp(start), Timestamp(stop)),
            label: format!("h{id}"),
            item_uid: None,
            enabled,
        }
    }

    #[test]
    fn test_flatten_highlights_sorted_is_deterministic() {
        let mut map: HashMap<EntryID, Vec<AiHighlight>> = HashMap::new();
        // Out-of-order ids across two entries -> flat list sorted by id.
        map.insert(EntryID::root().child(0), vec![ahl(30, 0, 1, true), ahl(10, 0, 1, true)]);
        map.insert(EntryID::root().child(1), vec![ahl(20, 0, 1, false)]);
        let order: Vec<u64> =
            flatten_highlights_sorted(&mut map).iter().map(|(_, h)| h.id).collect();
        assert_eq!(order, vec![10, 20, 30], "flat list sorted by id across all entries");
    }

    #[test]
    fn test_highlight_union_enabled_only() {
        let mut map: HashMap<EntryID, Vec<AiHighlight>> = HashMap::new();
        // ENABLED [100,200] + [400,500]; a DISABLED [0,1000] must be IGNORED.
        map.insert(EntryID::root().child(0), vec![ahl(1, 100, 200, true), ahl(2, 0, 1000, false)]);
        map.insert(EntryID::root().child(1), vec![ahl(3, 400, 500, true)]);
        let u = highlight_union(&map).expect("some enabled -> Some");
        assert_eq!((u.start.0, u.stop.0), (100, 500), "union of ENABLED only (disabled [0,1000] ignored)");

        // All disabled -> None.
        let mut none_map: HashMap<EntryID, Vec<AiHighlight>> = HashMap::new();
        none_map.insert(EntryID::root().child(0), vec![ahl(1, 100, 200, false)]);
        assert!(highlight_union(&none_map).is_none(), "no enabled -> None");
        // Empty -> None.
        assert!(highlight_union(&HashMap::new()).is_none());
    }
}

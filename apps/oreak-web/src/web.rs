use std::collections::{BTreeMap, BTreeSet};

use gloo_timers::callback::Timeout;
use js_sys::Date;
use lucide_yew::{
    ChevronDown, ChevronRight, Eraser, FolderTree, GitCommitHorizontal, Image, Map, Maximize,
    MessageSquare, MousePointer2, PanelLeft, PanelLeftClose, PanelRight, PanelRightClose, Plus,
    Settings, Shapes, SlidersHorizontal, Square, Trash, Users, ZoomIn, ZoomOut,
};
use oreak_core::{
    BlameEntry, BlindPixel, BlindStroke, CellEdit, CellKind, CollectCapacity, CollectLayer,
    CommandEnvelope, DecoratorKind, DirectionMode, EntityId, EntityMove, EntityTransform,
    GridAnchor, GridPoint, GridSize, HistoryEvent, LevelCommand, LevelSnapshot, LevelTarget,
    PlaceableEntity, PlaceableEntityKind, Shape, ShapeCell,
};
use oreak_protocol::{
    ApplyCommandRequest, ApplyCommandResponse, ApplyCommandResult, LevelEvent, LevelPresenceItem,
    LevelSnapshotResponse, LevelSubscriptionItem, ProjectLevelTarget, UndoLatestRequest,
    UndoLatestResponse, UpdateLevelCursorRequest,
};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{
    CanvasRenderingContext2d, ClipboardEvent, DragEvent, Element, File, HtmlCanvasElement,
    HtmlElement, HtmlInputElement, HtmlSelectElement, KeyboardEvent, PointerEvent, ResizeObserver,
    WheelEvent,
};
use yew::prelude::*;

use crate::api::{
    ApiError, CatalogSnapshot, ImageCatalogEntry, LevelConfiguration, LevelSummary,
    MembershipSummary, ProjectConfiguration, ProjectInvitationSummary, ProjectSummary, RestClient,
    ShapeCatalogEntry, ShapeDefinition, UserSummary, WorkspaceSummary,
    catalog_project_for_workspace, draft_storage_key, initial_catalog_level,
};
use crate::model::{
    EditorModel, MapResizeEdge, Mode, ModelChange, MoveDirection, PresenceRoster, Selection,
    ShapeBoundaryEdges, Shortcut, ShortcutScope, ViewportTransform,
    blind_pixel_from_top_left_sample, designer_mask_from_shape, dragged_entity_preview,
    entity_ids_in_rect, hydrate_history, key_locker_for_key, key_locker_for_lock,
    plan_content_aware_edge_resize, pool_tile_geometry, rasterize_blind_segment_in_tile,
    reconnect_delay_ms, resolve_shortcut, resolved_entity_preview, select_entity,
    shape_boundary_edges, shape_from_designer_mask, shape_world_points,
};
use crate::rpc::{RpcClient, RpcUpdate};

#[path = "image_studio.rs"]
mod image_studio;
use image_studio::{ImageMsg, ImageStudioState};

#[path = "distribution_studio.rs"]
mod distribution_studio;
use distribution_studio::{DistributionMsg, DistributionState};

const CANVAS_WIDTH: u32 = 1280;
const CANVAS_HEIGHT: u32 = 720;
const BOARD_ORIGIN: f64 = 44.0;
const CELL_SIZE: f64 = 85.0;
const BLIND_INSET: f64 = 6.0;
const THEME_STORAGE_KEY: &str = "oreak.theme.override";
const UI_FONT_SIZE_STORAGE_KEY: &str = "oreak.ui.font-size.v1";
const UI_SCALE_STORAGE_KEY: &str = "oreak.ui.scale.v1";
const INSPECTOR_COLLAPSE_STORAGE_PREFIX: &str = "oreak.inspector.collapsed.v1.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Bootstrapping,
    Authentication,
    Editor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AuthMode {
    Login,
    Register,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Theme {
    Dark,
    Light,
}

impl Theme {
    const fn from_name(value: &str) -> Option<Self> {
        match value.as_bytes() {
            b"dark" => Some(Self::Dark),
            b"light" => Some(Self::Light),
            _ => None,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Light => "Light",
        }
    }

    const fn opposite(self) -> Self {
        match self {
            Self::Dark => Self::Light,
            Self::Light => Self::Dark,
        }
    }
}

#[derive(Debug)]
struct ThemeSettings {
    project_default: Theme,
    user_override: Option<Theme>,
}

impl ThemeSettings {
    fn load() -> Self {
        let user_override = storage()
            .and_then(|store| store.get_item(THEME_STORAGE_KEY).ok().flatten())
            .and_then(|value| match value.as_str() {
                "dark" => Some(Theme::Dark),
                "light" => Some(Theme::Light),
                _ => None,
            });
        Self {
            project_default: Theme::Dark,
            user_override,
        }
    }

    const fn active(&self) -> Theme {
        match self.user_override {
            Some(theme) => theme,
            None => self.project_default,
        }
    }

    fn toggle(&mut self) {
        let next = self.active().opposite();
        self.user_override = Some(next);
        if let Some(store) = storage() {
            let _ = store.set_item(THEME_STORAGE_KEY, next.name());
        }
    }

    fn use_project_default(&mut self) {
        self.user_override = None;
        if let Some(store) = storage() {
            let _ = store.remove_item(THEME_STORAGE_KEY);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct UiSettings {
    font_size_offset: i8,
    scale_percent: u16,
}

impl UiSettings {
    const FONT_SIZE_OPTIONS: [(i8, &'static str); 4] = [
        (-1, "Compact"),
        (0, "Default"),
        (1, "Large"),
        (2, "Extra large"),
    ];
    const SCALE_OPTIONS: [(u16, &'static str); 4] =
        [(90, "90%"), (100, "100%"), (110, "110%"), (125, "125%")];

    fn load() -> Self {
        let store = storage();
        let font_size_offset = store
            .as_ref()
            .and_then(|store| store.get_item(UI_FONT_SIZE_STORAGE_KEY).ok().flatten())
            .and_then(|value| value.parse::<i8>().ok())
            .filter(|value| {
                Self::FONT_SIZE_OPTIONS
                    .iter()
                    .any(|(option, _)| option == value)
            })
            .unwrap_or(0);
        let scale_percent = store
            .as_ref()
            .and_then(|store| store.get_item(UI_SCALE_STORAGE_KEY).ok().flatten())
            .and_then(|value| value.parse::<u16>().ok())
            .filter(|value| {
                Self::SCALE_OPTIONS
                    .iter()
                    .any(|(option, _)| option == value)
            })
            .unwrap_or(100);
        Self {
            font_size_offset,
            scale_percent,
        }
    }

    fn set_font_size(&mut self, value: &str) -> bool {
        let Ok(value) = value.parse::<i8>() else {
            return false;
        };
        if !Self::FONT_SIZE_OPTIONS
            .iter()
            .any(|(option, _)| *option == value)
            || self.font_size_offset == value
        {
            return false;
        }
        self.font_size_offset = value;
        if let Some(store) = storage() {
            let _ = store.set_item(UI_FONT_SIZE_STORAGE_KEY, &value.to_string());
        }
        true
    }

    fn set_scale(&mut self, value: &str) -> bool {
        let Ok(value) = value.parse::<u16>() else {
            return false;
        };
        if !Self::SCALE_OPTIONS
            .iter()
            .any(|(option, _)| *option == value)
            || self.scale_percent == value
        {
            return false;
        }
        self.scale_percent = value;
        if let Some(store) = storage() {
            let _ = store.set_item(UI_SCALE_STORAGE_KEY, &value.to_string());
        }
        true
    }

    fn reset(&mut self) -> bool {
        if *self == Self::default() {
            return false;
        }
        *self = Self::default();
        if let Some(store) = storage() {
            let _ = store.remove_item(UI_FONT_SIZE_STORAGE_KEY);
            let _ = store.remove_item(UI_SCALE_STORAGE_KEY);
        }
        true
    }

    fn canvas_font(self, base_size: u8, bold: bool) -> String {
        let size = i16::from(base_size) + i16::from(self.font_size_offset);
        format!(
            "{}{size}px ui-monospace, SFMono-Regular, Consolas, monospace",
            if bold { "bold " } else { "" }
        )
    }

    fn shell_style(self, left_sidebar_width: f64, right_sidebar_width: f64) -> String {
        let ui_scale = f64::from(self.scale_percent) / 100.0;
        let inverse_scale = 100.0 / ui_scale;
        let layout_left_width = left_sidebar_width / ui_scale;
        let layout_right_width = right_sidebar_width / ui_scale;
        let canvas_min_width = 480.0 / ui_scale;
        let compact_canvas_min_width = 440.0 / ui_scale;
        let narrow_sidebar_width = 220.0 / ui_scale;
        let narrow_canvas_min_width = 460.0 / ui_scale;
        format!(
            "--left-sidebar-width: {layout_left_width:.2}px; --right-sidebar-width: {layout_right_width:.2}px; --canvas-min-width: {canvas_min_width:.2}px; --compact-canvas-min-width: {compact_canvas_min_width:.2}px; --narrow-sidebar-width: {narrow_sidebar_width:.2}px; --narrow-canvas-min-width: {narrow_canvas_min_width:.2}px; --ui-font-offset: {}px; --ui-scale: {ui_scale:.2}; width: {inverse_scale:.4}vw; height: {inverse_scale:.4}vh; zoom: var(--ui-scale);",
            self.font_size_offset,
        )
    }
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            font_size_offset: 0,
            scale_percent: 100,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TopMenu {
    File,
    History,
    View,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UiIcon {
    Blame,
    Explorer,
    Comments,
    Collaboration,
    Inspector,
    LevelConfiguration,
    ShapeTemplates,
    PlaceEntity,
    MapDesign,
    ImageStudio,
    DrawWall,
    ClearWall,
    Resize,
    DockLeft,
    DockRight,
    HideLeft,
    HideRight,
    Float,
    ZoomOut,
    Frame,
    ZoomIn,
}

fn ui_icon(icon: UiIcon, size: usize) -> Html {
    let icon = match icon {
        UiIcon::Blame => html! { <GitCommitHorizontal size={size} /> },
        UiIcon::Explorer => html! { <FolderTree size={size} /> },
        UiIcon::Comments => html! { <MessageSquare size={size} /> },
        UiIcon::Collaboration => html! { <Users size={size} /> },
        UiIcon::Inspector => html! { <SlidersHorizontal size={size} /> },
        UiIcon::LevelConfiguration => html! { <Settings size={size} /> },
        UiIcon::ShapeTemplates => html! { <Shapes size={size} /> },
        UiIcon::PlaceEntity => html! { <MousePointer2 size={size} /> },
        UiIcon::MapDesign => html! { <Map size={size} /> },
        UiIcon::ImageStudio => html! { <Image size={size} /> },
        UiIcon::DrawWall => html! { <Square size={size} /> },
        UiIcon::ClearWall => html! { <Eraser size={size} /> },
        UiIcon::DockLeft => html! { <PanelLeft size={size} /> },
        UiIcon::DockRight => html! { <PanelRight size={size} /> },
        UiIcon::HideLeft => html! { <PanelLeftClose size={size} /> },
        UiIcon::HideRight => html! { <PanelRightClose size={size} /> },
        UiIcon::Resize | UiIcon::Frame | UiIcon::Float => html! { <Maximize size={size} /> },
        UiIcon::ZoomOut => html! { <ZoomOut size={size} /> },
        UiIcon::ZoomIn => html! { <ZoomIn size={size} /> },
    };
    html! { <span class="ui-icon" aria-hidden="true">{icon}</span> }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RightTab {
    Inspector,
    Blame,
    LevelStructure,
    LevelConfiguration,
    AdvancedDistribution,
}

impl RightTab {
    const fn label(self) -> &'static str {
        match self {
            Self::Inspector => "Inspector",
            Self::Blame => "Blame",
            Self::LevelStructure => "Level structure",
            Self::LevelConfiguration => "Level configuration",
            Self::AdvancedDistribution => "Advanced Distribution",
        }
    }

    const fn icon(self) -> UiIcon {
        match self {
            Self::Inspector => UiIcon::Inspector,
            Self::Blame => UiIcon::Blame,
            Self::LevelStructure => UiIcon::Explorer,
            Self::LevelConfiguration => UiIcon::LevelConfiguration,
            Self::AdvancedDistribution => UiIcon::Inspector,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum LeftTab {
    #[default]
    ShapeTemplates,
    SandStudio,
    ApplyImage,
    Explorer,
    Comments,
    Collaboration,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ShapeTemplateView {
    #[default]
    Grid,
    List,
}

impl LeftTab {
    const fn label(self) -> &'static str {
        match self {
            Self::ShapeTemplates => "Shape templates",
            Self::SandStudio => "Sand Studio",
            Self::ApplyImage => "Apply Image",
            Self::Explorer => "Project explorer",
            Self::Comments => "Comments",
            Self::Collaboration => "Collaboration session",
        }
    }

    const fn icon(self) -> UiIcon {
        match self {
            Self::ShapeTemplates => UiIcon::ShapeTemplates,
            Self::SandStudio => UiIcon::ImageStudio,
            Self::ApplyImage => UiIcon::ImageStudio,
            Self::Explorer => UiIcon::Explorer,
            Self::Comments => UiIcon::Comments,
            Self::Collaboration => UiIcon::Collaboration,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum InspectorSection {
    Selection,
    EntityData,
    Capacity,
    Decorators,
    PoolResolution,
}

impl InspectorSection {
    const fn storage_key(self) -> &'static str {
        match self {
            Self::Selection => "selection",
            Self::EntityData => "entity-data",
            Self::Capacity => "capacity",
            Self::Decorators => "decorators",
            Self::PoolResolution => "pool-resolution",
        }
    }

    fn from_storage_key(value: &str) -> Option<Self> {
        match value {
            "selection" => Some(Self::Selection),
            "entity-data" => Some(Self::EntityData),
            "capacity" => Some(Self::Capacity),
            "decorators" => Some(Self::Decorators),
            "pool-resolution" => Some(Self::PoolResolution),
            _ => None,
        }
    }

    const fn content_id(self) -> &'static str {
        match self {
            Self::Selection => "inspector-selection-content",
            Self::EntityData => "inspector-entity-data-content",
            Self::Capacity => "inspector-capacity-content",
            Self::Decorators => "inspector-decorators-content",
            Self::PoolResolution => "inspector-pool-resolution-content",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum WorkspaceTool {
    Place,
    #[default]
    Transform,
    Decorate,
    Cells,
    Resize,
    Paint,
    Fill,
    SandboxMove,
}

impl WorkspaceTool {
    const fn for_mode(mode: Mode) -> &'static [Self] {
        match mode {
            Mode::Select => &[Self::Transform, Self::Place, Self::Decorate],
            Mode::Map => &[Self::Cells, Self::Resize],
            Mode::Brush => &[Self::Paint, Self::Fill],
            Mode::Distribution => &[],
            Mode::Sandbox => &[Self::SandboxMove],
        }
    }

    const fn default_for(mode: Mode) -> Self {
        match mode {
            Mode::Select => Self::Transform,
            Mode::Map => Self::Cells,
            Mode::Brush => Self::Paint,
            Mode::Distribution => Self::Transform,
            Mode::Sandbox => Self::SandboxMove,
        }
    }

    fn from_key(mode: Mode, key: &str) -> Option<Self> {
        let index = match key.to_ascii_lowercase().as_str() {
            "z" => 0,
            "x" => 1,
            "c" => 2,
            _ => return None,
        };
        Self::for_mode(mode).get(index).copied()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PaletteCommand {
    Select,
    Map,
    Brush,
    Sandbox,
    Undo,
    ToggleTheme,
    Reconnect,
}

impl PaletteCommand {
    const ALL: [Self; 7] = [
        Self::Select,
        Self::Map,
        Self::Brush,
        Self::Sandbox,
        Self::Undo,
        Self::ToggleTheme,
        Self::Reconnect,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Select => "Switch mode: Select",
            Self::Map => "Switch mode: Map",
            Self::Brush => "Switch mode: Brush",
            Self::Sandbox => "Switch mode: Sandbox (parity gated)",
            Self::Undo => "Undo my latest change",
            Self::ToggleTheme => "Toggle light or dark theme",
            Self::Reconnect => "Reconnect and resync collaboration",
        }
    }

    const fn shortcut(self) -> &'static str {
        match self {
            Self::Select => "Q",
            Self::Map => "W",
            Self::Brush => "B",
            Self::Sandbox => "P",
            Self::Undo => "Ctrl+Z",
            Self::ToggleTheme | Self::Reconnect => "",
        }
    }
}

#[derive(Debug)]
struct Toast {
    id: u32,
    message: String,
    tone: &'static str,
    occurred_at_ms: i64,
}

#[derive(Debug)]
enum RpcState {
    Connecting,
    Online,
    Resyncing,
    Offline,
}

#[derive(Clone, Debug)]
struct PendingCell {
    kind: CellKind,
    command_id: String,
}

#[derive(Clone, Debug)]
struct PendingEntity {
    command_id: String,
    preview: Option<PlaceableEntity>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlacementKind {
    Block,
    Blind,
}

impl PlacementKind {
    const fn label(self) -> &'static str {
        match self {
            Self::Block => "Block",
            Self::Blind => "Pool",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EntityAction {
    RotateClockwise,
    FlipHorizontal,
    Delete,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DecoratorAction {
    ToggleIce,
    ToggleGlass,
    CycleDirection,
    SetDirection(DirectionMode),
    BeginKeyLocker,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum BlindBrushTool {
    #[default]
    Paint,
    FloodFill,
}

#[derive(Debug)]
enum BlindGestureOperation {
    Paint { color_index: u8 },
    Erase,
    FloodFill { start: BlindPixel, color_index: u8 },
}

#[derive(Debug)]
struct BlindGesture {
    pointer_id: i32,
    entity_id: EntityId,
    operation: BlindGestureOperation,
    pixels: BTreeSet<BlindPixel>,
    partition: BTreeSet<BlindPixel>,
    last_pixel: Option<BlindPixel>,
}

#[derive(Debug)]
struct EntityDrag {
    pointer_id: i32,
    start: GridPoint,
    current: GridPoint,
    origins: Vec<(EntityId, GridPoint)>,
    duplicate: bool,
    rotation_steps: u8,
    start_canvas_x: f64,
    start_canvas_y: f64,
    active: bool,
    pointer_on_grid: bool,
}

#[derive(Debug)]
struct EntityDragPreview {
    entities: Vec<PlaceableEntity>,
    valid: bool,
}

#[derive(Debug)]
struct PanGesture {
    pointer_id: i32,
    start_x: f64,
    start_y: f64,
    offset_x: f64,
    offset_y: f64,
}

#[derive(Clone, Copy, Debug)]
struct PlacementDrag {
    kind: PlacementKind,
    shape: Shape,
    hover: Option<GridPoint>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SidebarSide {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PanelLayout {
    Docked,
    Floating,
    Hidden,
}

impl PanelLayout {
    const fn label(self) -> &'static str {
        match self {
            Self::Docked => "DOCKED",
            Self::Floating => "FLOATING",
            Self::Hidden => "HIDDEN",
        }
    }

    const fn toggled(self) -> Self {
        match self {
            Self::Docked => Self::Floating,
            Self::Floating | Self::Hidden => Self::Docked,
        }
    }

    const fn toggle_label(self) -> &'static str {
        match self {
            Self::Docked => "Float",
            Self::Floating | Self::Hidden => "Dock",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StudioModal {
    Shape,
    Image,
}

impl StudioModal {
    const fn label(self) -> &'static str {
        match self {
            Self::Shape => "Shape Studio",
            Self::Image => "Image Studio",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ExplorerAction {
    CreateWorkspace,
    CreateProject {
        workspace_id: String,
    },
    CreateLevel {
        project_id: String,
    },
    DeleteProject {
        project_id: String,
    },
    DeleteLevel {
        project_id: String,
        level_id: String,
    },
}

#[derive(Debug)]
struct SidebarResize {
    side: SidebarSide,
    pointer_id: i32,
    start_client_x: f64,
    start_width: f64,
}

#[derive(Debug)]
struct MapResizeGesture {
    pointer_id: i32,
    edge: MapResizeEdge,
    start_x: f64,
    start_y: f64,
    start_size: GridSize,
    preview_size: GridSize,
    anchor: GridAnchor,
    cell_size: f64,
}

#[derive(Debug)]
struct MarqueeGesture {
    pointer_id: i32,
    start: GridPoint,
    current: GridPoint,
    start_canvas_x: f64,
    start_canvas_y: f64,
    current_canvas_x: f64,
    current_canvas_y: f64,
    additive: bool,
    active: bool,
}

pub struct App {
    phase: Phase,
    session_id: String,
    user: Option<UserSummary>,
    workspaces: Vec<WorkspaceSummary>,
    projects: Vec<ProjectSummary>,
    selected_workspace_id: String,
    catalog_project: Option<ProjectSummary>,
    catalog_levels: BTreeMap<String, Vec<LevelSummary>>,
    catalog_invitations: Vec<ProjectInvitationSummary>,
    workspace_draft_name: String,
    explorer_action: Option<ExplorerAction>,
    expanded_workspaces: BTreeSet<String>,
    expanded_projects: BTreeSet<String>,
    auto_open_initial_level: bool,
    catalog_request_generation: u64,
    level_intent_generation: u64,
    project_configuration: Option<ProjectConfiguration>,
    configuration_project: Option<ProjectSummary>,
    configuration_pending: bool,
    configuration_error: Option<String>,
    invitation_email: String,
    invitation_role: String,
    auth_mode: AuthMode,
    auth_email: String,
    auth_password: String,
    project_name: String,
    level_name: String,
    level_duration: String,
    request_pending: bool,
    form_error: Option<String>,
    model: EditorModel,
    mode: Mode,
    selection: Option<Selection>,
    drag_kind: Option<CellKind>,
    map_paint_kind: Option<CellKind>,
    selected_placement_kind: PlacementKind,
    last_drag: Option<GridPoint>,
    blind_brush_tool: BlindBrushTool,
    blind_color_index: u8,
    blind_gesture: Option<BlindGesture>,
    entity_drag: Option<EntityDrag>,
    pan_gesture: Option<PanGesture>,
    placement_drag: Option<PlacementDrag>,
    sidebar_resize: Option<SidebarResize>,
    map_resize_gesture: Option<MapResizeGesture>,
    marquee_gesture: Option<MarqueeGesture>,
    pending_edge_view: Option<MapResizeEdge>,
    viewport: ViewportTransform,
    canvas_width: u32,
    canvas_height: u32,
    canvas_size_initialized: bool,
    canvas_resize_observer: Option<ResizeObserver>,
    canvas_resize_callback: Option<Closure<dyn FnMut()>>,
    left_sidebar_width: f64,
    right_sidebar_width: f64,
    left_panel_layout: PanelLayout,
    left_panel_layout_before_sand_studio: Option<PanelLayout>,
    right_panel_layout: PanelLayout,
    resize_width: String,
    resize_height: String,
    resize_anchor: GridAnchor,
    shape_catalog: Vec<ShapeCatalogEntry>,
    shape_catalog_pending: bool,
    shape_template_view: ShapeTemplateView,
    shape_draft_name: String,
    shape_draft_mask: u64,
    selected_shape_id: Option<String>,
    active_shape_template_id: Option<String>,
    image_studio: ImageStudioState,
    distribution: DistributionState,
    image_catalog: Vec<ImageCatalogEntry>,
    image_catalog_pending: bool,
    image_draft_name: String,
    image_file: Option<File>,
    image_import_error: Option<String>,
    isolated_blind: Option<EntityId>,
    key_locker_assignment: Option<EntityId>,
    canvas_ref: NodeRef,
    root_ref: NodeRef,
    palette_input_ref: NodeRef,
    image_file_input_ref: NodeRef,
    canvas_dirty: bool,
    theme: ThemeSettings,
    ui_settings: UiSettings,
    inspector_collapsed: BTreeSet<InspectorSection>,
    open_menu: Option<TopMenu>,
    left_tab: LeftTab,
    left_tab_before_sand_studio: Option<LeftTab>,
    right_tab: RightTab,
    studio_modal: Option<StudioModal>,
    workspace_tool: WorkspaceTool,
    palette_open: bool,
    palette_query: String,
    toasts: Vec<Toast>,
    active_toasts: BTreeSet<u32>,
    notifications_open: bool,
    next_toast_id: u32,
    target: ProjectLevelTarget,
    workspace_name: String,
    project_display_name: String,
    level_display_name: String,
    level_duration_seconds: f64,
    level_configuration_open: bool,
    level_configuration_name: String,
    level_configuration_duration: String,
    level_configuration_pending: bool,
    level_configuration_error: Option<String>,
    level_configuration_generation: u64,
    can_edit_timeline: bool,
    rpc: Option<RpcClient>,
    rpc_state: RpcState,
    connection_attempt: u64,
    reconnect_failures: u32,
    reconnect_timer: Option<Timeout>,
    presence: PresenceRoster,
    desired_cursor: Option<GridPoint>,
    last_sent_cursor: Option<Option<GridPoint>>,
    cursor_timer: Option<Timeout>,
    cursor_in_flight: bool,
    server_sequence: Option<u64>,
    server_hash: Option<String>,
    server_events: Vec<HistoryEvent>,
    remote_blame: BTreeMap<GridPoint, BlameEntry>,
    remote_entity_blame: BTreeMap<EntityId, BlameEntry>,
    seen_commands: BTreeSet<String>,
    pending_commands: BTreeSet<String>,
    pending_cells: BTreeMap<GridPoint, PendingCell>,
    pending_entities: BTreeMap<EntityId, PendingEntity>,
    pending_placements: BTreeMap<GridPoint, String>,
    pending_grid: Option<String>,
}

pub enum Msg {
    ImageStudio(ImageMsg),
    Distribution(DistributionMsg),
    BootstrapFinished(Result<UserSummary, ApiError>),
    SetAuthMode(AuthMode),
    AuthEmail(String),
    AuthPassword(String),
    SubmitAuth,
    AuthFinished(Result<UserSummary, ApiError>),
    CatalogLoaded {
        generation: u64,
        result: Result<CatalogSnapshot, ApiError>,
    },
    ReloadCatalog,
    BeginExplorerAction(ExplorerAction),
    CancelExplorerAction,
    ToggleWorkspace(String),
    ToggleProject(String),
    WorkspaceName(String),
    CreateWorkspace,
    WorkspaceCreated {
        intent_generation: u64,
        result: Result<WorkspaceSummary, ApiError>,
    },
    SelectWorkspace(String),
    SelectProject(ProjectSummary),
    ProjectName(String),
    CreateProject,
    ProjectCreated {
        intent_generation: u64,
        result: Result<ProjectSummary, ApiError>,
    },
    DeleteProject(String),
    ProjectDeleted {
        project_id: String,
        result: Result<(), ApiError>,
    },
    LevelName(String),
    LevelDuration(String),
    CreateLevel,
    LevelCreated {
        project: ProjectSummary,
        intent_generation: u64,
        result: Result<LevelSummary, ApiError>,
    },
    DeleteLevel {
        project_id: String,
        level_id: String,
    },
    LevelDeleted {
        project_id: String,
        level_id: String,
        result: Result<(), ApiError>,
    },
    OpenLevel(ProjectSummary, LevelSummary),
    OpenLevelConfiguration,
    CloseLevelConfiguration,
    LevelConfigurationName(String),
    LevelConfigurationDuration(String),
    SaveLevelConfiguration,
    LevelConfigurationLoaded {
        target: ProjectLevelTarget,
        generation: u64,
        result: Result<LevelConfiguration, ApiError>,
    },
    LevelConfigurationUpdated {
        target: ProjectLevelTarget,
        generation: u64,
        result: Result<LevelConfiguration, ApiError>,
    },
    OpenProjectConfiguration(ProjectSummary),
    ProjectConfigurationLoaded {
        project_id: String,
        result: Result<ProjectConfiguration, ApiError>,
    },
    CloseProjectConfiguration,
    InvitationEmail(String),
    InvitationRole(String),
    InviteProjectMember,
    ProjectMemberInvited {
        project_id: String,
        result: Result<ProjectInvitationSummary, ApiError>,
    },
    AcceptProjectInvitation(ProjectInvitationSummary),
    ProjectInvitationAccepted(Result<ProjectSummary, ApiError>),
    SetProjectMemberRole(String, String),
    ProjectMemberUpdated {
        project_id: String,
        result: Result<MembershipSummary, ApiError>,
    },
    SetProjectDefaultTheme(String),
    ProjectConfigurationUpdated {
        project_id: String,
        result: Result<ProjectSummary, ApiError>,
    },
    Logout,
    LogoutFinished(Result<(), ApiError>),
    SetMode(Mode),
    SetMapPaintKind(CellKind),
    SetPlacementContext(PlacementKind),
    OpenSandStudio,
    SetBlindColor(u8),
    OpenStudio(StudioModal),
    CloseStudio,
    FocusExplorerEntity(EntityId),
    EditEntity(EntityAction),
    EditDecorator(DecoratorAction),
    SetIceCount(String),
    SetGlassCount(String),
    SetPoolResolution(u8),
    SetBlockLayerCapacity(usize, String),
    AddBlockCollectLayer,
    RemoveBlockCollectLayer(usize),
    ResizeWidth(String),
    ResizeHeight(String),
    SetResizeAnchor(GridAnchor),
    ResizeGrid,
    ZoomIn,
    ZoomOut,
    FrameGrid,
    CanvasWheel(WheelEvent),
    ShapeCatalogLoaded {
        project_id: String,
        result: Result<Vec<ShapeCatalogEntry>, ApiError>,
    },
    SetShapeTemplateView(ShapeTemplateView),
    ShapeDraftName(String),
    ToggleShapeCell(u8, u8),
    NewShapeDraft,
    SelectShape(String),
    SelectShapeTemplate(String),
    SaveShapeDraft,
    UpdateShapeDraft,
    ShapeSaved {
        project_id: String,
        result: Result<ShapeCatalogEntry, ApiError>,
    },
    DeleteShapeDraft,
    ShapeDeleted {
        project_id: String,
        shape_id: String,
        result: Result<(), ApiError>,
    },
    ImageCatalogLoaded {
        scope: u64,
        project_id: String,
        result: Result<Vec<ImageCatalogEntry>, ApiError>,
    },
    ImageDraftName(String),
    ImageFileSelected(Option<File>),
    ImagePasted(ClipboardEvent),
    ImportImage,
    ImageImported {
        scope: u64,
        generation: u64,
        project_id: String,
        result: Result<ImageCatalogEntry, ApiError>,
    },
    BeginPlacementDrag(PlacementKind, Shape),
    EndPlacementDrag,
    CanvasDragOver(DragEvent),
    CanvasDrop(DragEvent),
    CanvasResized(u32, u32),
    CanvasDown(PointerEvent),
    CanvasMove(PointerEvent),
    CanvasUp(PointerEvent),
    CanvasCancel(PointerEvent),
    CanvasLeave,
    KeyDown(KeyboardEvent),
    ToggleTheme,
    UseProjectTheme,
    SetUiFontSize(String),
    SetUiScale(String),
    ResetUiSettings,
    Undo,
    ToggleMenu(TopMenu),
    SetLeftTab(LeftTab),
    SetRightTab(RightTab),
    SetWorkspaceTool(WorkspaceTool),
    BeginSidebarResize(SidebarSide, PointerEvent),
    SidebarResizeMove(PointerEvent),
    EndSidebarResize(PointerEvent),
    SetPanelLayout(SidebarSide, PanelLayout),
    TogglePanelLayout(SidebarSide),
    ToggleInspectorSection(InspectorSection),
    TogglePalette,
    CloseOverlays,
    PaletteQuery(String),
    RunPalette(PaletteCommand),
    SaveDraft,
    ResetDraft,
    Reconnect,
    RetryConnection(u64),
    DismissToast(u32),
    ToggleNotifications,
    ClearNotifications,
    RpcReady {
        attempt: u64,
        result: Result<RpcClient, String>,
    },
    RpcUpdate {
        attempt: u64,
        update: Box<RpcUpdate>,
    },
    RpcApplyFinished {
        attempt: u64,
        command_id: String,
        result: Box<Result<ApplyCommandResponse, String>>,
    },
    RpcUndoFinished {
        attempt: u64,
        command_id: String,
        result: Box<Result<UndoLatestResponse, String>>,
    },
    FlushCursor(u64),
    CursorSent {
        attempt: u64,
        cursor: Option<GridPoint>,
        result: Result<(), String>,
    },
}

impl Component for App {
    type Message = Msg;
    type Properties = ();

    fn create(ctx: &Context<Self>) -> Self {
        let session_id = browser_session_id();
        spawn_bootstrap(ctx);

        Self {
            phase: Phase::Bootstrapping,
            session_id: session_id.clone(),
            user: None,
            workspaces: Vec::new(),
            projects: Vec::new(),
            selected_workspace_id: String::new(),
            catalog_project: None,
            catalog_levels: BTreeMap::new(),
            catalog_invitations: Vec::new(),
            workspace_draft_name: String::new(),
            explorer_action: None,
            expanded_workspaces: BTreeSet::new(),
            expanded_projects: BTreeSet::new(),
            auto_open_initial_level: false,
            catalog_request_generation: 0,
            level_intent_generation: 0,
            project_configuration: None,
            configuration_project: None,
            configuration_pending: false,
            configuration_error: None,
            invitation_email: String::new(),
            invitation_role: "editor".to_owned(),
            auth_mode: AuthMode::Login,
            auth_email: String::new(),
            auth_password: String::new(),
            project_name: String::new(),
            level_name: String::new(),
            level_duration: "0".to_owned(),
            request_pending: false,
            form_error: None,
            model: EditorModel::blank("bootstrapping", &session_id),
            mode: Mode::Select,
            selection: None,
            drag_kind: None,
            map_paint_kind: None,
            selected_placement_kind: PlacementKind::Block,
            last_drag: None,
            blind_brush_tool: BlindBrushTool::Paint,
            blind_color_index: 1,
            blind_gesture: None,
            entity_drag: None,
            pan_gesture: None,
            placement_drag: None,
            sidebar_resize: None,
            map_resize_gesture: None,
            marquee_gesture: None,
            pending_edge_view: None,
            viewport: ViewportTransform::frame_rect(
                GridSize::new(8, 8).expect("the initial viewport grid is valid"),
                f64::from(CANVAS_WIDTH),
                f64::from(CANVAS_HEIGHT),
                BOARD_ORIGIN,
                CELL_SIZE,
            ),
            canvas_width: CANVAS_WIDTH,
            canvas_height: CANVAS_HEIGHT,
            canvas_size_initialized: false,
            canvas_resize_observer: None,
            canvas_resize_callback: None,
            left_sidebar_width: 252.0,
            right_sidebar_width: 296.0,
            left_panel_layout: PanelLayout::Docked,
            left_panel_layout_before_sand_studio: None,
            right_panel_layout: PanelLayout::Docked,
            resize_width: "8".to_owned(),
            resize_height: "8".to_owned(),
            resize_anchor: GridAnchor::Center,
            shape_catalog: Vec::new(),
            shape_catalog_pending: false,
            shape_template_view: ShapeTemplateView::Grid,
            shape_draft_name: String::new(),
            shape_draft_mask: 1,
            selected_shape_id: None,
            active_shape_template_id: None,
            image_studio: ImageStudioState::default(),
            distribution: DistributionState::default(),
            image_catalog: Vec::new(),
            image_catalog_pending: false,
            image_draft_name: String::new(),
            image_file: None,
            image_import_error: None,
            isolated_blind: None,
            key_locker_assignment: None,
            canvas_ref: NodeRef::default(),
            root_ref: NodeRef::default(),
            palette_input_ref: NodeRef::default(),
            image_file_input_ref: NodeRef::default(),
            canvas_dirty: true,
            theme: ThemeSettings::load(),
            ui_settings: UiSettings::load(),
            inspector_collapsed: BTreeSet::new(),
            open_menu: None,
            left_tab: LeftTab::ShapeTemplates,
            left_tab_before_sand_studio: None,
            right_tab: RightTab::Inspector,
            studio_modal: None,
            workspace_tool: WorkspaceTool::Transform,
            palette_open: false,
            palette_query: String::new(),
            toasts: Vec::new(),
            active_toasts: BTreeSet::new(),
            notifications_open: false,
            next_toast_id: 1,
            target: ProjectLevelTarget::new(String::new(), String::new()),
            workspace_name: String::new(),
            project_display_name: String::new(),
            level_display_name: String::new(),
            level_duration_seconds: 0.0,
            level_configuration_open: false,
            level_configuration_name: String::new(),
            level_configuration_duration: String::new(),
            level_configuration_pending: false,
            level_configuration_error: None,
            level_configuration_generation: 0,
            can_edit_timeline: false,
            rpc: None,
            rpc_state: RpcState::Offline,
            connection_attempt: 0,
            reconnect_failures: 0,
            reconnect_timer: None,
            presence: PresenceRoster::default(),
            desired_cursor: None,
            last_sent_cursor: None,
            cursor_timer: None,
            cursor_in_flight: false,
            server_sequence: None,
            server_hash: None,
            server_events: Vec::new(),
            remote_blame: BTreeMap::new(),
            remote_entity_blame: BTreeMap::new(),
            seen_commands: BTreeSet::new(),
            pending_commands: BTreeSet::new(),
            pending_cells: BTreeMap::new(),
            pending_entities: BTreeMap::new(),
            pending_placements: BTreeMap::new(),
            pending_grid: None,
        }
    }

    fn update(&mut self, ctx: &Context<Self>, msg: Self::Message) -> bool {
        // Leaving the workspace or resetting the timeline discards uncommitted previews.
        if matches!(&msg, Msg::ResetDraft | Msg::Undo | Msg::SelectProject(_) | Msg::SelectWorkspace(_)
            | Msg::OpenLevel(..) | Msg::Logout | Msg::ImageStudio(ImageMsg::Start(_))) {
            self.distribution = DistributionState::default();
        }
        if self.distribution_modal_open() && matches!(&msg,
            Msg::EditEntity(_) | Msg::EditDecorator(_) | Msg::SetIceCount(_) | Msg::SetGlassCount(_)
            | Msg::SetPoolResolution(_) | Msg::SetBlockLayerCapacity(..) | Msg::AddBlockCollectLayer
            | Msg::RemoveBlockCollectLayer(_) | Msg::ResizeGrid | Msg::BeginPlacementDrag(..)
            | Msg::CanvasDrop(_) | Msg::RunPalette(_)) { return false; }
        // The image overlay is preview-only: no unrelated workspace edit may leak through.
        if self.image_studio.apply.is_some() {
            if matches!(&msg, Msg::EditEntity(_) | Msg::EditDecorator(_) | Msg::SetIceCount(_)
                | Msg::SetGlassCount(_) | Msg::SetPoolResolution(_) | Msg::SetBlockLayerCapacity(..)
                | Msg::AddBlockCollectLayer | Msg::RemoveBlockCollectLayer(_) | Msg::ResizeGrid
                | Msg::Undo | Msg::ResetDraft | Msg::BeginPlacementDrag(..) | Msg::CanvasDrop(_)
                | Msg::SetWorkspaceTool(_) | Msg::FocusExplorerEntity(_) | Msg::RunPalette(_)) {
                return false;
            }
            if matches!(&msg, Msg::SetMode(_) | Msg::SetMapPaintKind(_) | Msg::SetPlacementContext(_)
                | Msg::OpenSandStudio | Msg::SelectProject(_) | Msg::SelectWorkspace(_) | Msg::OpenLevel(..)
                | Msg::Logout) {
                self.cancel_apply_image();
                self.cancel_conversion();
            }
        }
        match msg {
            Msg::ImageStudio(msg) => self.image_studio_message(ctx, msg),
            Msg::Distribution(msg) => self.distribution_message(ctx, msg),
            Msg::BootstrapFinished(result) => {
                match result {
                    Ok(user) => self.enter_editor(ctx, user),
                    Err(error) if error.is_unauthorized() => {
                        self.phase = Phase::Authentication;
                        self.form_error = None;
                    }
                    Err(error) => {
                        self.phase = Phase::Authentication;
                        self.form_error = Some(format!(
                            "Unable to reach the session endpoint: {}",
                            error.message
                        ));
                    }
                }
                true
            }
            Msg::SetAuthMode(mode) => {
                self.auth_mode = mode;
                self.form_error = None;
                true
            }
            Msg::AuthEmail(value) => {
                self.auth_email = value;
                true
            }
            Msg::AuthPassword(value) => {
                self.auth_password = value;
                true
            }
            Msg::SubmitAuth => {
                if self.request_pending {
                    return false;
                }
                self.request_pending = true;
                self.form_error = None;
                let email = self.auth_email.clone();
                let password = self.auth_password.clone();
                let mode = self.auth_mode;
                let link = ctx.link().clone();
                wasm_bindgen_futures::spawn_local(async move {
                    let client = RestClient;
                    let result = match mode {
                        AuthMode::Login => client.login(&email, &password).await,
                        AuthMode::Register => client.register(&email, &password).await,
                    };
                    link.send_message(Msg::AuthFinished(result));
                });
                true
            }
            Msg::AuthFinished(result) => {
                self.request_pending = false;
                match result {
                    Ok(user) => {
                        self.auth_password.clear();
                        self.enter_editor(ctx, user);
                    }
                    Err(error) => {
                        self.form_error = Some(api_error_message(&error));
                    }
                }
                true
            }
            Msg::CatalogLoaded { generation, result } => {
                if generation != self.catalog_request_generation
                    || self.phase != Phase::Editor
                    || self.user.is_none()
                {
                    return false;
                }
                self.request_pending = false;
                match result {
                    Ok(snapshot) => {
                        let should_open_initial_level =
                            std::mem::take(&mut self.auto_open_initial_level);
                        let initialize_expansion = self.workspaces.is_empty();
                        self.workspaces = snapshot.workspaces;
                        self.catalog_levels = snapshot
                            .projects
                            .iter()
                            .map(|node| (node.project.id.clone(), node.levels.clone()))
                            .collect();
                        self.projects = snapshot
                            .projects
                            .into_iter()
                            .map(|node| node.project)
                            .collect();
                        self.catalog_invitations = snapshot.invitations;
                        if self.selected_workspace_id.is_empty()
                            || !self
                                .workspaces
                                .iter()
                                .any(|workspace| workspace.id == self.selected_workspace_id)
                        {
                            self.selected_workspace_id = self
                                .user
                                .as_ref()
                                .map(|user| user.personal_workspace_id.clone())
                                .filter(|id| {
                                    self.workspaces.iter().any(|workspace| &workspace.id == id)
                                })
                                .or_else(|| {
                                    self.workspaces
                                        .first()
                                        .map(|workspace| workspace.id.clone())
                                })
                                .unwrap_or_default();
                        }
                        if initialize_expansion && !self.selected_workspace_id.is_empty() {
                            self.expanded_workspaces
                                .insert(self.selected_workspace_id.clone());
                        }
                        let current_project_id = self
                            .catalog_project
                            .as_ref()
                            .map(|project| project.id.clone());
                        self.catalog_project = catalog_project_for_workspace(
                            &self.projects,
                            &self.selected_workspace_id,
                            current_project_id.as_deref(),
                        );
                        let initial_level = should_open_initial_level
                            .then(|| {
                                initial_catalog_level(
                                    &self.projects,
                                    &self.catalog_levels,
                                    &self.selected_workspace_id,
                                )
                            })
                            .flatten();
                        self.form_error = None;
                        if let Some((project, level)) = initial_level {
                            self.open_level(ctx, project, level);
                        }
                    }
                    Err(error) => return self.handle_authenticated_api_error(error),
                }
                true
            }
            Msg::ReloadCatalog => {
                if self.request_pending || self.phase != Phase::Editor {
                    return false;
                }
                self.form_error = None;
                self.load_catalog(ctx);
                true
            }
            Msg::BeginExplorerAction(action) => {
                if self.request_pending
                    && matches!(
                        self.explorer_action,
                        Some(ExplorerAction::DeleteProject { .. })
                            | Some(ExplorerAction::DeleteLevel { .. })
                    )
                {
                    return false;
                }
                self.advance_level_intent();
                self.form_error = None;
                self.open_menu = None;
                self.palette_open = false;
                match &action {
                    ExplorerAction::CreateWorkspace => self.workspace_draft_name.clear(),
                    ExplorerAction::CreateProject { workspace_id } => {
                        self.selected_workspace_id.clone_from(workspace_id);
                        self.catalog_project = None;
                        self.project_name.clear();
                        self.expanded_workspaces.insert(workspace_id.clone());
                    }
                    ExplorerAction::CreateLevel { project_id } => {
                        if let Some(project) = self
                            .projects
                            .iter()
                            .find(|project| project.id == *project_id)
                            .cloned()
                        {
                            self.selected_workspace_id = project.workspace_id.clone();
                            self.catalog_project = Some(project);
                        }
                        self.level_name.clear();
                        self.level_duration = "0".to_owned();
                        self.expanded_projects.insert(project_id.clone());
                    }
                    ExplorerAction::DeleteProject { .. } | ExplorerAction::DeleteLevel { .. } => {}
                }
                self.explorer_action = Some(action);
                true
            }
            Msg::CancelExplorerAction => {
                self.explorer_action = None;
                self.form_error = None;
                self.advance_level_intent();
                true
            }
            Msg::ToggleWorkspace(workspace_id) => {
                if !self.expanded_workspaces.remove(&workspace_id) {
                    self.expanded_workspaces.insert(workspace_id);
                }
                true
            }
            Msg::ToggleProject(project_id) => {
                if !self.expanded_projects.remove(&project_id) {
                    self.expanded_projects.insert(project_id);
                }
                true
            }
            Msg::WorkspaceName(value) => {
                self.workspace_draft_name = value;
                self.form_error = None;
                true
            }
            Msg::CreateWorkspace => {
                if self.request_pending || self.workspace_draft_name.trim().is_empty() {
                    return false;
                }
                self.request_pending = true;
                self.form_error = None;
                let name = self.workspace_draft_name.trim().to_owned();
                let intent_generation = self.level_intent_generation;
                let link = ctx.link().clone();
                wasm_bindgen_futures::spawn_local(async move {
                    link.send_message(Msg::WorkspaceCreated {
                        intent_generation,
                        result: RestClient.create_workspace(&name).await,
                    });
                });
                true
            }
            Msg::WorkspaceCreated {
                intent_generation,
                result,
            } => {
                self.request_pending = false;
                match result {
                    Ok(workspace) => {
                        self.workspace_draft_name.clear();
                        if self.level_intent_generation == intent_generation {
                            self.selected_workspace_id = workspace.id.clone();
                            self.expanded_workspaces.insert(workspace.id);
                            self.catalog_project = None;
                            self.explorer_action = None;
                        }
                        self.push_toast("Workspace created".to_owned(), "success");
                        self.load_catalog(ctx);
                    }
                    Err(error) => return self.handle_authenticated_api_error(error),
                }
                true
            }
            Msg::SelectWorkspace(workspace_id) => {
                self.advance_level_intent();
                self.expanded_workspaces.insert(workspace_id.clone());
                self.selected_workspace_id = workspace_id;
                self.catalog_project = self
                    .projects
                    .iter()
                    .find(|project| project.workspace_id == self.selected_workspace_id)
                    .cloned();
                self.form_error = None;
                true
            }
            Msg::SelectProject(project) => {
                self.advance_level_intent();
                self.expanded_workspaces
                    .insert(project.workspace_id.clone());
                self.expanded_projects.insert(project.id.clone());
                self.selected_workspace_id = project.workspace_id.clone();
                self.catalog_project = Some(project);
                self.form_error = None;
                true
            }
            Msg::ProjectName(value) => {
                self.project_name = value;
                self.form_error = None;
                true
            }
            Msg::CreateProject => {
                let can_create_project = self.workspaces.iter().any(|workspace| {
                    workspace.id == self.selected_workspace_id
                        && matches!(workspace.role.as_str(), "owner" | "admin")
                });
                if self.request_pending
                    || self.project_name.trim().is_empty()
                    || !can_create_project
                {
                    return false;
                }
                self.request_pending = true;
                self.form_error = None;
                let workspace_id = self.selected_workspace_id.clone();
                let name = self.project_name.trim().to_owned();
                let intent_generation = self.level_intent_generation;
                let link = ctx.link().clone();
                wasm_bindgen_futures::spawn_local(async move {
                    link.send_message(Msg::ProjectCreated {
                        intent_generation,
                        result: RestClient.create_project(&workspace_id, &name).await,
                    });
                });
                true
            }
            Msg::ProjectCreated {
                intent_generation,
                result,
            } => {
                self.request_pending = false;
                match result {
                    Ok(project) => {
                        self.project_name.clear();
                        if self.level_intent_generation == intent_generation {
                            self.selected_workspace_id = project.workspace_id.clone();
                            self.expanded_workspaces
                                .insert(project.workspace_id.clone());
                            self.expanded_projects.insert(project.id.clone());
                            self.catalog_project = Some(project.clone());
                            self.explorer_action = None;
                        }
                        self.projects.push(project.clone());
                        self.catalog_levels.entry(project.id).or_default();
                        self.push_toast("Project created".to_owned(), "success");
                        self.load_catalog(ctx);
                    }
                    Err(error) => return self.handle_authenticated_api_error(error),
                }
                true
            }
            Msg::DeleteProject(project_id) => {
                if self.request_pending {
                    return false;
                }
                self.request_pending = true;
                self.form_error = None;
                let request_project_id = project_id.clone();
                let link = ctx.link().clone();
                wasm_bindgen_futures::spawn_local(async move {
                    link.send_message(Msg::ProjectDeleted {
                        project_id: request_project_id.clone(),
                        result: RestClient.delete_project(&request_project_id).await,
                    });
                });
                true
            }
            Msg::ProjectDeleted { project_id, result } => {
                self.request_pending = false;
                match result {
                    Ok(()) => {
                        let deleted_active = self.target.project_id.as_str() == project_id;
                        self.projects.retain(|project| project.id != project_id);
                        self.catalog_levels.remove(&project_id);
                        self.expanded_projects.remove(&project_id);
                        if self
                            .catalog_project
                            .as_ref()
                            .is_some_and(|project| project.id == project_id)
                        {
                            self.catalog_project = None;
                        }
                        self.explorer_action = None;
                        if deleted_active {
                            self.unbind_level();
                        }
                        self.push_toast("Project deleted".to_owned(), "success");
                        self.load_catalog(ctx);
                    }
                    Err(error) => return self.handle_authenticated_api_error(error),
                }
                true
            }
            Msg::LevelName(value) => {
                self.level_name = value;
                self.form_error = None;
                true
            }
            Msg::LevelDuration(value) => {
                self.level_duration = value;
                self.form_error = None;
                true
            }
            Msg::CreateLevel => {
                let Some(project) = self.catalog_project.clone() else {
                    return false;
                };
                let Ok(duration_seconds) = self.level_duration.parse::<f64>() else {
                    self.form_error = Some("Duration must be a nonnegative number".to_owned());
                    return true;
                };
                if self.request_pending
                    || self.level_name.trim().is_empty()
                    || !duration_seconds.is_finite()
                    || duration_seconds < 0.0
                    || !project.can_edit_timeline()
                {
                    return false;
                }
                self.request_pending = true;
                self.form_error = None;
                let name = self.level_name.trim().to_owned();
                let project_id = project.id.clone();
                let intent_generation = self.level_intent_generation;
                let link = ctx.link().clone();
                wasm_bindgen_futures::spawn_local(async move {
                    link.send_message(Msg::LevelCreated {
                        project,
                        intent_generation,
                        result: RestClient
                            .create_level(&project_id, &name, duration_seconds)
                            .await,
                    });
                });
                true
            }
            Msg::LevelCreated {
                project,
                intent_generation,
                result,
            } => {
                self.request_pending = false;
                match result {
                    Ok(level) => {
                        self.level_name.clear();
                        self.level_duration = "0".to_owned();
                        self.catalog_levels
                            .entry(project.id.clone())
                            .or_default()
                            .push(level.clone());
                        if self.phase == Phase::Editor
                            && self.level_intent_generation == intent_generation
                        {
                            self.explorer_action = None;
                            self.open_level(ctx, project, level);
                        } else {
                            self.push_toast(
                                "Level created. Select it from the Explorer.".to_owned(),
                                "success",
                            );
                            if self.phase == Phase::Editor {
                                self.load_catalog(ctx);
                            }
                        }
                    }
                    Err(error) => return self.handle_authenticated_api_error(error),
                }
                true
            }
            Msg::DeleteLevel {
                project_id,
                level_id,
            } => {
                if self.request_pending {
                    return false;
                }
                self.request_pending = true;
                self.form_error = None;
                let request_project_id = project_id.clone();
                let request_level_id = level_id.clone();
                let link = ctx.link().clone();
                wasm_bindgen_futures::spawn_local(async move {
                    link.send_message(Msg::LevelDeleted {
                        project_id: request_project_id.clone(),
                        level_id: request_level_id.clone(),
                        result: RestClient
                            .delete_level(&request_project_id, &request_level_id)
                            .await,
                    });
                });
                true
            }
            Msg::LevelDeleted {
                project_id,
                level_id,
                result,
            } => {
                self.request_pending = false;
                match result {
                    Ok(()) => {
                        let deleted_active = self.target.project_id.as_str() == project_id
                            && self.target.level_id.as_str() == level_id;
                        if let Some(levels) = self.catalog_levels.get_mut(&project_id) {
                            levels.retain(|level| level.id != level_id);
                        }
                        self.explorer_action = None;
                        if deleted_active {
                            self.unbind_level();
                        }
                        self.push_toast("Level deleted".to_owned(), "success");
                        self.load_catalog(ctx);
                    }
                    Err(error) => return self.handle_authenticated_api_error(error),
                }
                true
            }
            Msg::OpenLevel(project, level) => {
                self.open_level(ctx, project, level);
                true
            }
            Msg::OpenLevelConfiguration => {
                if self.phase != Phase::Editor || self.target.project_id.as_str().is_empty() {
                    return false;
                }
                self.right_tab = RightTab::LevelConfiguration;
                self.level_configuration_open = true;
                self.level_configuration_pending = true;
                self.level_configuration_error = None;
                self.level_configuration_name = self.level_display_name.clone();
                self.level_configuration_duration = self.level_duration_seconds.to_string();
                self.level_configuration_generation =
                    self.level_configuration_generation.wrapping_add(1);
                let generation = self.level_configuration_generation;
                let target = self.target.clone();
                let project_id = target.project_id.to_string();
                let level_id = target.level_id.to_string();
                let link = ctx.link().clone();
                wasm_bindgen_futures::spawn_local(async move {
                    link.send_message(Msg::LevelConfigurationLoaded {
                        target,
                        generation,
                        result: RestClient.level_configuration(&project_id, &level_id).await,
                    });
                });
                true
            }
            Msg::CloseLevelConfiguration => {
                self.reset_level_configuration();
                true
            }
            Msg::LevelConfigurationName(value) => {
                self.level_configuration_name = value;
                true
            }
            Msg::LevelConfigurationDuration(value) => {
                self.level_configuration_duration = value;
                true
            }
            Msg::SaveLevelConfiguration => self.save_level_configuration(ctx),
            Msg::LevelConfigurationLoaded {
                target,
                generation,
                result,
            } => {
                if !self.level_configuration_open
                    || target != self.target
                    || generation != self.level_configuration_generation
                {
                    return false;
                }
                self.level_configuration_pending = false;
                match result {
                    Ok(configuration) => self.accept_level_configuration(configuration),
                    Err(error) if error.is_unauthorized() => {
                        return self.handle_authenticated_api_error(error);
                    }
                    Err(error) => {
                        self.level_configuration_error = Some(api_error_message(&error));
                    }
                }
                true
            }
            Msg::LevelConfigurationUpdated {
                target,
                generation,
                result,
            } => {
                if !self.level_configuration_open
                    || target != self.target
                    || generation != self.level_configuration_generation
                {
                    return false;
                }
                self.level_configuration_pending = false;
                match result {
                    Ok(configuration) => {
                        self.accept_level_configuration(configuration);
                        self.push_toast("Level configuration saved".to_owned(), "success");
                    }
                    Err(error) if error.is_unauthorized() => {
                        return self.handle_authenticated_api_error(error);
                    }
                    Err(error) => {
                        self.level_configuration_error = Some(api_error_message(&error));
                    }
                }
                true
            }
            Msg::OpenProjectConfiguration(project) => {
                self.explorer_action = None;
                self.advance_level_intent();
                self.configuration_project = Some(project.clone());
                self.project_configuration = None;
                self.configuration_pending = true;
                self.configuration_error = None;
                self.invitation_email.clear();
                let project_id = project.id.clone();
                let request_project_id = project_id.clone();
                let link = ctx.link().clone();
                wasm_bindgen_futures::spawn_local(async move {
                    link.send_message(Msg::ProjectConfigurationLoaded {
                        project_id: request_project_id,
                        result: RestClient.project_configuration(&project_id).await,
                    });
                });
                true
            }
            Msg::ProjectConfigurationLoaded { project_id, result } => {
                if self
                    .configuration_project
                    .as_ref()
                    .map(|project| project.id.as_str())
                    != Some(project_id.as_str())
                {
                    return false;
                }
                self.configuration_pending = false;
                match result {
                    Ok(configuration) => {
                        self.configuration_project = Some(configuration.project.clone());
                        self.project_configuration = Some(configuration);
                        self.configuration_error = None;
                    }
                    Err(error) => {
                        if error.is_unauthorized() {
                            return self.handle_authenticated_api_error(error);
                        }
                        self.configuration_error = Some(api_error_message(&error));
                    }
                }
                true
            }
            Msg::CloseProjectConfiguration => {
                self.configuration_project = None;
                self.project_configuration = None;
                self.configuration_error = None;
                true
            }
            Msg::InvitationEmail(email) => {
                self.invitation_email = email;
                true
            }
            Msg::InvitationRole(role) => {
                self.invitation_role = role;
                true
            }
            Msg::InviteProjectMember => self.invite_project_member(ctx),
            Msg::ProjectMemberInvited { project_id, result } => {
                if self
                    .configuration_project
                    .as_ref()
                    .map(|project| project.id.as_str())
                    != Some(project_id.as_str())
                {
                    return false;
                }
                self.configuration_pending = false;
                match result {
                    Ok(invitation) => {
                        if let Some(configuration) = self.project_configuration.as_mut() {
                            configuration.invitations.push(invitation);
                        }
                        self.invitation_email.clear();
                        self.configuration_error = None;
                    }
                    Err(error) => self.configuration_error = Some(api_error_message(&error)),
                }
                true
            }
            Msg::AcceptProjectInvitation(invitation) => {
                if self.request_pending {
                    return false;
                }
                self.request_pending = true;
                let project_id = invitation.project_id;
                let invitation_id = invitation.id;
                let link = ctx.link().clone();
                wasm_bindgen_futures::spawn_local(async move {
                    link.send_message(Msg::ProjectInvitationAccepted(
                        RestClient
                            .accept_project_invitation(&project_id, &invitation_id)
                            .await,
                    ));
                });
                true
            }
            Msg::ProjectInvitationAccepted(result) => {
                self.request_pending = false;
                match result {
                    Ok(_) => self.load_catalog(ctx),
                    Err(error) => return self.handle_authenticated_api_error(error),
                }
                true
            }
            Msg::SetProjectMemberRole(member_id, role) => {
                let Some(project) = self.configuration_project.as_ref() else {
                    return false;
                };
                if self.configuration_pending || !project.can_manage_members() {
                    return false;
                }
                self.configuration_pending = true;
                let project_id = project.id.clone();
                let request_project_id = project_id.clone();
                let roles = vec![role];
                let link = ctx.link().clone();
                wasm_bindgen_futures::spawn_local(async move {
                    link.send_message(Msg::ProjectMemberUpdated {
                        project_id: request_project_id,
                        result: RestClient
                            .update_project_member(&project_id, &member_id, &roles)
                            .await,
                    });
                });
                true
            }
            Msg::ProjectMemberUpdated { project_id, result } => {
                if self
                    .configuration_project
                    .as_ref()
                    .map(|project| project.id.as_str())
                    != Some(project_id.as_str())
                {
                    return false;
                }
                self.configuration_pending = false;
                match result {
                    Ok(member) => {
                        if self
                            .user
                            .as_ref()
                            .is_some_and(|user| user.id == member.user_id)
                        {
                            if let Some(project) = self.configuration_project.as_mut() {
                                project.roles.clone_from(&member.roles);
                                project.capabilities.clone_from(&member.capabilities);
                            }
                            if let Some(project) = self
                                .projects
                                .iter_mut()
                                .find(|project| project.id == project_id)
                            {
                                project.roles.clone_from(&member.roles);
                                project.capabilities.clone_from(&member.capabilities);
                            }
                            if let Some(project) = self
                                .catalog_project
                                .as_mut()
                                .filter(|project| project.id == project_id)
                            {
                                project.roles.clone_from(&member.roles);
                                project.capabilities.clone_from(&member.capabilities);
                            }
                        }
                        if let Some(existing) =
                            self.project_configuration
                                .as_mut()
                                .and_then(|configuration| {
                                    configuration
                                        .members
                                        .iter_mut()
                                        .find(|existing| existing.user_id == member.user_id)
                                })
                        {
                            *existing = member;
                        }
                        self.configuration_error = None;
                    }
                    Err(error) => self.configuration_error = Some(api_error_message(&error)),
                }
                true
            }
            Msg::SetProjectDefaultTheme(default_theme) => {
                let Some(project) = self.configuration_project.as_ref() else {
                    return false;
                };
                if self.configuration_pending || !project.can_manage_theme() {
                    return false;
                }
                self.configuration_pending = true;
                let project_id = project.id.clone();
                let request_project_id = project_id.clone();
                let link = ctx.link().clone();
                wasm_bindgen_futures::spawn_local(async move {
                    link.send_message(Msg::ProjectConfigurationUpdated {
                        project_id: request_project_id,
                        result: RestClient
                            .update_project_configuration(&project_id, &default_theme)
                            .await,
                    });
                });
                true
            }
            Msg::ProjectConfigurationUpdated { project_id, result } => {
                if self
                    .configuration_project
                    .as_ref()
                    .map(|project| project.id.as_str())
                    != Some(project_id.as_str())
                {
                    return false;
                }
                self.configuration_pending = false;
                match result {
                    Ok(project) => {
                        self.configuration_project = Some(project.clone());
                        if let Some(configuration) = self.project_configuration.as_mut() {
                            configuration.project = project.clone();
                        }
                        if let Some(existing) = self
                            .projects
                            .iter_mut()
                            .find(|existing| existing.id == project.id)
                        {
                            existing.clone_from(&project);
                        }
                        if let Some(selected) = self
                            .catalog_project
                            .as_mut()
                            .filter(|selected| selected.id == project.id)
                        {
                            selected.clone_from(&project);
                        }
                        self.configuration_error = None;
                    }
                    Err(error) => self.configuration_error = Some(api_error_message(&error)),
                }
                true
            }
            Msg::Logout => {
                if self.request_pending {
                    return false;
                }
                self.request_pending = true;
                self.invalidate_rpc();
                let link = ctx.link().clone();
                wasm_bindgen_futures::spawn_local(async move {
                    link.send_message(Msg::LogoutFinished(RestClient.logout().await));
                });
                true
            }
            Msg::LogoutFinished(result) => {
                self.request_pending = false;
                if let Err(error) = result
                    && !error.is_unauthorized()
                {
                    self.form_error = Some(api_error_message(&error));
                }
                self.clear_session();
                true
            }
            Msg::SetMode(mode) => {
                if self.mode == mode {
                    self.set_workspace_tool(WorkspaceTool::default_for(mode))
                } else {
                    self.set_mode(mode)
                }
            }
            Msg::SetMapPaintKind(kind) => {
                let mut changed = self.set_mode(Mode::Map);
                changed |= self.set_workspace_tool(WorkspaceTool::Cells);
                if self.map_paint_kind != Some(kind) {
                    self.map_paint_kind = Some(kind);
                    changed = true;
                }
                changed
            }
            Msg::SetPlacementContext(kind) => {
                let mut changed = self.set_mode(Mode::Select);
                changed |= self.set_workspace_tool(WorkspaceTool::Place);
                if self.selected_placement_kind != kind {
                    self.selected_placement_kind = kind;
                    changed = true;
                }
                if self.left_tab != LeftTab::ShapeTemplates {
                    self.left_tab = LeftTab::ShapeTemplates;
                    changed = true;
                }
                if self.studio_modal.is_some() {
                    self.studio_modal = None;
                    changed = true;
                }
                changed
            }
            Msg::OpenSandStudio => self.open_sand_studio(),
            Msg::SetBlindColor(color_index) => {
                if self.mode != Mode::Brush
                    || !(1..=10).contains(&color_index)
                    || self.blind_color_index == color_index
                {
                    false
                } else {
                    self.blind_color_index = color_index;
                    true
                }
            }
            Msg::OpenStudio(studio) => {
                self.studio_modal = Some(studio);
                if studio == StudioModal::Image {
                    self.load_image_catalog(ctx);
                    self.load_prepared_catalog(ctx);
                }
                true
            }
            Msg::CloseStudio => {
                self.cancel_conversion();
                self.studio_modal = None;
                true
            }
            Msg::FocusExplorerEntity(entity_id) => {
                if self
                    .model
                    .timeline()
                    .snapshot()
                    .entity(&entity_id)
                    .is_none()
                {
                    return false;
                }
                self.select_entities_and_focus_inspector(vec![entity_id]);
                self.canvas_dirty = true;
                true
            }
            Msg::EditEntity(action) => self.edit_selected_entity(ctx, action),
            Msg::EditDecorator(action) => self.edit_selected_decorator(ctx, action),
            Msg::SetIceCount(value) => self.set_selected_ice_count(ctx, &value),
            Msg::SetGlassCount(value) => self.set_selected_glass_count(ctx, &value),
            Msg::SetPoolResolution(pixels_per_cell) => {
                self.set_pool_resolution(ctx, pixels_per_cell)
            }
            Msg::SetBlockLayerCapacity(layer_index, value) => {
                self.set_block_layer_capacity(ctx, layer_index, &value)
            }
            Msg::AddBlockCollectLayer => self.add_block_collect_layer(ctx),
            Msg::RemoveBlockCollectLayer(layer_index) => {
                self.remove_block_collect_layer(ctx, layer_index)
            }
            Msg::ResizeWidth(value) => {
                self.resize_width = value;
                true
            }
            Msg::ResizeHeight(value) => {
                self.resize_height = value;
                true
            }
            Msg::SetResizeAnchor(anchor) => {
                self.resize_anchor = anchor;
                true
            }
            Msg::ResizeGrid => self.resize_grid(ctx),
            Msg::ZoomIn => self.zoom_canvas(1.2),
            Msg::ZoomOut => self.zoom_canvas(1.0 / 1.2),
            Msg::FrameGrid => {
                self.frame_grid();
                true
            }
            Msg::CanvasWheel(event) => self.canvas_wheel(event),
            Msg::ShapeCatalogLoaded { project_id, result } => {
                if self.target.project_id.as_str() != project_id {
                    return false;
                }
                self.shape_catalog_pending = false;
                match result {
                    Ok(shapes) => {
                        self.shape_catalog = shapes;
                        let active_is_available =
                            self.active_shape_template_id.as_ref().is_some_and(|id| {
                                self.shape_catalog.iter().any(|entry| &entry.id == id)
                            });
                        if !active_is_available {
                            self.active_shape_template_id =
                                self.shape_catalog.first().map(|entry| entry.id.clone());
                        }
                    }
                    Err(error) => return self.handle_authenticated_api_error(error),
                }
                true
            }
            Msg::SetShapeTemplateView(view) => {
                self.shape_template_view = view;
                true
            }
            Msg::ShapeDraftName(value) => {
                self.shape_draft_name = value;
                true
            }
            Msg::ToggleShapeCell(x, y) => {
                self.shape_draft_mask ^= 1_u64 << (u32::from(x) + u32::from(y) * 8);
                true
            }
            Msg::NewShapeDraft => {
                self.selected_shape_id = None;
                self.shape_draft_name.clear();
                self.shape_draft_mask = 1;
                true
            }
            Msg::SelectShape(shape_id) => self.select_shape_draft(&shape_id),
            Msg::SelectShapeTemplate(shape_id) => {
                if !self.shape_catalog.iter().any(|entry| entry.id == shape_id) {
                    return false;
                }
                self.active_shape_template_id = Some(shape_id);
                self.canvas_dirty = true;
                true
            }
            Msg::SaveShapeDraft => self.save_shape_draft(ctx, false),
            Msg::UpdateShapeDraft => self.save_shape_draft(ctx, true),
            Msg::ShapeSaved { project_id, result } => {
                if self.target.project_id.as_str() != project_id {
                    return false;
                }
                self.shape_catalog_pending = false;
                match result {
                    Ok(saved) => {
                        if let Some(existing) = self
                            .shape_catalog
                            .iter_mut()
                            .find(|entry| entry.id == saved.id)
                        {
                            *existing = saved.clone();
                        } else {
                            self.shape_catalog.push(saved.clone());
                        }
                        self.selected_shape_id = Some(saved.id.clone());
                        self.active_shape_template_id = Some(saved.id);
                        self.push_toast("Project shape saved".to_owned(), "success");
                    }
                    Err(error) => return self.handle_authenticated_api_error(error),
                }
                true
            }
            Msg::DeleteShapeDraft => self.delete_shape_draft(ctx),
            Msg::ShapeDeleted {
                project_id,
                shape_id,
                result,
            } => {
                if self.target.project_id.as_str() != project_id {
                    return false;
                }
                self.shape_catalog_pending = false;
                match result {
                    Ok(()) => {
                        self.shape_catalog.retain(|entry| entry.id != shape_id);
                        self.selected_shape_id = None;
                        if self.active_shape_template_id.as_ref() == Some(&shape_id) {
                            self.active_shape_template_id =
                                self.shape_catalog.first().map(|entry| entry.id.clone());
                        }
                        self.shape_draft_name.clear();
                        self.shape_draft_mask = 1;
                        self.push_toast("Project shape deleted".to_owned(), "success");
                    }
                    Err(error) => return self.handle_authenticated_api_error(error),
                }
                true
            }
            Msg::ImageCatalogLoaded { scope, project_id, result } => {
                if self.target.project_id.as_str() != project_id || scope != self.image_scope().0 {
                    return false;
                }
                self.image_catalog_pending = false;
                match result {
                    Ok(images) => self.image_catalog = images,
                    Err(error) => {
                        self.image_import_error = Some(api_error_message(&error));
                        return self.handle_authenticated_api_error(error);
                    }
                }
                true
            }
            Msg::ImageDraftName(value) => {
                self.image_draft_name = value;
                self.image_import_error = None;
                true
            }
            Msg::ImageFileSelected(file) => {
                let changed = self.select_image_file(file);
                if self.image_file.is_some() { self.import_image(ctx); }
                changed
            }
            Msg::ImagePasted(event) => {
                let changed = self.select_pasted_image(event);
                if changed && self.image_file.is_some() { self.import_image(ctx); }
                changed
            }
            Msg::ImportImage => self.import_image(ctx),
            Msg::ImageImported { scope, generation, project_id, result } => {
                if self.target.project_id.as_str() != project_id || scope != self.image_scope().0 {
                    return false;
                }
                self.image_catalog_pending = false;
                match result {
                    Ok(image) => {
                        let source_id = image.id.clone();
                        self.image_catalog.push(image);
                        if generation == self.image_scope().1 && self.studio_modal == Some(StudioModal::Image) {
                            self.image_studio_message(ctx, ImageMsg::Convert(source_id));
                        }
                        self.image_file = None;
                        if let Some(input) = self.image_file_input_ref.cast::<HtmlInputElement>() {
                            input.set_value("");
                        }
                        self.image_draft_name.clear();
                        self.image_import_error = None;
                        self.push_toast("Shared image template imported".to_owned(), "success");
                    }
                    Err(error) => {
                        self.image_import_error = Some(api_error_message(&error));
                        return self.handle_authenticated_api_error(error);
                    }
                }
                true
            }
            Msg::BeginPlacementDrag(kind, shape) => self.begin_placement_drag(kind, shape),
            Msg::EndPlacementDrag => self.end_placement_drag(),
            Msg::CanvasDragOver(event) => self.canvas_drag_over(event),
            Msg::CanvasDrop(event) => self.canvas_drop(ctx, event),
            Msg::CanvasResized(width, height) => self.canvas_resized(width, height),
            Msg::CanvasDown(event) => self.canvas_down(ctx, event),
            Msg::CanvasMove(event) => self.canvas_move(ctx, event),
            Msg::CanvasUp(event) => self.canvas_up(ctx, event),
            Msg::CanvasCancel(event) => self.canvas_cancel(event),
            Msg::CanvasLeave => self.queue_cursor(ctx, None),
            Msg::KeyDown(event) => self.key_down(ctx, event),
            Msg::ToggleTheme => {
                self.theme.toggle();
                self.open_menu = None;
                self.canvas_dirty = true;
                self.push_toast(
                    format!(
                        "Using {} theme (user override)",
                        self.theme.active().label()
                    ),
                    "info",
                );
                true
            }
            Msg::UseProjectTheme => {
                self.theme.use_project_default();
                self.canvas_dirty = true;
                self.open_menu = None;
                self.push_toast("Using project-default theme".to_owned(), "info");
                true
            }
            Msg::SetUiFontSize(value) => {
                let changed = self.ui_settings.set_font_size(&value);
                self.canvas_dirty |= changed;
                changed
            }
            Msg::SetUiScale(value) => {
                let changed = self.ui_settings.set_scale(&value);
                self.canvas_dirty |= changed;
                changed
            }
            Msg::ResetUiSettings => {
                let changed = self.ui_settings.reset();
                self.canvas_dirty |= changed;
                changed
            }
            Msg::Undo => self.undo(ctx),
            Msg::ToggleMenu(menu) => {
                self.open_menu = (self.open_menu != Some(menu)).then_some(menu);
                self.palette_open = false;
                true
            }
            Msg::SetLeftTab(tab) => {
                if (tab == LeftTab::SandStudio && self.mode != Mode::Brush)
                    || (tab == LeftTab::ApplyImage && self.image_studio.apply.is_none()) || self.left_tab == tab
                {
                    false
                } else {
                    self.left_tab = tab;
                    true
                }
            }
            Msg::SetRightTab(tab) => {
                let changed = self.right_tab != tab;
                self.right_tab = tab;
                if tab == RightTab::LevelConfiguration && !self.level_configuration_open {
                    ctx.link().send_message(Msg::OpenLevelConfiguration);
                }
                changed
            }
            Msg::SetWorkspaceTool(tool) => self.set_workspace_tool(tool),
            Msg::BeginSidebarResize(side, event) => self.begin_sidebar_resize(side, event),
            Msg::SidebarResizeMove(event) => self.resize_sidebar(event),
            Msg::EndSidebarResize(event) => self.end_sidebar_resize(event),
            Msg::SetPanelLayout(side, layout) => self.set_panel_layout(side, layout),
            Msg::TogglePanelLayout(side) => {
                let layout = match side {
                    SidebarSide::Left => self.left_panel_layout,
                    SidebarSide::Right => self.right_panel_layout,
                };
                self.set_panel_layout(side, layout.toggled())
            }
            Msg::ToggleInspectorSection(section) => self.toggle_inspector_section(section),
            Msg::TogglePalette => {
                self.palette_open = !self.palette_open;
                self.palette_query.clear();
                self.open_menu = None;
                true
            }
            Msg::CloseOverlays => {
                let had_overlay = self.palette_open || self.open_menu.is_some();
                self.palette_open = false;
                self.open_menu = None;
                had_overlay
            }
            Msg::PaletteQuery(query) => {
                self.palette_query = query;
                true
            }
            Msg::RunPalette(command) => self.run_palette(ctx, command),
            Msg::SaveDraft => {
                self.persist_draft();
                self.open_menu = None;
                self.push_toast("Local draft snapshot saved".to_owned(), "success");
                true
            }
            Msg::ResetDraft => {
                self.open_menu = None;
                self.push_toast(
                    "Reset is unavailable because live levels only accept timeline commands"
                        .to_owned(),
                    "warning",
                );
                true
            }
            Msg::Reconnect => {
                self.reconnect_failures = 0;
                self.start_connection(ctx);
                true
            }
            Msg::RetryConnection(attempt) => {
                if attempt != self.connection_attempt || self.phase != Phase::Editor {
                    return false;
                }
                self.reconnect_timer = None;
                self.start_connection(ctx);
                true
            }
            Msg::DismissToast(id) => {
                self.active_toasts.remove(&id);
                true
            }
            Msg::ToggleNotifications => {
                self.notifications_open = !self.notifications_open;
                self.open_menu = None;
                self.palette_open = false;
                true
            }
            Msg::ClearNotifications => {
                self.toasts.clear();
                self.active_toasts.clear();
                true
            }
            Msg::RpcReady { attempt, result } => {
                if attempt != self.connection_attempt {
                    return false;
                }
                match result {
                    Ok(client) => {
                        self.rpc = Some(client);
                        self.rpc_state = RpcState::Online;
                        self.reconnect_failures = 0;
                        self.reconnect_timer = None;
                        if self.desired_cursor.is_some() {
                            self.schedule_cursor_flush(ctx);
                        }
                        self.push_toast(
                            format!("Collaboration connected for {}", self.level_display_name),
                            "success",
                        );
                    }
                    Err(error) => {
                        self.schedule_reconnect(ctx, error);
                    }
                }
                true
            }
            Msg::RpcUpdate { attempt, update } => {
                if attempt != self.connection_attempt {
                    return false;
                }
                self.handle_rpc_update(ctx, *update)
            }
            Msg::RpcApplyFinished {
                attempt,
                command_id,
                result,
            } => {
                if attempt != self.connection_attempt {
                    return false;
                }
                self.handle_apply_response(ctx, command_id, *result)
            }
            Msg::RpcUndoFinished {
                attempt,
                command_id,
                result,
            } => {
                if attempt != self.connection_attempt {
                    return false;
                }
                self.handle_undo_response(ctx, command_id, *result)
            }
            Msg::FlushCursor(attempt) => {
                if attempt != self.connection_attempt {
                    return false;
                }
                self.cursor_timer = None;
                self.flush_cursor(ctx);
                false
            }
            Msg::CursorSent {
                attempt,
                cursor,
                result,
            } => {
                if attempt != self.connection_attempt {
                    return false;
                }
                self.cursor_in_flight = false;
                if result.is_ok() {
                    self.last_sent_cursor = Some(cursor);
                }
                if self.last_sent_cursor != Some(self.desired_cursor) {
                    self.schedule_cursor_flush(ctx);
                }
                false
            }
        }
    }

    fn rendered(&mut self, ctx: &Context<Self>, first_render: bool) {
        if first_render {
            if let Some(root) = self.root_ref.cast::<HtmlElement>() {
                let _ = root.focus();
            }
        }
        if self.palette_open {
            if let Some(input) = self.palette_input_ref.cast::<HtmlInputElement>() {
                let _ = input.focus();
            }
        }
        if self.phase == Phase::Editor {
            self.draw_division_canvas();
            self.ensure_canvas_resize_observer(ctx);
            if let Some(canvas) = self.canvas_ref.cast::<HtmlCanvasElement>() {
                self.canvas_resized(
                    u32::try_from(canvas.client_width().max(1)).unwrap_or(CANVAS_WIDTH),
                    u32::try_from(canvas.client_height().max(1)).unwrap_or(CANVAS_HEIGHT),
                );
            }
            if self.canvas_dirty {
                let _ = self.draw_canvas();
                self.canvas_dirty = false;
            }
        }
    }

    fn view(&self, ctx: &Context<Self>) -> Html {
        let active_theme = self.theme.active();
        if self.phase != Phase::Editor {
            return html! {
                <div class="access-shell" data-theme={active_theme.name()}>
                    {
                        match self.phase {
                            Phase::Bootstrapping => self.view_bootstrapping(),
                            Phase::Authentication => self.view_authentication(ctx),
                            Phase::Editor => Html::default(),
                        }
                    }
                </div>
            };
        }
        let shell_style = self
            .ui_settings
            .shell_style(self.left_sidebar_width, self.right_sidebar_width);
        html! {
            <div
                class={classes!(
                    "app-shell",
                    self.sidebar_resize.is_some().then_some("resizing-sidebar"),
                    (self.left_panel_layout != PanelLayout::Docked).then_some("left-overlay"),
                    (self.right_panel_layout != PanelLayout::Docked).then_some("right-overlay"),
                )}
                data-theme={active_theme.name()}
                style={shell_style}
                tabindex="0"
                ref={self.root_ref.clone()}
                onkeydown={ctx.link().callback(Msg::KeyDown)}
                onpaste={ctx.link().callback(|event: Event| Msg::ImagePasted(event.unchecked_into()))}
                onpointermove={ctx.link().callback(Msg::SidebarResizeMove)}
                onpointerup={ctx.link().callback(Msg::EndSidebarResize)}
                onpointercancel={ctx.link().callback(Msg::EndSidebarResize)}
            >
                { self.view_header(ctx) }
                { if self.left_panel_layout == PanelLayout::Hidden { Html::default() } else { self.view_left_sidebar(ctx) } }
                { if self.left_panel_layout == PanelLayout::Docked { html! { <div class="sidebar-resizer left-resizer" title="Resize left sidebar" onpointerdown={ctx.link().callback(|event| Msg::BeginSidebarResize(SidebarSide::Left, event))}></div> } } else { Html::default() } }
                { self.view_canvas(ctx) }
                { if self.right_panel_layout == PanelLayout::Docked { html! { <div class="sidebar-resizer right-resizer" title="Resize right sidebar" onpointerdown={ctx.link().callback(|event| Msg::BeginSidebarResize(SidebarSide::Right, event))}></div> } } else { Html::default() } }
                { if self.right_panel_layout == PanelLayout::Hidden { Html::default() } else { self.view_right_sidebar(ctx) } }
                { self.view_status(ctx) }
                { self.view_palette(ctx) }
                { self.view_toasts(ctx) }
                { self.view_notifications(ctx) }
                { self.view_studio_modal(ctx) }
                { self.view_pool_division(ctx) }
                { self.view_project_configuration(ctx) }
            </div>
        }
    }
}

impl App {
    fn view_bootstrapping(&self) -> Html {
        html! {
            <main class="access-stage bootstrap-stage">
                <div class="access-brand"><span class="mark-glyph">{"OR"}</span><strong>{"OREAK"}</strong></div>
                <div class="bootstrap-readout"><span></span><code>{"SESSION / VERIFYING"}</code></div>
            </main>
        }
    }

    fn view_authentication(&self, ctx: &Context<Self>) -> Html {
        let submit_label = match self.auth_mode {
            AuthMode::Login => "Open session",
            AuthMode::Register => "Create account",
        };
        html! {
            <main class="access-stage">
                <section class="access-intro">
                    <div class="access-brand"><span class="mark-glyph">{"OR"}</span><strong>{"OREAK"}</strong></div>
                    <p class="eyebrow">{"COLLABORATIVE LEVEL SYSTEM / WEB NODE"}</p>
                    <h1>{"Enter the technical editor."}</h1>
                    <p>{"Authenticate against the same-origin session service. Credentials stay in the HttpOnly server session; the editor never reads the token."}</p>
                    <div class="protocol-grid">
                        <span>{"REST"}<strong>{"CATALOG"}</strong></span>
                        <span>{"RPC"}<strong>{"TIMELINE"}</strong></span>
                        <span>{"CORE"}<strong>{"VALIDATION"}</strong></span>
                    </div>
                </section>
                <form class="access-panel" onsubmit={ctx.link().callback(|event: SubmitEvent| {
                    event.prevent_default();
                    Msg::SubmitAuth
                })}>
                    <header><small>{"ACCESS CONTROL"}</small><code>{"A-01"}</code></header>
                    <div class="auth-tabs">
                        <button type="button" class={if self.auth_mode == AuthMode::Login { "active" } else { "" }} onclick={ctx.link().callback(|_| Msg::SetAuthMode(AuthMode::Login))}>{"Sign in"}</button>
                        <button type="button" class={if self.auth_mode == AuthMode::Register { "active" } else { "" }} onclick={ctx.link().callback(|_| Msg::SetAuthMode(AuthMode::Register))}>{"Register"}</button>
                    </div>
                    <label class="technical-field"><span>{"EMAIL"}</span><input type="email" autocomplete="email" value={self.auth_email.clone()} oninput={ctx.link().callback(|event: InputEvent| Msg::AuthEmail(event.target_unchecked_into::<HtmlInputElement>().value()))} /></label>
                    <label class="technical-field"><span>{"PASSWORD"}</span><input type="password" autocomplete={if self.auth_mode == AuthMode::Login { "current-password" } else { "new-password" }} value={self.auth_password.clone()} oninput={ctx.link().callback(|event: InputEvent| Msg::AuthPassword(event.target_unchecked_into::<HtmlInputElement>().value()))} /></label>
                    {
                        self.form_error.as_ref().map(|error| html! { <div class="access-error"><strong>{"REQUEST FAILED"}</strong><span>{error}</span></div> }).unwrap_or_default()
                    }
                    <button type="submit" class="primary-action" disabled={self.request_pending || self.auth_email.is_empty() || self.auth_password.is_empty()}>
                        <span>{if self.request_pending { "Working..." } else { submit_label }}</span><code>{">_"}</code>
                    </button>
                    <footer><span>{"COOKIE / HTTPONLY"}</span><span>{"SAMESITE / LAX"}</span></footer>
                </form>
            </main>
        }
    }

    fn view_project_configuration(&self, ctx: &Context<Self>) -> Html {
        let Some(project) = self.configuration_project.as_ref() else {
            return Html::default();
        };
        html! {
            <div class="configuration-backdrop" onclick={ctx.link().callback(|_| Msg::CloseProjectConfiguration)}>
                <section class="project-configuration" role="dialog" aria-modal="true" onclick={Callback::from(|event: MouseEvent| event.stop_propagation())}>
                    <header><div><small>{"PROJECT CONFIGURATION"}</small><strong>{project.name.clone()}</strong><code title={project.id.clone()}>{project.id.clone()}</code></div><button aria-label="Close configuration" onclick={ctx.link().callback(|_| Msg::CloseProjectConfiguration)}>{"x"}</button></header>
                    { if self.configuration_pending && self.project_configuration.is_none() { html! { <div class="configuration-loading">{"Loading authoritative project configuration..."}</div> } } else if let Some(configuration) = self.project_configuration.as_ref() { html! {
                        <div class="configuration-body">
                            <section><div class="configuration-heading"><span>{"DEFAULT THEME"}</span><code>{configuration.project.default_theme.clone()}</code></div><div class="configuration-theme"><button class={classes!((configuration.project.default_theme == "dark").then_some("active"))} disabled={self.configuration_pending || !project.can_manage_theme()} onclick={ctx.link().callback(|_| Msg::SetProjectDefaultTheme("dark".to_owned()))}>{"Dark"}</button><button class={classes!((configuration.project.default_theme == "light").then_some("active"))} disabled={self.configuration_pending || !project.can_manage_theme()} onclick={ctx.link().callback(|_| Msg::SetProjectDefaultTheme("light".to_owned()))}>{"Light"}</button></div></section>
                            <section><div class="configuration-heading"><span>{"MEMBERS"}</span><code>{configuration.members.len()}</code></div>{ if configuration.members_visible { html! { <div class="configuration-members">{ for configuration.members.iter().map(|member| {
                                let member_id = member.user_id.clone();
                                let selected_role = member.roles.first().cloned().unwrap_or_else(|| "viewer".to_owned());
                                html! { <article><div><strong>{member.email.clone().unwrap_or_else(|| member.user_id.clone())}</strong><code title={member.user_id.clone()}>{member.user_id.clone()}</code></div><select disabled={self.configuration_pending || !project.can_manage_members()} value={selected_role} onchange={ctx.link().callback(move |event: Event| Msg::SetProjectMemberRole(member_id.clone(), event.target_unchecked_into::<HtmlSelectElement>().value()))}><option value="owner">{"Owner"}</option><option value="admin">{"Admin"}</option><option value="editor">{"Editor"}</option><option value="viewer">{"Viewer"}</option></select></article> }
                            }) }</div> } } else { html! { <p class="configuration-note">{"Member roster requires manage_members permission."}</p> } } }</section>
                            { if project.can_manage_members() { html! { <section><div class="configuration-heading"><span>{"INVITE REGISTERED USER"}</span><code>{configuration.invitations.len()}</code></div><form class="configuration-invite" onsubmit={ctx.link().callback(|event: SubmitEvent| { event.prevent_default(); Msg::InviteProjectMember })}><input type="email" placeholder="person@example.com" value={self.invitation_email.clone()} oninput={ctx.link().callback(|event: InputEvent| Msg::InvitationEmail(event.target_unchecked_into::<HtmlInputElement>().value()))} /><select value={self.invitation_role.clone()} onchange={ctx.link().callback(|event: Event| Msg::InvitationRole(event.target_unchecked_into::<HtmlSelectElement>().value()))}><option value="editor">{"Editor"}</option><option value="viewer">{"Viewer"}</option><option value="admin">{"Admin"}</option></select><button type="submit" disabled={self.configuration_pending || self.invitation_email.trim().is_empty()}>{"Send invite"}</button></form><div class="pending-invites">{ for configuration.invitations.iter().map(|invitation| html! { <div><span>{invitation.invitee_email.clone()}</span><code>{invitation.roles.join(" + ")}</code></div> }) }</div></section> } } else { Html::default() } }
                        </div>
                    } } else { Html::default() } }
                    { self.configuration_error.as_ref().map(|error| html! { <div class="configuration-error">{error}</div> }).unwrap_or_default() }
                </section>
            </div>
        }
    }

    fn view_level_configuration_panel(&self, ctx: &Context<Self>) -> Html {
        if self.target.project_id.as_str().is_empty() {
            return html! {
                <div class="empty-selection panel-placeholder">
                    <span>{"NO LEVEL SELECTED"}</span>
                    <p>{"Open the workspace explorer to select or create a level."}</p>
                    <button onclick={ctx.link().callback(|_| Msg::SetLeftTab(LeftTab::Explorer))}>{"Open explorer"}</button>
                </div>
            };
        }
        if !self.level_configuration_open {
            return html! {
                <div class="empty-selection panel-placeholder">
                    <span>{"LEVEL CONFIGURATION"}</span>
                    <p>{"Load the authoritative level settings into this secondary sidebar."}</p>
                    <button onclick={ctx.link().callback(|_| Msg::OpenLevelConfiguration)}>{"Load settings"}</button>
                </div>
            };
        }
        html! {
            <section class="level-configuration sidebar-level-configuration" aria-label="Level configuration">
                <header>
                    <div><small>{"LEVEL CONFIGURATION"}</small><strong>{self.level_display_name.clone()}</strong><code>{self.target.level_id.to_string()}</code></div>
                    <button aria-label="Close level configuration" onclick={ctx.link().callback(|_| Msg::CloseLevelConfiguration)}>{"x"}</button>
                </header>
                <div class="level-configuration-body">
                    <label><span>{"NAME"}</span><input value={self.level_configuration_name.clone()} disabled={self.level_configuration_pending || !self.can_edit_timeline} oninput={ctx.link().callback(|event: InputEvent| Msg::LevelConfigurationName(event.target_unchecked_into::<HtmlInputElement>().value()))} /></label>
                    <label><span>{"DURATION / SECONDS"}</span><input type="number" min="0" step="0.1" value={self.level_configuration_duration.clone()} disabled={self.level_configuration_pending || !self.can_edit_timeline} oninput={ctx.link().callback(|event: InputEvent| Msg::LevelConfigurationDuration(event.target_unchecked_into::<HtmlInputElement>().value()))} /></label>
                    <p>{"Name and duration are authoritative project settings and do not create timeline revisions."}</p>
                </div>
                { self.level_configuration_error.as_ref().map(|error| html! { <div class="configuration-error">{error}</div> }).unwrap_or_default() }
                <footer><button onclick={ctx.link().callback(|_| Msg::CloseLevelConfiguration)}>{"Close"}</button><button class="primary-action compact" disabled={self.level_configuration_pending || !self.can_edit_timeline || self.level_configuration_name.trim().is_empty()} onclick={ctx.link().callback(|_| Msg::SaveLevelConfiguration)}>{if self.level_configuration_pending { "Saving..." } else { "Save level" }}</button></footer>
            </section>
        }
    }

    fn view_studio_modal(&self, ctx: &Context<Self>) -> Html {
        let Some(studio) = self.studio_modal else {
            return Html::default();
        };
        html! {
            <div class="studio-backdrop" onclick={ctx.link().callback(|_| Msg::CloseStudio)}>
                <section class="studio-modal" role="dialog" aria-modal="true" aria-label={studio.label()} onclick={Callback::from(|event: MouseEvent| event.stop_propagation())}>
                    <header class="studio-header"><div><small>{"FOCUSED EDITING"}</small><strong>{studio.label()}</strong></div><button aria-label="Close studio" onclick={ctx.link().callback(|_| Msg::CloseStudio)}>{"x"}</button></header>
                    {
                        match studio {
                            StudioModal::Shape => self.view_shape_studio(ctx),
                            StudioModal::Image => self.view_image_studio(ctx),
                        }
                    }
                </section>
            </div>
        }
    }

    fn view_shape_studio(&self, ctx: &Context<Self>) -> Html {
        let shape = shape_from_designer_mask(self.shape_draft_mask).ok();
        let can_use_template =
            self.can_edit_timeline && matches!(self.rpc_state, RpcState::Online) && shape.is_some();
        html! {
            <div class="shape-studio-layout">
                <aside class="shape-studio-library">
                    <div class="studio-section-heading"><span>{"SAVED SHAPES"}</span><button onclick={ctx.link().callback(|_| Msg::NewShapeDraft)}>{"New Shape"}</button></div>
                    <div class="shape-catalog">
                        { if self.shape_catalog_pending && self.shape_catalog.is_empty() {
                            html! { <span>{"Loading project shapes..."}</span> }
                        } else if self.shape_catalog.is_empty() {
                            html! { <span>{"No project shapes saved."}</span> }
                        } else {
                            html! { <>{ for self.shape_catalog.iter().map(|sample| {
                                let id = sample.id.clone();
                                html! { <button class={classes!((self.selected_shape_id.as_ref() == Some(&sample.id)).then_some("active"))} title={format!("{} x {}", sample.shape.width, sample.shape.height)} onclick={ctx.link().callback(move |_| Msg::SelectShape(id.clone()))}>{shape_template_thumbnail(sample)}<span><strong>{sample.name.clone()}</strong><code>{format!("{}x{} · {} cells", sample.shape.width, sample.shape.height, shape_occupied_count(sample))}</code></span></button> }
                            }) }</> }
                        } }
                    </div>
                </aside>
                <section class="shape-studio-editor">
                    <div class="studio-section-heading"><span>{"SHAPE EDITOR"}</span><code>{shape.map_or_else(|| "INVALID".to_owned(), |shape| format!("{}x{} / {}", shape.width(), shape.height(), shape.occupied_count()))}</code></div>
                    <div class="shape-grid studio-shape-grid" aria-label="8 by 8 shape designer">
                        { for (0_u8..64).map(|index| html! {
                            <button
                                key={index}
                                class={classes!((self.shape_draft_mask & (1_u64 << u32::from(index)) != 0).then_some("active"))}
                                aria-label={format!("Shape cell {}, {}", index % 8, index / 8)}
                                onclick={ctx.link().callback(move |_| Msg::ToggleShapeCell(index % 8, index / 8))}
                            ></button>
                        }) }
                    </div>
                    <p>{"Toggle cells to build one edge-connected footprint."}</p>
                </section>
                <aside class="shape-studio-properties">
                    <div class="studio-section-heading"><span>{"PROPERTIES"}</span></div>
                    <label class="shape-name"><span>{"Sample name"}</span><input value={self.shape_draft_name.clone()} placeholder="Connected shape" oninput={ctx.link().callback(|event: InputEvent| Msg::ShapeDraftName(event.target_unchecked_into::<HtmlInputElement>().value()))} /></label>
                    <div class="shape-save-row">
                        <button disabled={self.shape_catalog_pending || !self.can_edit_timeline || shape.is_none() || self.shape_draft_name.trim().is_empty()} onclick={ctx.link().callback(|_| Msg::SaveShapeDraft)}>{"Save new"}</button>
                        <button disabled={self.shape_catalog_pending || !self.can_edit_timeline || self.selected_shape_id.is_none() || shape.is_none()} onclick={ctx.link().callback(|_| Msg::UpdateShapeDraft)}>{"Update"}</button>
                        <button disabled={self.shape_catalog_pending || !self.can_edit_timeline || self.selected_shape_id.is_none()} onclick={ctx.link().callback(|_| Msg::DeleteShapeDraft)}>{"Delete"}</button>
                    </div>
                    <div class="shape-place-row">
                        <button class={classes!((self.selected_placement_kind == PlacementKind::Block).then_some("active"))} disabled={!can_use_template} onclick={ctx.link().callback(|_| Msg::SetPlacementContext(PlacementKind::Block))}><span class="swatch block"></span>{"Use Block template"}<kbd>{"OPEN"}</kbd></button>
                        <button class={classes!((self.selected_placement_kind == PlacementKind::Blind).then_some("active"))} disabled={!can_use_template} onclick={ctx.link().callback(|_| Msg::SetPlacementContext(PlacementKind::Blind))}><span class="swatch blind"></span>{"Use Pool template"}<kbd>{"OPEN"}</kbd></button>
                    </div>
                </aside>
            </div>
        }
    }

    fn view_image_studio(&self, ctx: &Context<Self>) -> Html {
        if let Some(converter) = self.view_image_converter(ctx) { return converter; }
        let selected_file = self.image_file.as_ref();
        let selected_file_label = selected_file
            .map(|file| format!("{} · {} bytes", file.name(), file.size() as u64))
            .unwrap_or_else(|| "No image selected".to_owned());
        let import_disabled = self.image_catalog_pending
            || !self.can_edit_timeline
            || selected_file.is_none()
            || self.image_draft_name.trim().is_empty();
        html! {
            <div class="image-studio-layout">
                <section class="image-studio-import">
                    <div class="studio-section-heading"><span>{"UPLOAD ORIGINAL"}</span><code>{"IMMUTABLE SOURCE"}</code></div>
                    <label class="image-file-picker">
                        <span>{"Choose image"}</span>
                        <input ref={self.image_file_input_ref.clone()} type="file" accept="image/png,image/jpeg,image/webp" disabled={self.image_catalog_pending || !self.can_edit_timeline} onchange={ctx.link().callback(|event: Event| {
                            let input = event.target_unchecked_into::<HtmlInputElement>();
                            Msg::ImageFileSelected(input.files().and_then(|files| files.get(0)))
                        })} />
                    </label>
                    <p class="image-file-status">{selected_file_label}</p>
                    <label class="image-template-name"><span>{"Template name"}</span><input value={self.image_draft_name.clone()} placeholder="Portal texture" disabled={self.image_catalog_pending || !self.can_edit_timeline} oninput={ctx.link().callback(|event: InputEvent| Msg::ImageDraftName(event.target_unchecked_into::<HtmlInputElement>().value()))} /></label>
                    { self.image_import_error.as_ref().map(|error| html! { <p class="image-import-error">{error}</p> }).unwrap_or_default() }
                    <button class="primary-action" disabled={import_disabled} onclick={ctx.link().callback(|_| Msg::ImportImage)}>{if self.image_catalog_pending { "Importing..." } else { "Upload and convert" }}</button>
                    <p class="image-studio-note">{"Static PNG, JPEG, and WebP · 512 KiB maximum · up to 1024px per side. Paste a copied image here or choose a file; the server verifies its bytes, dimensions, and digest."}</p>
                </section>
                <section class="image-studio-library">
                    <div class="studio-section-heading"><span>{"PROJECT IMAGE TEMPLATES"}</span><code>{self.image_catalog.len()}</code></div>
                    { if self.image_catalog_pending && self.image_catalog.is_empty() {
                        html! { <p class="image-library-empty">{"Loading shared templates..."}</p> }
                    } else if self.image_catalog.is_empty() {
                        html! { <p class="image-library-empty">{"No image templates are stored for this project."}</p> }
                    } else {
                        html! { <div class="image-catalog">{ for self.image_catalog.iter().map(|image| {
                            let source = format!("/api/projects/{}/images/{}/content", self.target.project_id, image.id);
                            let source_id = image.id.clone();
                            html! {
                                <article key={image.id.clone()} class="image-catalog-card">
                                    <img src={source} alt={image.name.clone()} loading="lazy" decoding="async" />
                                    <div><strong>{image.name.clone()}</strong><code>{format!("{} × {}", image.width, image.height)}</code><small>{format!("{} · {} bytes", image.media_type, image.byte_size)}</small><button disabled={!self.can_edit_timeline} onclick={ctx.link().callback(move |_| Msg::ImageStudio(ImageMsg::Convert(source_id.clone())))}>{"Convert palette"}</button></div>
                                </article>
                            }
                        }) }</div> }
                    } }
                    <p class="image-studio-note">{"Choosing or pasting uploads the original and opens the converter. Canceling conversion keeps the original. Prepared images never replace their source."}</p>
                    {self.view_prepared_images(ctx)}
                </section>
            </div>
        }
    }

    fn view_header(&self, ctx: &Context<Self>) -> Html {
        html! {
            <header class="topbar">
                <div class="product-mark" aria-label="Oreak">
                    <span class="mark-glyph">{"OR"}</span>
                    <span><strong>{"OREAK"}</strong><small>{"TECHNICAL EDITOR"}</small></span>
                </div>
                <nav class="menu-strip" aria-label="Application menu">
                    { self.view_menu_button(ctx, TopMenu::File, "File") }
                    { self.view_menu_button(ctx, TopMenu::History, "History") }
                    { self.view_menu_button(ctx, TopMenu::View, "View") }
                </nav>
                <div class="context-switcher" aria-label="Current editing context">
                    <button class="context-part" title="Open workspace explorer" onclick={ctx.link().callback(|_| Msg::SetLeftTab(LeftTab::Explorer))}>
                        <small>{"WORKSPACE"}</small><span>{self.workspace_name.clone()}</span>
                    </button>
                    <span class="context-slash">{"/"}</span>
                    <button class="context-part" title="Open project explorer" onclick={ctx.link().callback(|_| Msg::SetLeftTab(LeftTab::Explorer))}>
                        <small>{"PROJECT"}</small><span>{self.project_display_name.clone()}</span>
                    </button>
                    <span class="context-slash">{"/"}</span>
                    <button class="context-part context-level" title="Configure level" onclick={ctx.link().callback(|_| Msg::OpenLevelConfiguration)}>
                        <small>{"LEVEL"}</small><span>{self.level_display_name.clone()}</span>
                    </button>
                </div>
                <div class="header-tools">
                    <button class="header-nav" onclick={ctx.link().callback(|_| Msg::SetLeftTab(LeftTab::Explorer))}>{"Explorer"}</button>
                    <button
                        class="icon-button command-trigger"
                        onclick={ctx.link().callback(|_| Msg::TogglePalette)}
                        title="Open command palette (Ctrl+K)"
                    >
                        <span>{"Command"}</span><kbd>{"Ctrl K"}</kbd>
                    </button>
                    <button class={classes!("notification-trigger", self.notifications_open.then_some("active"))} aria-expanded={self.notifications_open.to_string()} title="Open notification history" onclick={ctx.link().callback(|_| Msg::ToggleNotifications)}><span>{"Notices"}</span><code>{self.toasts.len()}</code></button>
                    <div class="presence-stack" aria-label={format!("{} actors in {} connections", self.presence.actor_count(), self.presence.participant_count())}>
                        { for self.presence.participants().filter(|participant| !self.presence.is_self(&participant.id)).take(3).map(|participant| html! { <span class="presence" style={format!("background:{}", presence_color(participant.actor.as_str()))}>{actor_mark(participant.actor.as_str())}</span> }) }
                        <span class="presence-protocol">{self.presence.actor_count()}</span>
                        <small>{"PRESENCE"}</small>
                    </div>
                    <button
                        class="theme-toggle"
                        onclick={ctx.link().callback(|_| Msg::ToggleTheme)}
                        title={format!(
                            "Project default: {}. Current: {}{}",
                            self.theme.project_default.label(),
                            self.theme.active().label(),
                            if self.theme.user_override.is_some() { " (user override)" } else { "" }
                        )}
                    >
                        <span class="theme-track"><span class="theme-knob"></span></span>
                        <span>{self.theme.active().label()}</span>
                    </button>
                    <button class="header-nav" onclick={ctx.link().callback(|_| Msg::Logout)}>{"Logout"}</button>
                </div>
                { self.view_open_menu(ctx) }
                {
                    if self.open_menu.is_some() {
                        html! { <div class="menu-dismiss-layer" aria-hidden="true" onclick={ctx.link().callback(|_| Msg::CloseOverlays)}></div> }
                    } else {
                        Html::default()
                    }
                }
            </header>
        }
    }

    fn view_menu_button(&self, ctx: &Context<Self>, menu: TopMenu, label: &'static str) -> Html {
        let active = self.open_menu == Some(menu);
        html! {
            <button
                class={if active { "menu-button active" } else { "menu-button" }}
                aria-expanded={active.to_string()}
                onclick={ctx.link().callback(move |_| Msg::ToggleMenu(menu))}
            >{label}</button>
        }
    }

    fn view_open_menu(&self, ctx: &Context<Self>) -> Html {
        let Some(menu) = self.open_menu else {
            return Html::default();
        };
        let (class, body) = match menu {
            TopMenu::File => (
                "menu-file",
                html! {
                    <>
                        <button onclick={ctx.link().callback(|_| Msg::SaveDraft)}><span>{"Save local draft"}</span><kbd>{"auto"}</kbd></button>
                        <button onclick={ctx.link().callback(|_| Msg::ResetDraft)}><span>{"Reset draft"}</span><kbd>{"8 x 8"}</kbd></button>
                        <button onclick={ctx.link().callback(|_| Msg::Reconnect)}><span>{"Reconnect / resync"}</span><kbd>{"RPC"}</kbd></button>
                        <div class="menu-note">{format!("Typed collaboration target: {} / {}", self.target.project_id, self.target.level_id)}</div>
                    </>
                },
            ),
            TopMenu::History => (
                "menu-history",
                html! {
                    <>
                        <button onclick={ctx.link().callback(|_| Msg::Undo)}><span>{"Undo my latest"}</span><kbd>{"Ctrl Z"}</kbd></button>
                        <div class="menu-note">{format!("{} visible events in this session", self.activity_events().len())}</div>
                    </>
                },
            ),
            TopMenu::View => (
                "menu-view",
                html! {
                    <>
                        <button onclick={ctx.link().callback(|_| Msg::ToggleTheme)}><span>{"Toggle theme"}</span><kbd>{self.theme.active().label()}</kbd></button>
                        <button onclick={ctx.link().callback(|_| Msg::UseProjectTheme)}><span>{"Use project default"}</span><kbd>{self.theme.project_default.label()}</kbd></button>
                        <label class="menu-setting"><span>{"Font size"}</span><select value={self.ui_settings.font_size_offset.to_string()} onchange={ctx.link().callback(|event: Event| Msg::SetUiFontSize(event.target_unchecked_into::<HtmlSelectElement>().value()))}>{ for UiSettings::FONT_SIZE_OPTIONS.into_iter().map(|(value, label)| html! { <option value={value.to_string()}>{label}</option> }) }</select></label>
                        <label class="menu-setting"><span>{"UI scale"}</span><select value={self.ui_settings.scale_percent.to_string()} onchange={ctx.link().callback(|event: Event| Msg::SetUiScale(event.target_unchecked_into::<HtmlSelectElement>().value()))}>{ for UiSettings::SCALE_OPTIONS.into_iter().map(|(value, label)| html! { <option value={value.to_string()}>{label}</option> }) }</select></label>
                        <button onclick={ctx.link().callback(|_| Msg::ResetUiSettings)}><span>{"Reset UI sizing"}</span><kbd>{"100%"}</kbd></button>
                        <button onclick={ctx.link().callback(|_| Msg::TogglePanelLayout(SidebarSide::Left))}><span>{format!("{} left sidebar", self.left_panel_layout.toggle_label())}</span><kbd>{self.left_panel_layout.label()}</kbd></button>
                        <button onclick={ctx.link().callback(|_| Msg::SetPanelLayout(SidebarSide::Left, PanelLayout::Hidden))}><span>{"Hide left sidebar"}</span><kbd>{"LEFT"}</kbd></button>
                        <button onclick={ctx.link().callback(|_| Msg::TogglePanelLayout(SidebarSide::Right))}><span>{format!("{} inspector panel", self.right_panel_layout.toggle_label())}</span><kbd>{self.right_panel_layout.label()}</kbd></button>
                        <button onclick={ctx.link().callback(|_| Msg::SetPanelLayout(SidebarSide::Right, PanelLayout::Hidden))}><span>{"Hide inspector panel"}</span><kbd>{"RIGHT"}</kbd></button>
                    </>
                },
            ),
        };
        html! { <div class={classes!("top-menu-popover", class)}>{body}</div> }
    }

    fn view_primary_toolbox(&self, ctx: &Context<Self>) -> Html {
        let entity_mode = self.mode == Mode::Select;
        let mapping = self.mode == Mode::Map;
        let distribution = self.mode == Mode::Distribution;
        html! {
            <nav class="canvas-toolbox primary-canvas-tools" aria-label="Primary editor tools">
                <button class={classes!(entity_mode.then_some("active"))} aria-pressed={entity_mode.to_string()} onclick={ctx.link().callback(|_| Msg::SetMode(Mode::Select))}>{ui_icon(UiIcon::PlaceEntity, 15)}<span>{"Entity Placement"}</span></button>
                <button class={classes!(distribution.then_some("active"))} aria-pressed={distribution.to_string()} onclick={ctx.link().callback(|_| Msg::SetMode(Mode::Distribution))}>{ui_icon(UiIcon::Inspector, 15)}<span>{"Distribution"}</span></button>
                <button class={classes!(mapping.then_some("active"))} aria-pressed={mapping.to_string()} onclick={ctx.link().callback(|_| Msg::SetMode(Mode::Map))}>{ui_icon(UiIcon::MapDesign, 15)}<span>{"Map Design"}</span></button>
                <button title="Open Shape Studio" onclick={ctx.link().callback(|_| Msg::OpenStudio(StudioModal::Shape))}>{ui_icon(UiIcon::ShapeTemplates, 15)}<span>{"Shape Studio"}</span></button>
                <button title="Open Image Studio" onclick={ctx.link().callback(|_| Msg::OpenStudio(StudioModal::Image))}>{ui_icon(UiIcon::ImageStudio, 15)}<span>{"Image Studio"}</span></button>
            </nav>
        }
    }

    fn view_left_sidebar(&self, ctx: &Context<Self>) -> Html {
        let mut tabs = vec![LeftTab::ShapeTemplates];
        if self.image_studio.apply.is_some() { tabs.push(LeftTab::ApplyImage); }
        if self.mode == Mode::Brush {
            tabs.push(LeftTab::SandStudio);
        }
        tabs.extend([LeftTab::Explorer, LeftTab::Comments, LeftTab::Collaboration]);
        let toggle_layout_label = self.left_panel_layout.toggle_label();
        let toggle_layout_icon = if self.left_panel_layout == PanelLayout::Docked {
            UiIcon::Float
        } else {
            UiIcon::DockLeft
        };
        html! {
            <aside class={classes!("left-sidebar", "panel", (self.left_panel_layout == PanelLayout::Floating).then_some("panel-floating"))}>
                <div class="panel-tabs left-tabs compact-tab-bar">
                    { for tabs.into_iter().map(|tab| {
                        let active = self.left_tab == tab;
                        html! {
                            <button
                                class={classes!("sidebar-tab", active.then_some("active"))}
                                title={tab.label()}
                                aria-pressed={active.to_string()}
                                onclick={ctx.link().callback(move |_| Msg::SetLeftTab(tab))}
                            ><span class="sidebar-tab-icon">{ui_icon(tab.icon(), 14)}</span><span class="sidebar-tab-label">{tab.label()}</span></button>
                        }
                    }) }
                    <div class="panel-layout-actions"><button aria-label={format!("{} left sidebar", toggle_layout_label)} title={format!("{} left sidebar", toggle_layout_label)} onclick={ctx.link().callback(|_| Msg::TogglePanelLayout(SidebarSide::Left))}>{ui_icon(toggle_layout_icon, 12)}</button><button aria-label="Hide left sidebar" title="Hide left sidebar" onclick={ctx.link().callback(|_| Msg::SetPanelLayout(SidebarSide::Left, PanelLayout::Hidden))}>{ui_icon(UiIcon::HideLeft, 12)}</button></div>
                </div>
                <div class="left-panel-content">
                    {
                        match self.left_tab {
                            LeftTab::ShapeTemplates => self.view_shape_template_library(ctx),
                            LeftTab::SandStudio => self.view_sand_studio_panel(ctx),
                            LeftTab::ApplyImage => self.view_apply_image_panel(ctx),
                            LeftTab::Explorer => self.view_project_explorer(ctx),
                            LeftTab::Comments => self.view_comments_panel(),
                            LeftTab::Collaboration => self.view_collaboration_session(),
                        }
                    }
                </div>
            </aside>
        }
    }

    fn view_sand_studio_panel(&self, ctx: &Context<Self>) -> Html {
        let pool = self.selected_entity().and_then(|entity| {
            entity.as_blind().map(|blind| {
                (
                    entity.id().clone(),
                    blind.pixels_per_cell(),
                    entity.shape().occupied_count(),
                )
            })
        });
        let online = matches!(self.rpc_state, RpcState::Online);
        let busy = pool
            .as_ref()
            .is_some_and(|(entity_id, _, _)| self.pending_entities.contains_key(entity_id));
        let disabled = pool.is_none() || !self.can_edit_timeline || !online || busy;
        let status = if !online {
            "OFFLINE"
        } else if busy {
            "BUSY"
        } else {
            "BRUSH"
        };
        let painting = self.workspace_tool == WorkspaceTool::Paint;
        let filling = self.workspace_tool == WorkspaceTool::Fill;
        html! {
            <div class="shape-template-content sand-studio-panel">
                <div class="panel-heading">
                    <div><small>{"POOL PIXEL EDITOR"}</small><strong>{"Sand Studio"}</strong></div>
                    <span class="panel-code">{status}</span>
                </div>
                {
                    if let Some((entity_id, resolution, cells)) = pool {
                        html! {
                            <div class="sand-studio-target">
                                <small>{"ISOLATED POOL"}</small>
                                <strong title={entity_id.to_string()}>{truncate_entity_id(&entity_id)}</strong>
                                <span>{format!("{cells} cells · {resolution}×{resolution} pixels per cell")}</span>
                            </div>
                        }
                    } else {
                        html! { <div class="shape-template-empty"><strong>{"Select one Pool"}</strong><span>{"Sand Studio tools activate for a single selected Pool."}</span></div> }
                    }
                }
                <section class="sand-studio-section">
                    <div class="section-title"><span>{"BRUSH STYLE"}</span><code>{if painting { "PAINT" } else { "FILL" }}</code></div>
                    <div class="brush-tool-row">
                        <button class={classes!(painting.then_some("active"))} disabled={disabled} onclick={ctx.link().callback(|_| Msg::SetWorkspaceTool(WorkspaceTool::Paint))}><span>{"Paint"}</span><kbd>{"B"}</kbd></button>
                        <button class={classes!(filling.then_some("active"))} disabled={disabled} onclick={ctx.link().callback(|_| Msg::SetWorkspaceTool(WorkspaceTool::Fill))}><span>{"Fill"}</span><kbd>{"F"}</kbd></button>
                    </div>
                    <p class="shape-template-note">{if painting { "Drag inside one Pool cell to paint. Hold Shift while dragging to erase." } else { "Click any sand region to replace its current color." }}</p>
                </section>
                <section class="sand-studio-section">
                    <div class="section-title"><span>{"SAND COLOR"}</span><code>{format!("{:02}", self.blind_color_index)}</code></div>
                    <div class="brush-color-grid">
                        { for (1_u8..=10).map(|color_index| {
                            let active = self.blind_color_index == color_index;
                            html! {
                                <button class={classes!(active.then_some("active"))} style={format!("--brush-color: {};", blind_color(color_index))} disabled={disabled} aria-label={format!("Sand color {color_index}")} aria-pressed={active.to_string()} onclick={ctx.link().callback(move |_| Msg::SetBlindColor(color_index))}>
                                    <span></span><kbd>{if color_index == 10 { "0".to_owned() } else { color_index.to_string() }}</kbd>
                                </button>
                            }
                        }) }
                    </div>
                </section>
                <button class="blind-isolation active" onclick={ctx.link().callback(|_| Msg::SetMode(Mode::Select))}>{"Exit Sand Studio"}</button>
            </div>
        }
    }

    fn view_project_explorer(&self, ctx: &Context<Self>) -> Html {
        let snapshot = self.model.timeline().snapshot();
        let current_level_name = if self.level_display_name.is_empty() {
            "No level selected".to_owned()
        } else {
            self.level_display_name.clone()
        };
        let creating_workspace =
            matches!(self.explorer_action, Some(ExplorerAction::CreateWorkspace));
        html! {
            <div class="explorer-content">
                <div class="panel-heading"><div><small>{"CURRENT LEVEL"}</small><strong>{current_level_name}</strong></div><span class="panel-code">{format!("{} ENT", snapshot.entities().len())}</span></div>
                <section class="explorer-section explorer-projects-section">
                    <div class="section-title explorer-section-title">
                        <span>{"WORKSPACES"}</span>
                        <button class="explorer-title-action" title="New workspace" aria-label="New workspace" onclick={ctx.link().callback(|_| Msg::BeginExplorerAction(ExplorerAction::CreateWorkspace))}><Plus size={12} /></button>
                    </div>
                    <div class="explorer-tree vscode-tree" role="tree" aria-label="Workspace hierarchy">
                        { if creating_workspace { html! {
                            <form class="explorer-inline-form depth-0" onsubmit={ctx.link().callback(|event: SubmitEvent| { event.prevent_default(); Msg::CreateWorkspace })}>
                                <input autofocus=true aria-label="Workspace name" value={self.workspace_draft_name.clone()} placeholder="Workspace name" disabled={self.request_pending} oninput={ctx.link().callback(|event: InputEvent| Msg::WorkspaceName(event.target_unchecked_into::<HtmlInputElement>().value()))} />
                                <button type="submit" disabled={self.request_pending || self.workspace_draft_name.trim().is_empty()}>{"Create"}</button>
                                <button type="button" disabled={self.request_pending} onclick={ctx.link().callback(|_| Msg::CancelExplorerAction)}>{"Cancel"}</button>
                            </form>
                        } } else { Html::default() } }
                        { for self.workspaces.iter().map(|workspace| {
                            let workspace_id = workspace.id.clone();
                            let workspace_id_for_toggle = workspace.id.clone();
                            let workspace_id_for_add = workspace.id.clone();
                            let selected_workspace = workspace.id == self.selected_workspace_id;
                            let expanded = self.expanded_workspaces.contains(&workspace.id);
                            let can_manage = matches!(workspace.role.as_str(), "owner" | "admin");
                            let creating_project = matches!(&self.explorer_action, Some(ExplorerAction::CreateProject { workspace_id }) if workspace_id == &workspace.id);
                            let projects = self.projects.iter().filter(|project| project.workspace_id == workspace.id).collect::<Vec<_>>();
                            html! {
                                <div class={classes!("explorer-branch", "workspace-branch-row", selected_workspace.then_some("selected-context"))} key={workspace.id.clone()} role="treeitem" aria-level="1" aria-expanded={expanded.to_string()} aria-selected={selected_workspace.to_string()}>
                                    <div class="explorer-row depth-0">
                                        <button class="explorer-chevron" title={if expanded { "Collapse workspace" } else { "Expand workspace" }} aria-label={if expanded { "Collapse workspace" } else { "Expand workspace" }} aria-expanded={expanded.to_string()} onclick={ctx.link().callback(move |_| Msg::ToggleWorkspace(workspace_id_for_toggle.clone()))}>{ if expanded { html! { <ChevronDown size={12} /> } } else { html! { <ChevronRight size={12} /> } } }</button>
                                        <button class="explorer-row-label" onclick={ctx.link().callback(move |_| Msg::SelectWorkspace(workspace_id.clone()))}><span class="tree-icon">{"W"}</span><span>{workspace.name.clone()}</span></button>
                                        <div class="explorer-row-actions">{ if can_manage { html! {
                                            <button title="New project" aria-label={format!("New project in {}", workspace.name)} onclick={ctx.link().callback(move |_| Msg::BeginExplorerAction(ExplorerAction::CreateProject { workspace_id: workspace_id_for_add.clone() }))}><Plus size={12} /></button>
                                        } } else { Html::default() } }</div>
                                    </div>
                                    { if expanded { html! {
                                        <div class="explorer-branch-children" role="group">
                                            { if creating_project { html! {
                                                <form class="explorer-inline-form depth-1" onsubmit={ctx.link().callback(|event: SubmitEvent| { event.prevent_default(); Msg::CreateProject })}>
                                                    <input autofocus=true aria-label="Project name" value={self.project_name.clone()} placeholder="Project name" disabled={self.request_pending} oninput={ctx.link().callback(|event: InputEvent| Msg::ProjectName(event.target_unchecked_into::<HtmlInputElement>().value()))} />
                                                    <button type="submit" disabled={self.request_pending || self.project_name.trim().is_empty()}>{"Create"}</button>
                                                    <button type="button" disabled={self.request_pending} onclick={ctx.link().callback(|_| Msg::CancelExplorerAction)}>{"Cancel"}</button>
                                                </form>
                                            } } else { Html::default() } }
                                            { for projects.into_iter().map(|project| {
                                                let project_id_for_toggle = project.id.clone();
                                                let project_id_for_select = project.id.clone();
                                                let project_id_for_add = project.id.clone();
                                                let project_id_for_delete = project.id.clone();
                                                let project_for_select = project.clone();
                                                let project_for_configuration = project.clone();
                                                let levels = self.catalog_levels.get(&project.id).cloned().unwrap_or_default();
                                                let current_project = self.target.project_id.as_str() == project.id;
                                                let selected_project = self.catalog_project.as_ref().is_some_and(|selected| selected.id == project.id);
                                                let project_expanded = self.expanded_projects.contains(&project.id);
                                                let creating_level = matches!(&self.explorer_action, Some(ExplorerAction::CreateLevel { project_id }) if project_id == &project.id);
                                                let deleting_project = matches!(&self.explorer_action, Some(ExplorerAction::DeleteProject { project_id }) if project_id == &project.id);
                                                let can_delete_project = can_manage;
                                                html! {
                                                    <div class={classes!("explorer-branch", "project-branch-row", current_project.then_some("active"), selected_project.then_some("selected-context"))} key={project.id.clone()} role="treeitem" aria-level="2" aria-expanded={project_expanded.to_string()} aria-selected={selected_project.to_string()}>
                                                        <div class="explorer-row depth-1">
                                                            <button class="explorer-chevron" title={if project_expanded { "Collapse project" } else { "Expand project" }} aria-label={if project_expanded { "Collapse project" } else { "Expand project" }} aria-expanded={project_expanded.to_string()} onclick={ctx.link().callback(move |_| Msg::ToggleProject(project_id_for_toggle.clone()))}>{ if project_expanded { html! { <ChevronDown size={12} /> } } else { html! { <ChevronRight size={12} /> } } }</button>
                                                            <button class="explorer-row-label" onclick={ctx.link().callback(move |_| Msg::SelectProject(project_for_select.clone()))}><span class="tree-icon">{"P"}</span><span>{project.name.clone()}</span></button>
                                                            <div class="explorer-row-actions">
                                                                { if project.can_edit_timeline() { html! { <button title="New level" aria-label={format!("New level in {}", project.name)} onclick={ctx.link().callback(move |_| Msg::BeginExplorerAction(ExplorerAction::CreateLevel { project_id: project_id_for_add.clone() }))}><Plus size={12} /></button> } } else { Html::default() } }
                                                                { if project.can_manage_members() { html! { <button title="Project settings" aria-label={format!("Project settings for {}", project.name)} onclick={ctx.link().callback(move |_| Msg::OpenProjectConfiguration(project_for_configuration.clone()))}><Settings size={11} /></button> } } else { Html::default() } }
                                                                { if can_delete_project { html! { <button class="danger" title="Delete project" aria-label={format!("Delete project {}", project.name)} onclick={ctx.link().callback(move |_| Msg::BeginExplorerAction(ExplorerAction::DeleteProject { project_id: project_id_for_delete.clone() }))}><Trash size={11} /></button> } } else { Html::default() } }
                                                            </div>
                                                        </div>
                                                        { if deleting_project { html! {
                                                            <div class="explorer-inline-confirm depth-2"><span>{format!("Delete {} and all levels?", project.name)}</span><button class="danger" disabled={self.request_pending} onclick={{ let project_id = project.id.clone(); ctx.link().callback(move |_| Msg::DeleteProject(project_id.clone())) }}>{"Delete"}</button><button disabled={self.request_pending} onclick={ctx.link().callback(|_| Msg::CancelExplorerAction)}>{"Cancel"}</button></div>
                                                        } } else { Html::default() } }
                                                        { if project_expanded { html! {
                                                            <div class="explorer-branch-children" role="group">
                                                                { if creating_level { html! {
                                                                    <form class="explorer-inline-form depth-2 level-form" onsubmit={ctx.link().callback(|event: SubmitEvent| { event.prevent_default(); Msg::CreateLevel })}>
                                                                        <input autofocus=true aria-label="Level name" value={self.level_name.clone()} placeholder="Level name" disabled={self.request_pending} oninput={ctx.link().callback(|event: InputEvent| Msg::LevelName(event.target_unchecked_into::<HtmlInputElement>().value()))} />
                                                                        <input class="duration" type="number" min="0" step="0.1" title="Duration in seconds" aria-label="Level duration in seconds" value={self.level_duration.clone()} disabled={self.request_pending} oninput={ctx.link().callback(|event: InputEvent| Msg::LevelDuration(event.target_unchecked_into::<HtmlInputElement>().value()))} />
                                                                        <button type="submit" disabled={self.request_pending || self.level_name.trim().is_empty()}>{"Create"}</button>
                                                                        <button type="button" disabled={self.request_pending} onclick={ctx.link().callback(|_| Msg::CancelExplorerAction)}>{"Cancel"}</button>
                                                                    </form>
                                                                } } else { Html::default() } }
                                                                { for levels.into_iter().map(|level| {
                                                                    let project_for_open = project.clone();
                                                                    let level_for_open = level.clone();
                                                                    let project_id_for_level_delete = project_id_for_select.clone();
                                                                    let level_id_for_delete = level.id.clone();
                                                                    let current_level = current_project && self.target.level_id.as_str() == level.id;
                                                                    let deleting_level = matches!(&self.explorer_action, Some(ExplorerAction::DeleteLevel { project_id, level_id }) if project_id == &project.id && level_id == &level.id);
                                                                    html! {
                                                                        <div class="explorer-level-wrap" key={level.id.clone()} role="treeitem" aria-level="3" aria-selected={current_level.to_string()}>
                                                                            <div class={classes!("explorer-row", "depth-2", current_level.then_some("active"))}>
                                                                                <span class="explorer-spacer"></span>
                                                                                <button class="explorer-row-label" onclick={ctx.link().callback(move |_| Msg::OpenLevel(project_for_open.clone(), level_for_open.clone()))}><span class="tree-icon">{"L"}</span><span>{level.name.clone()}</span></button>
                                                                                <div class="explorer-row-actions">{ if project.can_edit_timeline() { html! { <button class="danger" title="Delete level" aria-label={format!("Delete level {}", level.name)} onclick={ctx.link().callback(move |_| Msg::BeginExplorerAction(ExplorerAction::DeleteLevel { project_id: project_id_for_level_delete.clone(), level_id: level_id_for_delete.clone() }))}><Trash size={11} /></button> } } else { Html::default() } }</div>
                                                                            </div>
                                                                            { if deleting_level { html! {
                                                                                <div class="explorer-inline-confirm depth-3"><span>{format!("Delete {}?", level.name)}</span><button class="danger" disabled={self.request_pending} onclick={{ let project_id = project.id.clone(); let level_id = level.id.clone(); ctx.link().callback(move |_| Msg::DeleteLevel { project_id: project_id.clone(), level_id: level_id.clone() }) }}>{"Delete"}</button><button disabled={self.request_pending} onclick={ctx.link().callback(|_| Msg::CancelExplorerAction)}>{"Cancel"}</button></div>
                                                                            } } else { Html::default() } }
                                                                        </div>
                                                                    }
                                                                }) }
                                                            </div>
                                                        } } else { Html::default() } }
                                                    </div>
                                                }
                                            }) }
                                        </div>
                                    } } else { Html::default() } }
                                </div>
                            }
                        }) }
                        { if self.workspaces.is_empty() && !creating_workspace { html! { <div class="empty-selection"><span>{"NO WORKSPACES"}</span><p>{"Use + to create your first workspace."}</p></div> } } else { Html::default() } }
                        { self.form_error.as_ref().map(|error| html! { <div class="explorer-catalog-error"><strong>{"REQUEST FAILED"}</strong><p>{error}</p><button disabled={self.request_pending} onclick={ctx.link().callback(|_| Msg::ReloadCatalog)}>{"Retry catalog"}</button></div> }).unwrap_or_default() }
                    </div>
                </section>
                { if !self.catalog_invitations.is_empty() { html! {
                    <section class="explorer-section explorer-invitations"><div class="section-title"><span>{"INVITATIONS"}</span><code>{self.catalog_invitations.len()}</code></div>{ for self.catalog_invitations.iter().map(|invitation| { let invitation_message = invitation.clone(); html! { <div class="explorer-invitation"><span>{invitation.project_name.clone()}</span><button disabled={self.request_pending} onclick={ctx.link().callback(move |_| Msg::AcceptProjectInvitation(invitation_message.clone()))}>{"Accept"}</button></div> } }) }</section>
                } } else { Html::default() } }
                <section class="explorer-section">
                    <div class="section-title"><span>{"ENTITIES"}</span><code>{snapshot.entities().len()}</code></div>
                    <div class="explorer-tree">
                        { for snapshot.entities().iter().map(|entity| {
                            let entity_id = entity.id().clone();
                            let selected = matches!(&self.selection, Some(Selection::Entities(ids)) if ids.contains(entity.id()));
                            let kind = match entity.kind() { PlaceableEntityKind::Block(_) => "Sand Block", PlaceableEntityKind::Blind(_) => "Sand Pool" };
                            html! { <button class={classes!("explorer-entity", selected.then_some("active"))} onclick={ctx.link().callback(move |_| Msg::FocusExplorerEntity(entity_id.clone()))}><span class="tree-icon">{if matches!(entity.kind(), PlaceableEntityKind::Block(_)) { "B" } else { "P" }}</span><span><strong>{kind}</strong><small>{format!("{} / {:02}:{:02}", entity.id(), entity.origin().x, entity.origin().y)}</small></span></button> }
                        }) }
                        { if snapshot.entities().is_empty() { html! { <div class="empty-selection"><span>{"NO ENTITIES"}</span><p>{"Open or create a level to begin editing."}</p></div> } } else { Html::default() } }
                    </div>
                </section>
            </div>
        }
    }
    fn view_comments_panel(&self) -> Html {
        html! {
            <div class="empty-selection panel-placeholder">
                <span>{"COMMENTS"}</span>
                <p>{"Comment threads need their own server-backed model. This tab is reserved so the editor shell can host them without overloading audit history."}</p>
            </div>
        }
    }

    fn view_collaboration_session(&self) -> Html {
        html! {
            <>
                { self.view_activity_feed() }
                <section class="collaborators">
                    <div class="section-title"><span>{"PRESENCE"}</span><code>{format!("{} ACTORS / {} TABS", self.presence.actor_count(), self.presence.participant_count())}</code></div>
                    <div class="collaborator-list">
                        { for self.presence.participants().map(|participant| html! {
                            <article class="collaborator" key={participant.id.as_str().to_owned()}>
                                <span class="presence" style={format!("background:{}", presence_color(participant.actor.as_str()))}>{actor_mark(participant.actor.as_str())}</span>
                                <div><strong>{participant.actor.to_string()}</strong><small>{if self.presence.is_self(&participant.id) { "This tab" } else if participant.cursor.is_some() { "Editing canvas" } else { "Viewing level" }}</small></div>
                                <code>{participant.cursor.map(|point| format!("{:02}:{:02}", point.x, point.y)).unwrap_or_else(|| "--:--".to_owned())}</code>
                            </article>
                        }) }
                        { if self.presence.participant_count() == 0 { html! { <div class="presence-unavailable"><strong>{"Connecting roster"}</strong><small>{"Presence appears after the collaboration stream is ready."}</small></div> } } else { Html::default() } }
                    </div>
                </section>
            </>
        }
    }

    fn view_selection_inspector(&self, ctx: &Context<Self>) -> Html {
        let selected_cell = self.selected_cell();
        let selected_kind = selected_cell.and_then(|point| self.effective_cell(point));
        let selected_entity = self.selected_entity();
        let selected_entity_count = self.selected_entity_ids().len();
        let size = self.model.timeline().snapshot().size();
        let current_target = selection_status(self.selection.as_ref());
        let compact_target = inspector_target_status(self.selection.as_ref());
        let selection_collapsed = self.inspector_section_is_collapsed(InspectorSection::Selection);
        let entity_data_collapsed =
            self.inspector_section_is_collapsed(InspectorSection::EntityData);
        let capacity_collapsed = self.inspector_section_is_collapsed(InspectorSection::Capacity);
        html! {
            <div class="activity-content inspector-content">
                <div class="panel-heading compact-heading">
                    <div><small>{"CURRENT TARGET"}</small><strong class="current-target-label" title={current_target}>{compact_target}</strong></div>
                    <span class="panel-code">{"I-01"}</span>
                </div>
                <section class="inspector-section">
                    { self.view_inspector_section_header(ctx, InspectorSection::Selection, "Selection") }
                    {
                        if !selection_collapsed {
                            html! {
                                <div id={InspectorSection::Selection.content_id()} class="inspector-section-body">
                                    {
                                        if selected_entity_count > 1 {
                                            let selected_ids = self.selected_entity_ids();
                                            let snapshot = self.model.timeline().snapshot();
                                            let selected_entities = selected_ids.iter().filter_map(|entity_id| snapshot.entity(entity_id)).collect::<Vec<_>>();
                                            let all_blocks = !selected_entities.is_empty() && selected_entities.iter().all(|entity| matches!(entity.kind(), PlaceableEntityKind::Block(_)));
                                            let all_pools = !selected_entities.is_empty() && selected_entities.iter().all(|entity| matches!(entity.kind(), PlaceableEntityKind::Blind(_)));
                                            if all_blocks || all_pools {
                                                let group_kind = if all_blocks { "Sand Blocks" } else { "Sand Pools" };
                                                let common_origin = selected_entities.first().and_then(|first| {
                                                    selected_entities.iter().all(|entity| entity.origin() == first.origin()).then_some(first.origin())
                                                });
                                                let common_shape = selected_entities.first().and_then(|first| {
                                                    selected_entities.iter().all(|entity| entity.shape() == first.shape()).then_some(first.shape())
                                                });
                                                html! {
                                                    <>
                                                        <div class="property-table multi-selection-properties">
                                                            <div><span>{"Target"}</span><strong>{format!("{} selected", group_kind)}</strong></div>
                                                            <div><span>{"Count"}</span><strong>{selected_entity_count}</strong></div>
                                                            <div><span>{"Origin"}</span><code>{common_origin.map_or_else(|| "<different>".to_owned(), |origin| format!("x{:02} / y{:02}", origin.x, origin.y))}</code></div>
                                                            <div><span>{"Shape"}</span><code>{common_shape.map_or_else(|| "<different>".to_owned(), |shape| format!("{} x {} / {} cells", shape.width(), shape.height(), shape.occupied_count()))}</code></div>
                                                            <div><span>{"Bulk edit"}</span><strong>{if all_blocks { "Capacity layers" } else { "Transform tool" }}</strong></div>
                                                        </div>
                                                        <div class="entity-command-row inspector-selection-actions">
                                                            <button disabled={!self.can_edit_timeline || !matches!(self.rpc_state, RpcState::Online) || selected_ids.iter().any(|entity_id| self.pending_entities.contains_key(entity_id))} onclick={ctx.link().callback(|_| Msg::EditEntity(EntityAction::RotateClockwise))}>{"Rotate CW"}</button>
                                                            <button disabled={!self.can_edit_timeline || !matches!(self.rpc_state, RpcState::Online) || selected_ids.iter().any(|entity_id| self.pending_entities.contains_key(entity_id))} onclick={ctx.link().callback(|_| Msg::EditEntity(EntityAction::FlipHorizontal))}>{"Flip H"}</button>
                                                            <button class="entity-delete" disabled={!self.can_edit_timeline || !matches!(self.rpc_state, RpcState::Online) || selected_ids.iter().any(|entity_id| self.pending_entities.contains_key(entity_id))} onclick={ctx.link().callback(|_| Msg::EditEntity(EntityAction::Delete))}>{"Delete"}</button>
                                                        </div>
                                                    </>
                                                }
                                            } else {
                                                html! { <div class="empty-selection inspector-mixed-selection"><span>{"Multiple Type of Entity Selected"}</span><p>{"Select only Sand Blocks or only Sand Pools to inspect shared properties."}</p></div> }
                                            }
                                        } else if let Some(entity) = selected_entity {
                                            let shape = entity.shape();
                                            let entity_action_disabled = !self.can_edit_timeline
                                                || !matches!(self.rpc_state, RpcState::Online)
                                                || self.pending_entities.contains_key(entity.id());
                                            html! {
                                                <>
                                                <div class="property-table">
                                                    <div><span>{"Target"}</span><strong>{"Placeable entity"}</strong></div>
                                                    <div><span>{"ID"}</span><code title={entity.id().to_string()}>{truncate_entity_id(entity.id())}</code></div>
                                                    <div><span>{"Kind"}</span><strong>{entity_kind_label(entity.kind())}</strong></div>
                                                    <div><span>{"Origin"}</span><code>{format!("x{:02} / y{:02}", entity.origin().x, entity.origin().y)}</code></div>
                                                    <div><span>{"Shape"}</span><code>{format!("{} x {} / {} cells", shape.width(), shape.height(), shape.occupied_count())}</code></div>
                                                </div>
                                                <div class="entity-command-row inspector-selection-actions">
                                                    <button disabled={entity_action_disabled} onclick={ctx.link().callback(|_| Msg::EditEntity(EntityAction::RotateClockwise))}>{"Rotate CW"}</button>
                                                    <button disabled={entity_action_disabled} onclick={ctx.link().callback(|_| Msg::EditEntity(EntityAction::FlipHorizontal))}>{"Flip H"}</button>
                                                    <button class="entity-delete" disabled={entity_action_disabled} onclick={ctx.link().callback(|_| Msg::EditEntity(EntityAction::Delete))}>{"Delete"}</button>
                                                </div>
                                                </>
                                            }
                                        } else if let Some(point) = selected_cell {
                                            html! {
                                                <div class="property-table">
                                                    <div><span>{"Target"}</span><strong>{"Logical cell"}</strong></div>
                                                    <div><span>{"Coordinate"}</span><code>{format!("x{:02} / y{:02}", point.x, point.y)}</code></div>
                                                    <div><span>{"Kind"}</span><strong>{cell_kind_label(selected_kind.unwrap_or(CellKind::Floor))}</strong></div>
                                                    <div><span>{"Index"}</span><code>{format!("{:03}", point.x + point.y * size.width())}</code></div>
                                                </div>
                                            }
                                        } else {
                                            html! { <div class="empty-selection"><span>{"NO TARGET"}</span><p>{"Select a cell or entity footprint on the canvas."}</p></div> }
                                        }
                                    }
                                </div>
                            }
                        } else {
                            Html::default()
                        }
                    }
                </section>
                {
                    selected_entity.map(|entity| view_entity_kind_details(
                        entity,
                        entity_data_collapsed,
                        self.view_inspector_section_header(ctx, InspectorSection::EntityData, "Entity data"),
                    )).or_else(|| self.view_multi_entity_kind_details(ctx, entity_data_collapsed)).unwrap_or_default()
                }
                { self.view_block_capacity(ctx, capacity_collapsed) }
                { selected_entity.map(|entity| match entity.kind() {
                    PlaceableEntityKind::Block(_) => self.view_block_decorator_controls(ctx, entity, !matches!(self.rpc_state, RpcState::Online)),
                    PlaceableEntityKind::Blind(_) => self.view_blind_decorator_controls(ctx, entity, !matches!(self.rpc_state, RpcState::Online)),
                }).unwrap_or_default() }
                { self.view_pool_resolution(ctx) }
            </div>
        }
    }

    fn view_multi_entity_kind_details(&self, ctx: &Context<Self>, collapsed: bool) -> Option<Html> {
        let entities = self.selected_entities();
        let first = *entities.first()?;
        if entities.len() < 2
            || !entities.iter().all(|entity| {
                std::mem::discriminant(entity.kind()) == std::mem::discriminant(first.kind())
            })
        {
            return None;
        }
        let shapes_match = entities
            .iter()
            .all(|entity| entity.shape() == first.shape());
        let origin_match = entities
            .iter()
            .all(|entity| entity.origin() == first.origin());
        let shape = shapes_match.then_some(first.shape()).map_or_else(
            || "<different>".to_owned(),
            |shape| {
                format!(
                    "{} x {} / {} cells",
                    shape.width(),
                    shape.height(),
                    shape.occupied_count()
                )
            },
        );
        let origin = origin_match.then_some(first.origin()).map_or_else(
            || "<different>".to_owned(),
            |origin| format!("x{:02} / y{:02}", origin.x, origin.y),
        );
        let kind = entity_kind_label(first.kind());
        Some(html! {
            <section class="inspector-section">
                { self.view_inspector_section_header(ctx, InspectorSection::EntityData, "Entity data") }
                {
                    if !collapsed {
                        html! {
                            <div id={InspectorSection::EntityData.content_id()} class="inspector-section-body">
                                <div class="property-table">
                                    <div><span>{"Kind"}</span><strong>{kind}</strong></div>
                                    <div><span>{"Selected"}</span><strong>{entities.len()}</strong></div>
                                    <div><span>{"Origin"}</span><code>{origin}</code></div>
                                    <div><span>{"Shape"}</span><code>{shape}</code></div>
                                </div>
                            </div>
                        }
                    } else {
                        Html::default()
                    }
                }
            </section>
        })
    }

    fn view_block_capacity(&self, ctx: &Context<Self>, collapsed: bool) -> Html {
        let entities = self.selected_entities();
        if entities.is_empty()
            || !entities
                .iter()
                .all(|entity| matches!(entity.kind(), PlaceableEntityKind::Block(_)))
        {
            return Html::default();
        }
        let blocks = entities
            .iter()
            .filter_map(|entity| match entity.kind() {
                PlaceableEntityKind::Block(block) => Some(block),
                PlaceableEntityKind::Blind(_) => None,
            })
            .collect::<Vec<_>>();
        let max_layer_count = blocks
            .iter()
            .map(|block| block.collect_layers().len())
            .max()
            .unwrap_or(0);
        let single_selection = entities.len() == 1;
        let disabled = !self.can_edit_timeline
            || !matches!(self.rpc_state, RpcState::Online)
            || self
                .selected_entity_ids()
                .iter()
                .any(|entity_id| self.pending_entities.contains_key(entity_id));
        html! {
            <section class="inspector-section capacity-section">
                { self.view_inspector_section_header(ctx, InspectorSection::Capacity, "Capacity") }
                {
                    if !collapsed {
                        html! {
                            <div id={InspectorSection::Capacity.content_id()} class="inspector-section-body capacity-section-body">
                                {
                                    if !single_selection && blocks.iter().any(|block| block.collect_layers().len() != max_layer_count) {
                                        html! { <p class="inspector-note">{"<different> layer counts. A capacity can be batch-edited only where every selected Block has that layer."}</p> }
                                    } else if !single_selection {
                                        html! { <p class="inspector-note">{"<different> means selected Blocks disagree. Enter a value to apply it to every selected Block."}</p> }
                                    } else {
                                        Html::default()
                                    }
                                }
                                <div class="capacity-layer-list">
                                    { for (0..max_layer_count).map(|layer_index| {
                                        let layers = blocks.iter().map(|block| block.collect_layers().get(layer_index)).collect::<Vec<_>>();
                                        let all_have_layer = layers.iter().all(Option::is_some);
                                        let first_layer = layers.first().and_then(|layer| *layer);
                                        let capacity = first_layer.and_then(|layer| {
                                            all_have_layer.then_some(layer.capacity())
                                        }).filter(|capacity| layers.iter().all(|layer| layer.is_some_and(|item| item.capacity() == *capacity)));
                                        let color = first_layer.map(|layer| layer.color_index()).filter(|color| layers.iter().all(|layer| layer.is_some_and(|item| item.color_index() == *color)));
                                        let radius = first_layer.map(|layer| layer.radius()).filter(|radius| layers.iter().all(|layer| layer.is_some_and(|item| item.radius() == *radius)));
                                        let locked = first_layer.map(|layer| layer.is_locked()).filter(|locked| layers.iter().all(|layer| layer.is_some_and(|item| item.is_locked() == *locked)));
                                        let any_locked = layers.iter().any(|layer| layer.is_some_and(CollectLayer::is_locked));
                                        let capacity_editable = all_have_layer && !any_locked;
                                        let capacity_value = capacity.map_or_else(|| "<different>".to_owned(), collect_capacity_label);
                                        let color_value = color.map_or_else(|| "<different>".to_owned(), |color| format!("Color {color}"));
                                        let radius_value = radius.map_or_else(|| "<different>".to_owned(), |radius| radius.map_or_else(|| "Default radius".to_owned(), |radius| format!("Radius {radius}")));
                                        let locked_value = locked.map_or_else(|| "<different>".to_owned(), |locked| if locked { "Locked".to_owned() } else { "Unlocked".to_owned() });
                                        html! {
                                            <div class="capacity-layer" key={layer_index}>
                                                <div class="capacity-layer-heading"><strong>{format!("Layer {}", layer_index + 1)}</strong>{if single_selection { html! { <button class="layer-remove" disabled={disabled || any_locked} title={if any_locked { "Locked capacity layers cannot be removed" } else { "Remove capacity layer" }} onclick={ctx.link().callback(move |_| Msg::RemoveBlockCollectLayer(layer_index))}>{"Remove"}</button> } } else { Html::default() }}</div>
                                                <div class="capacity-layer-meta"><span>{color_value}</span><span>{radius_value}</span><span>{locked_value}</span></div>
                                                <label><span>{"Capacity"}</span><input type="text" value={capacity_value} disabled={disabled || !capacity_editable} title={if !all_have_layer { "Every selected Block must have this layer" } else if any_locked { "Locked capacity layers cannot be edited" } else { "Enter a number or unlimited" }} onchange={ctx.link().callback(move |event: Event| Msg::SetBlockLayerCapacity(layer_index, event.target_unchecked_into::<HtmlInputElement>().value()))} /></label>
                                            </div>
                                        }
                                    }) }
                                </div>
                                {
                                    if single_selection {
                                        html! { <button class="capacity-add-layer" disabled={disabled} onclick={ctx.link().callback(|_| Msg::AddBlockCollectLayer)}>{"Add capacity layer"}</button> }
                                    } else {
                                        Html::default()
                                    }
                                }
                            </div>
                        }
                    } else {
                        Html::default()
                    }
                }
            </section>
        }
    }

    fn view_level_structure(&self, ctx: &Context<Self>) -> Html {
        let snapshot = self.model.timeline().snapshot();
        let size = snapshot.size();
        html! {
            <div class="activity-content level-structure-content">
                <div class="panel-heading compact-heading">
                    <div><small>{"LEVEL"}</small><strong>{self.level_display_name.clone()}</strong></div>
                    <span class="panel-code">{"L-01"}</span>
                </div>
                <section class="inspector-section scene-tree">
                    <h2>{"Level structure"}</h2>
                    <button class="tree-row expanded"><span>{"v"}</span><strong>{self.level_display_name.clone()}</strong><code>{"ROOT"}</code></button>
                    <button class="tree-row child active"><span>{"#"}</span><strong>{"Logical map"}</strong><code>{format!("{}x{}", size.width(), size.height())}</code></button>
                    <button class="tree-row child"><span>{"E"}</span><strong>{"Core placeables"}</strong><code>{snapshot.entities().len()}</code></button>
                </section>
                <section class="inspector-section resize-controls">
                    <h2>{"Map dimensions"}</h2>
                    <div class="resize-dimensions">
                        <label><span>{"Width"}</span><input type="number" min="1" max="256" value={self.resize_width.clone()} oninput={ctx.link().callback(|event: InputEvent| Msg::ResizeWidth(event.target_unchecked_into::<HtmlInputElement>().value()))} /></label>
                        <label><span>{"Height"}</span><input type="number" min="1" max="256" value={self.resize_height.clone()} oninput={ctx.link().callback(|event: InputEvent| Msg::ResizeHeight(event.target_unchecked_into::<HtmlInputElement>().value()))} /></label>
                    </div>
                    <div class="resize-anchor-grid" aria-label="Resize anchor">
                        { for [
                            GridAnchor::TopLeft, GridAnchor::Top, GridAnchor::TopRight,
                            GridAnchor::Left, GridAnchor::Center, GridAnchor::Right,
                            GridAnchor::BottomLeft, GridAnchor::Bottom, GridAnchor::BottomRight,
                        ].into_iter().map(|anchor| html! {
                            <button class={classes!((self.resize_anchor == anchor).then_some("active"))} title={grid_anchor_label(anchor)} onclick={ctx.link().callback(move |_| Msg::SetResizeAnchor(anchor))}></button>
                        }) }
                    </div>
                    <button class="resize-apply" disabled={!self.can_edit_timeline || !matches!(self.rpc_state, RpcState::Online) || self.pending_grid.is_some()} onclick={ctx.link().callback(|_| Msg::ResizeGrid)}>{"Resize without clipping"}</button>
                </section>
            </div>
        }
    }

    fn view_pool_resolution(&self, ctx: &Context<Self>) -> Html {
        let entities = self.selected_entities();
        if entities.is_empty() || entities.iter().any(|entity| entity.as_blind().is_none()) {
            return Html::default();
        }
        let first_resolution = entities
            .first()
            .and_then(|entity| entity.as_blind())
            .map(|blind| blind.pixels_per_cell())
            .expect("a non-empty Pool selection has a resolution");
        let resolution = entities
            .iter()
            .all(|entity| {
                entity
                    .as_blind()
                    .is_some_and(|blind| blind.pixels_per_cell() == first_resolution)
            })
            .then_some(first_resolution)
            .map_or_else(|| "<different>".to_owned(), |value| value.to_string());
        let disabled = !self.can_edit_timeline
            || !matches!(self.rpc_state, RpcState::Online)
            || self
                .selected_entity_ids()
                .iter()
                .any(|entity_id| self.pending_entities.contains_key(entity_id));
        let collapsed = self.inspector_section_is_collapsed(InspectorSection::PoolResolution);
        html! {
            <section class="inspector-section pool-resolution">
                { self.view_inspector_section_header(ctx, InspectorSection::PoolResolution, "Pool resolution") }
                {
                    if !collapsed {
                        html! {
                            <div id={InspectorSection::PoolResolution.content_id()} class="inspector-section-body">
                                <label><span>{"Pixels per cell"}</span><input type="text" value={resolution} disabled={disabled} title={"Enter 1–32; this applies to every selected Pool"} onchange={ctx.link().callback(|event: Event| {
                                    let value = event.target_unchecked_into::<HtmlInputElement>().value().parse().unwrap_or(0);
                                    Msg::SetPoolResolution(value)
                                })} /></label>
                                <small>{if entities.len() > 1 { "<different> means selected Pools disagree. Changing resolution applies it to every selected Pool." } else { "Changing resolution resamples paint to nearest pixel center and is undoable." }}</small>
                            </div>
                        }
                    } else {
                        Html::default()
                    }
                }
            </section>
        }
    }

    fn view_shape_template_library(&self, ctx: &Context<Self>) -> Html {
        let grid_view = self.shape_template_view == ShapeTemplateView::Grid;
        let can_drag = self.mode == Mode::Select
            && self.workspace_tool == WorkspaceTool::Place
            && self.can_edit_timeline
            && matches!(self.rpc_state, RpcState::Online);
        let placement_kind = self.selected_placement_kind;
        html! {
            <section class="shape-template-library" aria-label="Project shape templates">
                <header class="shape-template-library-heading">
                    <div><small>{"PROJECT SHAPES"}</small><strong>{format!("{} templates", self.shape_catalog.len())}</strong></div>
                    <div class="shape-template-view-switcher" aria-label="Shape template view">
                        <button class={classes!(grid_view.then_some("active"))} aria-pressed={grid_view.to_string()} onclick={ctx.link().callback(|_| Msg::SetShapeTemplateView(ShapeTemplateView::Grid))}>{"Grid"}</button>
                        <button class={classes!((!grid_view).then_some("active"))} aria-pressed={(!grid_view).to_string()} onclick={ctx.link().callback(|_| Msg::SetShapeTemplateView(ShapeTemplateView::List))}>{"List"}</button>
                    </div>
                </header>
                {
                    if self.shape_catalog_pending && self.shape_catalog.is_empty() {
                        html! { <p class="shape-template-empty">{"Loading project shapes..."}</p> }
                    } else if self.shape_catalog.is_empty() {
                        html! { <p class="shape-template-empty">{"No project shapes saved."}</p> }
                    } else if grid_view {
                        html! {
                            <div class="shape-template-grid" aria-label="Shape template grid">
                                { for self.shape_catalog.iter().map(|sample| {
                                    let shape = Shape::new(sample.shape.width, sample.shape.height, sample.shape.occupied_mask);
                                    let draggable = can_drag && shape.is_ok();
                                    let drag_shape = shape.unwrap_or_else(|_| Shape::new(1, 1, 1).expect("fallback shape is valid"));
                                    let shape_id = sample.id.clone();
                                    let selected = self.active_shape_template_id.as_ref() == Some(&sample.id);
                                    html! {
                                        <button type="button" key={sample.id.clone()} class={classes!("shape-template-card", "grid", selected.then_some("active"))} title={sample.name.clone()} aria-label={format!("{} — {} by {} shape", sample.name, sample.shape.width, sample.shape.height)} aria-pressed={selected.to_string()} draggable={draggable.to_string()} onclick={ctx.link().callback(move |_| Msg::SelectShapeTemplate(shape_id.clone()))} ondragstart={template_drag_callback(ctx, placement_kind, drag_shape)} ondragend={ctx.link().callback(|_| Msg::EndPlacementDrag)}>
                                            { shape_template_thumbnail(sample) }
                                        </button>
                                    }
                                }) }
                            </div>
                        }
                    } else {
                        html! {
                            <div class="shape-template-list" aria-label="Shape template list">
                                { for self.shape_catalog.iter().map(|sample| {
                                    let shape = Shape::new(sample.shape.width, sample.shape.height, sample.shape.occupied_mask);
                                    let draggable = can_drag && shape.is_ok();
                                    let drag_shape = shape.unwrap_or_else(|_| Shape::new(1, 1, 1).expect("fallback shape is valid"));
                                    let shape_id = sample.id.clone();
                                    let selected = self.active_shape_template_id.as_ref() == Some(&sample.id);
                                    html! {
                                        <button type="button" key={sample.id.clone()} class={classes!("shape-template-card", "list", selected.then_some("active"))} aria-pressed={selected.to_string()} draggable={draggable.to_string()} onclick={ctx.link().callback(move |_| Msg::SelectShapeTemplate(shape_id.clone()))} ondragstart={template_drag_callback(ctx, placement_kind, drag_shape)} ondragend={ctx.link().callback(|_| Msg::EndPlacementDrag)}>
                                            { shape_template_thumbnail(sample) }
                                            <div><strong>{sample.name.clone()}</strong><small>{format!("{} × {} · {} cells", sample.shape.width, sample.shape.height, shape_occupied_count(sample))}</small></div>
                                        </button>
                                    }
                                }) }
                            </div>
                        }
                    }
                }
                <p class="shape-template-note">{format!("Select a shape to use it as the {} entity and map-cell brush footprint. Choose Place Block or Place Pool, then drag the template onto the canvas.", placement_kind.label().to_ascii_lowercase())}</p>
            </section>
        }
    }

    fn view_block_decorator_controls(
        &self,
        ctx: &Context<Self>,
        entity: &PlaceableEntity,
        sync_blocked: bool,
    ) -> Html {
        if self.mode != Mode::Select || !matches!(entity.kind(), PlaceableEntityKind::Block(_)) {
            return Html::default();
        }
        let snapshot = self.model.timeline().snapshot();
        let ice_count = snapshot.ice_blocking_count(entity.id());
        let direction = snapshot.direction_mode(entity.id());
        let outgoing = key_locker_for_key(snapshot, entity.id());
        let incoming = key_locker_for_lock(snapshot, entity.id());
        let pending_assignment = self.key_locker_assignment.as_ref() == Some(entity.id());
        let disabled = !self.can_edit_timeline
            || sync_blocked
            || self.pending_entities.contains_key(entity.id());
        let outgoing_label = outgoing.and_then(|decorator| match decorator.kind() {
            DecoratorKind::KeyLocker { entity, .. } => Some(entity.to_string()),
            _ => None,
        });
        let incoming_label = incoming.and_then(|decorator| match decorator.kind() {
            DecoratorKind::KeyLocker { key, .. } => Some(key.to_string()),
            _ => None,
        });
        let collapsed = self.inspector_section_is_collapsed(InspectorSection::Decorators);
        html! {
            <section class="inspector-section decorator-controls">
                { self.view_inspector_section_header(ctx, InspectorSection::Decorators, "Decorators") }
                {
                    if !collapsed {
                        html! {
                            <div id={InspectorSection::Decorators.content_id()} class="inspector-section-body">
                <div class="decorator-row">
                    <div><strong>{"Ice"}</strong><small>{if ice_count == 0 { "disabled".to_owned() } else { format!("blocks {ice_count}") }}</small></div>
                    <button class={classes!((ice_count > 0).then_some("active"))} disabled={disabled} onclick={ctx.link().callback(|_| Msg::EditDecorator(DecoratorAction::ToggleIce))}>{if ice_count > 0 { "On" } else { "Off" }}<kbd>{"I"}</kbd></button>
                </div>
                <label class="decorator-count"><span>{"Blocking count"}</span><input type="number" min="0" value={ice_count.to_string()} disabled={disabled} onchange={ctx.link().callback(|event: Event| Msg::SetIceCount(event.target_unchecked_into::<HtmlInputElement>().value()))} /></label>
                <div class="decorator-heading"><span>{"Direction"}</span><kbd>{"D / cycle"}</kbd></div>
                <div class="direction-control">
                    { for [
                        (DirectionMode::Disabled, "None"),
                        (DirectionMode::Horizontal, "Horizontal"),
                        (DirectionMode::Vertical, "Vertical"),
                    ].into_iter().map(|(mode, label)| html! {
                        <button class={classes!((direction == mode).then_some("active"))} disabled={disabled} onclick={ctx.link().callback(move |_| Msg::EditDecorator(DecoratorAction::SetDirection(mode)))}>{label}</button>
                    }) }
                </div>
                <div class="decorator-heading"><span>{"Key & Locker"}</span><kbd>{"K"}</kbd></div>
                <div class="key-locker-state">
                    <div><span>{"Carries key"}</span><code title={outgoing_label.clone().unwrap_or_default()}>{outgoing_label.unwrap_or_else(|| "none".to_owned())}</code></div>
                    <div><span>{"Unlocked by"}</span><code title={incoming_label.clone().unwrap_or_default()}>{incoming_label.unwrap_or_else(|| "none".to_owned())}</code></div>
                </div>
                <button class={classes!("key-locker-action", pending_assignment.then_some("active"))} disabled={disabled} onclick={ctx.link().callback(|_| Msg::EditDecorator(DecoratorAction::BeginKeyLocker))}>{if pending_assignment { "Cancel assignment" } else if outgoing.is_some() { "Reassign locker" } else { "Assign locker" }}</button>
                { if pending_assignment { html! { <p class="inspector-note assignment-prompt">{"Click a different available Block on the canvas. Invalid targets keep assignment active."}</p> } } else { Html::default() } }
                            </div>
                        }
                    } else {
                        Html::default()
                    }
                }
            </section>
        }
    }

    fn view_blind_decorator_controls(
        &self,
        ctx: &Context<Self>,
        entity: &PlaceableEntity,
        sync_blocked: bool,
    ) -> Html {
        if self.mode != Mode::Select || entity.as_blind().is_none() {
            return Html::default();
        }
        let glass_count = self
            .model
            .timeline()
            .snapshot()
            .glass_blocking_count(entity.id());
        let disabled = !self.can_edit_timeline
            || sync_blocked
            || self.pending_entities.contains_key(entity.id());
        let collapsed = self.inspector_section_is_collapsed(InspectorSection::Decorators);
        html! {
            <section class="inspector-section decorator-controls">
                { self.view_inspector_section_header(ctx, InspectorSection::Decorators, "Pool decorators") }
                {
                    if !collapsed {
                        html! {
                            <div id={InspectorSection::Decorators.content_id()} class="inspector-section-body">
                                <div class="decorator-row">
                                    <div><strong>{"Glass"}</strong><small>{if glass_count == 0 { "disabled".to_owned() } else { format!("blocks {glass_count}") }}</small></div>
                                    <button class={classes!((glass_count > 0).then_some("active"))} disabled={disabled} onclick={ctx.link().callback(|_| Msg::EditDecorator(DecoratorAction::ToggleGlass))}>{if glass_count > 0 { "On" } else { "Off" }}<kbd>{"G"}</kbd></button>
                                </div>
                                <label class="decorator-count"><span>{"Blocking count"}</span><input type="number" min="0" value={glass_count.to_string()} disabled={disabled} onchange={ctx.link().callback(|event: Event| Msg::SetGlassCount(event.target_unchecked_into::<HtmlInputElement>().value()))} /></label>
                                <p class="inspector-note">{"Glass is saved on this Pool and does not alter Sandbox simulation."}</p>
                            </div>
                        }
                    } else {
                        Html::default()
                    }
                }
            </section>
        }
    }

    fn view_canvas(&self, ctx: &Context<Self>) -> Html {
        let size = self.model.timeline().snapshot().size();
        let connection_label = match self.rpc_state {
            RpcState::Online => "SERVER LIVE",
            RpcState::Connecting => "CONNECTING",
            RpcState::Resyncing => "RESYNCING",
            RpcState::Offline => "RECONNECTING",
        };
        html! {
            <main class="workbench">
                <div class="canvas-stage">
                    <div class="canvas-primary-tool-dock">
                        { self.view_primary_toolbox(ctx) }
                    </div>
                    <div class="canvas-secondary-tool-dock">
                        { self.view_tool_selector(ctx) }
                        { if self.image_studio.apply.is_none() { html! { <div class="canvas-viewport-controls" aria-label="Level zoom controls">
                            <div class="viewport-tools">
                                <button class="tool-button" aria-label="Zoom out" title="Zoom out" onclick={ctx.link().callback(|_| Msg::ZoomOut)}>{ui_icon(UiIcon::ZoomOut, 14)}</button>
                                <button class="tool-button" aria-label="Frame the full grid" title="Frame the full grid" onclick={ctx.link().callback(|_| Msg::FrameGrid)}>{ui_icon(UiIcon::Frame, 14)}</button>
                                <button class="tool-button" aria-label="Zoom in" title="Zoom in" onclick={ctx.link().callback(|_| Msg::ZoomIn)}>{ui_icon(UiIcon::ZoomIn, 14)}</button>
                            </div>
                            <div class="canvas-readout">
                                <span><i class={classes!("status-dot", if matches!(self.rpc_state, RpcState::Online) { "online" } else { "local" })}></i>{connection_label}</span>
                                <code>{format!("{} EV", self.activity_events().len())}</code>
                                <code>{format!("{} PENDING", self.pending_commands.len())}</code>
                                <span>{format!("{:.0}%", self.viewport.scale * 100.0)}</span>
                            </div>
                        </div> } } else { Html::default() } }
                    </div>
                    <div class="axis-label axis-y">{"Y / ROW"}</div>
                    <canvas
                        ref={self.canvas_ref.clone()}
                        width={self.canvas_width.to_string()}
                        height={self.canvas_height.to_string()}
                        class={classes!(self.entity_drag.as_ref().is_some_and(|drag| drag.active).then_some("entity-dragging"), self.pan_gesture.is_some().then_some("panning"), self.placement_drag.is_some().then_some("placement-target"), self.map_resize_gesture.as_ref().is_some_and(|gesture| matches!(gesture.edge, MapResizeEdge::Left | MapResizeEdge::Right)).then_some("resize-horizontal"), self.map_resize_gesture.as_ref().is_some_and(|gesture| matches!(gesture.edge, MapResizeEdge::Top | MapResizeEdge::Bottom)).then_some("resize-vertical"))}
                        aria-label={format!("{} by {} editable logical map", size.width(), size.height())}
                        ondragover={ctx.link().callback(Msg::CanvasDragOver)}
                        ondrop={ctx.link().callback(Msg::CanvasDrop)}
                        onpointerdown={ctx.link().callback(Msg::CanvasDown)}
                        onpointermove={ctx.link().callback(Msg::CanvasMove)}
                        onpointerup={ctx.link().callback(Msg::CanvasUp)}
                        onpointercancel={ctx.link().callback(Msg::CanvasCancel)}
                        onpointerleave={ctx.link().callback(|_| Msg::CanvasLeave)}
                        onwheel={ctx.link().callback(Msg::CanvasWheel)}
                        oncontextmenu={Callback::from(|event: web_sys::MouseEvent| event.prevent_default())}
                    ></canvas>
                    <div class="axis-label axis-x">{"X / COLUMN"}</div>
                    {
                        if self.mode.is_parity_gated() && self.image_studio.apply.is_none() {
                            html! { <div class="parity-banner"><strong>{format!("{} PARITY GATED", self.mode.label().to_ascii_uppercase())}</strong><span>{"Visible for source-editor mode parity; commands are disabled in this MVP."}</span></div> }
                        } else {
                            Html::default()
                        }
                    }
                </div>
            </main>
        }
    }

    fn view_tool_selector(&self, ctx: &Context<Self>) -> Html {
        if self.image_studio.apply.is_some() { return self.view_apply_image_dock(ctx); }
        if self.mode == Mode::Distribution { return self.view_distribution_tools(ctx); }
        if self.mode == Mode::Select {
            let placing_block = self.workspace_tool == WorkspaceTool::Place
                && self.selected_placement_kind == PlacementKind::Block;
            let placing_pool = self.workspace_tool == WorkspaceTool::Place
                && self.selected_placement_kind == PlacementKind::Blind;
            let sand_studio_available = self.selected_entity_id().is_some_and(|entity_id| {
                self.model
                    .timeline()
                    .snapshot()
                    .entity(entity_id)
                    .is_some_and(|entity| entity.as_blind().is_some())
            });
            return html! {
                <nav class="canvas-toolbox contextual-canvas-tools" aria-label="Entity Placement context tools">
                    <button class={classes!(placing_block.then_some("active"))} aria-pressed={placing_block.to_string()} onclick={ctx.link().callback(|_| Msg::SetPlacementContext(PlacementKind::Block))}><Square size={14} /><span>{"Place Block"}</span></button>
                    <button class={classes!(placing_pool.then_some("active"))} aria-pressed={placing_pool.to_string()} onclick={ctx.link().callback(|_| Msg::SetPlacementContext(PlacementKind::Blind))}><Shapes size={14} /><span>{"Place Pool"}</span></button>
                    { if sand_studio_available { html! { <button onclick={ctx.link().callback(|_| Msg::OpenSandStudio)}>{ui_icon(UiIcon::ImageStudio, 14)}<span>{"Sand Studio"}</span></button> } } else { Html::default() } }
                </nav>
            };
        }
        if self.mode == Mode::Brush {
            let painting = self.workspace_tool == WorkspaceTool::Paint;
            let filling = self.workspace_tool == WorkspaceTool::Fill;
            return html! {
                <nav class="canvas-toolbox contextual-canvas-tools sand-studio-toolbox" aria-label="Sand Studio tools">
                    <button class={classes!(painting.then_some("active"))} aria-pressed={painting.to_string()} onclick={ctx.link().callback(|_| Msg::SetWorkspaceTool(WorkspaceTool::Paint))}><Square size={14} /><span>{"Paint"}</span></button>
                    <button class={classes!(filling.then_some("active"))} aria-pressed={filling.to_string()} onclick={ctx.link().callback(|_| Msg::SetWorkspaceTool(WorkspaceTool::Fill))}>{ui_icon(UiIcon::MapDesign, 14)}<span>{"Fill"}</span></button>
                    <button onclick={ctx.link().callback(|_| Msg::SetMode(Mode::Select))}><MousePointer2 size={14} /><span>{"Exit Sand Studio"}</span></button>
                </nav>
            };
        }
        if self.mode == Mode::Map {
            let drawing_walls = self.workspace_tool == WorkspaceTool::Cells
                && self.map_paint_kind == Some(CellKind::Wall);
            let clearing_walls = self.workspace_tool == WorkspaceTool::Cells
                && self.map_paint_kind == Some(CellKind::Floor);
            let resizing = self.workspace_tool == WorkspaceTool::Resize;
            return html! {
                <nav class="canvas-toolbox contextual-canvas-tools" aria-label="Map Design context tools">
                    <button class={classes!(drawing_walls.then_some("active"))} aria-pressed={drawing_walls.to_string()} onclick={ctx.link().callback(|_| Msg::SetMapPaintKind(CellKind::Wall))}>{ui_icon(UiIcon::DrawWall, 14)}<span>{"Draw Wall"}</span></button>
                    <button class={classes!(clearing_walls.then_some("active"))} aria-pressed={clearing_walls.to_string()} onclick={ctx.link().callback(|_| Msg::SetMapPaintKind(CellKind::Floor))}>{ui_icon(UiIcon::ClearWall, 14)}<span>{"Clear Wall"}</span></button>
                    <button class={classes!(resizing.then_some("active"))} aria-pressed={resizing.to_string()} onclick={ctx.link().callback(|_| Msg::SetWorkspaceTool(WorkspaceTool::Resize))}>{ui_icon(UiIcon::Resize, 14)}<span>{"Resize"}</span></button>
                    <button disabled=true title="Requires an atomic trim command">{ui_icon(UiIcon::MapDesign, 14)}<span>{"Strip Map"}</span></button>
                    <button disabled=true title="Requires an atomic clear command">{ui_icon(UiIcon::ClearWall, 14)}<span>{"Clear All"}</span></button>
                </nav>
            };
        }
        Html::default()
    }

    fn view_right_sidebar(&self, ctx: &Context<Self>) -> Html {
        let tabs = [
            RightTab::Inspector,
            RightTab::Blame,
            RightTab::LevelStructure,
            RightTab::LevelConfiguration,
            RightTab::AdvancedDistribution,
        ];
        let toggle_layout_label = self.right_panel_layout.toggle_label();
        let toggle_layout_icon = if self.right_panel_layout == PanelLayout::Docked {
            UiIcon::Float
        } else {
            UiIcon::DockRight
        };
        html! {
            <aside class={classes!("right-sidebar", "panel", (self.right_panel_layout == PanelLayout::Floating).then_some("panel-floating"))}>
                <div class="panel-tabs compact-tab-bar">
                    { for tabs.into_iter().filter(|tab| *tab != RightTab::AdvancedDistribution || self.mode == Mode::Distribution).map(|tab| {
                        let active = self.right_tab == tab;
                        html! {
                            <button
                                class={classes!("sidebar-tab", active.then_some("active"))}
                                title={tab.label()}
                                aria-pressed={active.to_string()}
                                onclick={ctx.link().callback(move |_| Msg::SetRightTab(tab))}
                            ><span class="sidebar-tab-icon">{ui_icon(tab.icon(), 14)}</span><span class="sidebar-tab-label">{tab.label()}</span></button>
                        }
                    }) }
                    <div class="panel-layout-actions"><button aria-label={format!("{} inspector panel", toggle_layout_label)} title={format!("{} inspector panel", toggle_layout_label)} onclick={ctx.link().callback(|_| Msg::TogglePanelLayout(SidebarSide::Right))}>{ui_icon(toggle_layout_icon, 12)}</button><button aria-label="Hide panel" title="Hide panel" onclick={ctx.link().callback(|_| Msg::SetPanelLayout(SidebarSide::Right, PanelLayout::Hidden))}>{ui_icon(UiIcon::HideRight, 12)}</button></div>
                </div>
                <div class="right-panel-content">
                    {
                        match self.right_tab {
                            RightTab::Inspector => self.view_selection_inspector(ctx),
                            RightTab::Blame => self.view_blame_panel(self.blame_for_selection()),
                            RightTab::LevelStructure => self.view_level_structure(ctx),
                            RightTab::LevelConfiguration => self.view_level_configuration_panel(ctx),
                            RightTab::AdvancedDistribution => self.view_distribution_panel(ctx),
                        }
                    }
                </div>
            </aside>
        }
    }

    fn view_activity_feed(&self) -> Html {
        let events = self.activity_events();
        let source = if self.server_sequence.is_some() {
            "server"
        } else {
            "local"
        };
        html! {
            <div class="activity-content">
                <div class="activity-summary">
                    <div><small>{"SESSION"}</small><strong>{format!("{:02}", events.len())}</strong><span>{"events"}</span></div>
                    <div><small>{"SOURCE"}</small><strong>{if self.server_sequence.is_some() { "RPC" } else { "LOC" }}</strong><span>{source}</span></div>
                    <div><small>{"STATE"}</small><strong>{"OK"}</strong><span>{"validated"}</span></div>
                </div>
                <div class="section-title"><span>{"COMMAND TIMELINE"}</span><code>{"NEWEST"}</code></div>
                <div class="event-list">
                    {
                        if events.is_empty() {
                            html! { <div class="empty-feed"><span>{"NO LOCAL EVENTS"}</span><p>{"Edit a map cell or placeable to start the timeline."}</p></div> }
                        } else {
                            html! { for event in events.iter().rev().take(14) {
                                {view_history_event(event)}
                            } }
                        }
                    }
                </div>
            </div>
        }
    }

    fn view_blame_panel(&self, blame: Option<BlameEntry>) -> Html {
        html! {
            <div class="activity-content blame-detail">
                <div class="section-title"><span>{"SELECTED PROVENANCE"}</span><code>{"LIVE"}</code></div>
                {
                    if let (Some(selection), Some(entry)) = (self.selection.as_ref(), blame) {
                        let history = self.history_for_selection(selection);
                        let (target_kind, target_value, history_label) = match selection {
                            Selection::Cell(point) => ("Cell", format!("{:02}:{:02}", point.x, point.y), "CELL HISTORY"),
                            Selection::Entities(entity_ids) => ("Entity", entity_ids[0].to_string(), "ENTITY HISTORY"),
                        };
                        html! {
                            <>
                                <div class="blame-hero">
                                <span class="actor-mark large">{actor_mark(entry.actor.as_str())}</span>
                                    <div><small>{"CURRENT ACTOR"}</small><strong>{entry.actor.to_string()}</strong><span>{format_time(entry.occurred_at_ms)}</span></div>
                                </div>
                                <div class="property-table right-properties">
                                    <div><span>{target_kind}</span><code title={target_value.clone()}>{target_value}</code></div>
                                    <div><span>{"Sequence"}</span><strong>{format!("#{:04}", entry.sequence)}</strong></div>
                                    <div><span>{"Command"}</span><code>{entry.command_id.to_string()}</code></div>
                                    <div><span>{"Reverts"}</span><strong>{entry.reverts_sequence.map(|seq| format!("#{seq:04}")).unwrap_or_else(|| "none".to_owned())}</strong></div>
                                </div>
                                <div class="section-title"><span>{history_label}</span><code>{history.len()}</code></div>
                                <div class="event-list">{ html! { for event in history.into_iter().rev() {
                                    {view_history_event(event)}
                                } } }</div>
                            </>
                        }
                    } else {
                        html! { <div class="empty-feed"><span>{"NO PROVENANCE"}</span><p>{"Select a changed cell or entity to inspect actor, time, and sequence."}</p></div> }
                    }
                }
            </div>
        }
    }

    fn view_status(&self, ctx: &Context<Self>) -> Html {
        let local_hash = self.model.timeline().snapshot().content_hash().to_string();
        let hash = if matches!(self.rpc_state, RpcState::Online | RpcState::Resyncing) {
            self.server_hash.as_deref().unwrap_or(&local_hash)
        } else {
            &local_hash
        };
        let rpc_label = match self.rpc_state {
            RpcState::Connecting => "RPC CONNECTING",
            RpcState::Online => "RPC ONLINE",
            RpcState::Resyncing => "RPC RESYNCING",
            RpcState::Offline => "LIVE RETRY / READ ONLY",
        };
        let endpoint = self
            .rpc
            .as_ref()
            .map(RpcClient::endpoint)
            .unwrap_or("ws(s)://host/rpc");
        let theme_source = if self.theme.user_override.is_some() {
            "User override"
        } else {
            "Project default"
        };
        let shortcut_hints: &[&str] = match (self.mode, self.workspace_tool) {
            (Mode::Select, WorkspaceTool::Transform) => {
                &["Click select", "Shift+Click multi", "Arrows move"]
            }
            (Mode::Select, WorkspaceTool::Place) => &["Drag template to place"],
            (Mode::Select, WorkspaceTool::Decorate) => {
                &["I ice", "G glass", "D direction", "K locker"]
            }
            (Mode::Map, WorkspaceTool::Cells) => &["Drag paint cells"],
            (Mode::Map, WorkspaceTool::Resize) => &["Drag map edge"],
            (Mode::Brush, WorkspaceTool::Paint) => &["B paint", "Shift+drag erase", "1–0 color"],
            (Mode::Brush, WorkspaceTool::Fill) => &["F fill", "1–0 color"],
            (Mode::Distribution, _) => &["Click Pools / Blocks", "Shift+click add", "Drag empty space to select"],
            (Mode::Sandbox, _) => &["Sandbox parity gated"],
            _ => &[],
        };
        html! {
            <footer class="statusbar">
                <span class="status-primary"><i class={classes!("status-dot", if matches!(self.rpc_state, RpcState::Online) { "online" } else { "local" })}></i>{rpc_label}</span>
                <span title={endpoint.to_owned()}>{format!("{} / {}", self.project_display_name, self.level_display_name)}</span>
                <span>{format!("MODE {}", self.mode.key())}</span>
                <span>{selection_status(self.selection.as_ref())}</span>
                <span>{self.server_sequence.map(|sequence| if matches!(self.rpc_state, RpcState::Online | RpcState::Resyncing) { format!("SEQ #{sequence:04}") } else { format!("LAST #{sequence:04}") }).unwrap_or_else(|| "SEQ LOCAL".to_owned())}</span>
                <span class="status-hash">{format!("HASH {}", &hash[..12.min(hash.len())])}</span>
                <span title={self.model.actor().to_string()}>{format!("ACTOR {}", actor_mark(self.model.actor().as_str()))}</span>
                <span class="status-theme-source" title={format!("{} theme / {}", self.theme.active().label(), theme_source)}>
                    <i class="source-dot"></i>
                    <span>{format!("THEME {}", self.theme.active().label().to_ascii_uppercase())}</span>
                    <small>{theme_source}</small>
                    { if self.theme.user_override.is_some() { html! { <button onclick={ctx.link().callback(|_| Msg::UseProjectTheme)}>{"Reset"}</button> } } else { Html::default() } }
                </span>
                <span class="status-spacer"></span>
                { for shortcut_hints.iter().map(|hint| html! { <span>{*hint}</span> }) }
                <span>{"Ctrl+K commands"}</span>
                <span>{"Ctrl+Z undo"}</span>
            </footer>
        }
    }

    fn view_palette(&self, ctx: &Context<Self>) -> Html {
        if !self.palette_open {
            return Html::default();
        }
        let commands = self.filtered_palette_commands();
        html! {
            <div class="palette-backdrop" onclick={ctx.link().callback(|_| Msg::CloseOverlays)}>
                <section class="command-palette" role="dialog" aria-modal="true" onclick={Callback::from(|event: MouseEvent| event.stop_propagation())}>
                    <div class="palette-input-row">
                        <span>{">"}</span>
                        <input
                            ref={self.palette_input_ref.clone()}
                            value={self.palette_query.clone()}
                            placeholder="Type a command or mode..."
                            autocomplete="off"
                            oninput={ctx.link().callback(|event: InputEvent| {
                                let input: HtmlInputElement = event.target_unchecked_into();
                                Msg::PaletteQuery(input.value())
                            })}
                        />
                        <kbd>{"ESC"}</kbd>
                    </div>
                    <div class="palette-section-label">{"AVAILABLE COMMANDS"}</div>
                    <div class="palette-results">
                        {
                            if commands.is_empty() {
                                html! { <div class="palette-empty">{"No matching command"}</div> }
                            } else {
                                html! { for (index, command) in commands.iter().copied().enumerate() {
                                    <button
                                        key={command.label()}
                                        class={if index == 0 { "selected" } else { "" }}
                                        onclick={ctx.link().callback(move |_| Msg::RunPalette(command))}
                                    >
                                        <span class="command-index">{format!("{:02}", index + 1)}</span>
                                        <strong>{command.label()}</strong>
                                        <kbd>{command.shortcut()}</kbd>
                                    </button>
                                } }
                            }
                        }
                    </div>
                    <div class="palette-footer"><span>{"Enter to run"}</span><span>{"Keyboard scope: palette"}</span></div>
                </section>
            </div>
        }
    }

    fn view_toasts(&self, ctx: &Context<Self>) -> Html {
        let visible_toasts = self
            .toasts
            .iter()
            .rev()
            .filter(|toast| self.active_toasts.contains(&toast.id))
            .take(4)
            .collect::<Vec<_>>();
        html! {
            <div class="toast-stack" aria-live="polite">
                { html! { for toast in visible_toasts.into_iter().rev() {
                    <div key={toast.id} class={classes!("toast", toast.tone)}>
                        <span class="toast-signal"></span>
                        <p>{&toast.message}</p>
                        <button
                            aria-label="Dismiss notification"
                            onclick={{
                                let id = toast.id;
                                ctx.link().callback(move |_| Msg::DismissToast(id))
                            }}
                        >{"x"}</button>
                    </div>
                } } }
            </div>
        }
    }

    fn view_notifications(&self, ctx: &Context<Self>) -> Html {
        if !self.notifications_open {
            return Html::default();
        }
        html! {
            <section class="notification-center" role="dialog" aria-label="Notification history">
                <header><div><small>{"NOTIFICATION CENTER"}</small><strong>{format!("{} recorded", self.toasts.len())}</strong></div><button disabled={self.toasts.is_empty()} onclick={ctx.link().callback(|_| Msg::ClearNotifications)}>{"Clear all"}</button></header>
                <div class="notification-list">
                    { if self.toasts.is_empty() {
                        html! { <div class="notification-empty">{"No notifications recorded."}</div> }
                    } else {
                        html! { for notification in self.toasts.iter().rev() {
                            <article key={notification.id} class={classes!("notification-record", notification.tone)}>
                                <span class="toast-signal"></span>
                                <div><p>{&notification.message}</p><small>{format_time(notification.occurred_at_ms)}</small></div>
                            </article>
                        } }
                    } }
                </div>
            </section>
        }
    }

    fn enter_editor(&mut self, ctx: &Context<Self>, user: UserSummary) {
        self.invalidate_image_studio();
        self.invalidate_rpc();
        self.advance_level_intent();
        self.reset_level_configuration();
        self.selected_workspace_id = user.personal_workspace_id.clone();
        self.inspector_collapsed = load_inspector_collapsed_sections(&user.id);
        self.model = EditorModel::blank(&user.id, &self.session_id);
        self.selection = None;
        self.drag_kind = None;
        self.map_paint_kind = None;
        self.last_drag = None;
        self.blind_gesture = None;
        self.entity_drag = None;
        self.pan_gesture = None;
        self.placement_drag = None;
        self.map_resize_gesture = None;
        self.marquee_gesture = None;
        self.isolated_blind = None;
        self.key_locker_assignment = None;
        self.shape_catalog.clear();
        self.selected_shape_id = None;
        self.active_shape_template_id = None;
        self.shape_catalog_pending = false;
        self.image_catalog.clear();
        self.image_catalog_pending = false;
        self.studio_modal = None;
        self.palette_open = false;
        self.open_menu = None;
        self.user = Some(user);
        self.phase = Phase::Editor;
        self.workspaces.clear();
        self.projects.clear();
        self.catalog_levels.clear();
        self.catalog_invitations.clear();
        self.catalog_project = None;
        self.explorer_action = None;
        self.expanded_workspaces.clear();
        self.expanded_projects.clear();
        self.workspace_draft_name.clear();
        self.project_name.clear();
        self.level_name.clear();
        self.level_duration = "0".to_owned();
        self.workspace_name = "No workspace".to_owned();
        self.project_display_name = "No project".to_owned();
        self.level_display_name = "No level".to_owned();
        self.target = ProjectLevelTarget::new(String::new(), String::new());
        self.level_duration_seconds = 0.0;
        self.can_edit_timeline = false;
        self.canvas_size_initialized = false;
        self.frame_grid();
        self.left_tab = LeftTab::Explorer;
        self.auto_open_initial_level = true;
        self.form_error = None;
        self.canvas_dirty = true;
        self.load_catalog(ctx);
    }

    fn load_catalog(&mut self, ctx: &Context<Self>) {
        self.catalog_request_generation = self.catalog_request_generation.wrapping_add(1);
        let generation = self.catalog_request_generation;
        self.request_pending = true;
        let link = ctx.link().clone();
        wasm_bindgen_futures::spawn_local(async move {
            link.send_message(Msg::CatalogLoaded {
                generation,
                result: RestClient.catalog().await,
            });
        });
    }

    fn invite_project_member(&mut self, ctx: &Context<Self>) -> bool {
        let Some(project) = self.configuration_project.as_ref() else {
            return false;
        };
        if self.configuration_pending
            || !project.can_manage_members()
            || self.invitation_email.trim().is_empty()
        {
            return false;
        }
        self.configuration_pending = true;
        self.configuration_error = None;
        let project_id = project.id.clone();
        let request_project_id = project_id.clone();
        let email = self.invitation_email.trim().to_owned();
        let roles = vec![self.invitation_role.clone()];
        let link = ctx.link().clone();
        wasm_bindgen_futures::spawn_local(async move {
            link.send_message(Msg::ProjectMemberInvited {
                project_id: request_project_id,
                result: RestClient
                    .invite_project_member(&project_id, &email, &roles)
                    .await,
            });
        });
        true
    }

    fn open_level(&mut self, ctx: &Context<Self>, project: ProjectSummary, level: LevelSummary) {
        self.invalidate_image_studio();
        let Some(user_id) = self.user.as_ref().map(|user| user.id.clone()) else {
            self.clear_session();
            return;
        };
        self.advance_level_intent();
        self.reset_level_configuration();
        let key = draft_storage_key(&user_id, &project.id, &level.id);
        self.model = load_draft(&key)
            .map(|snapshot| EditorModel::from_snapshot(snapshot, &user_id, &self.session_id))
            .unwrap_or_else(|| EditorModel::blank(&user_id, &self.session_id));
        self.selected_workspace_id = project.workspace_id.clone();
        self.workspace_name = self
            .workspaces
            .iter()
            .find(|workspace| workspace.id == project.workspace_id)
            .map(|workspace| workspace.name.clone())
            .unwrap_or_else(|| project.workspace_id.clone());
        self.project_display_name = project.name.clone();
        self.level_display_name = level.name.clone();
        self.level_duration_seconds = level.duration_seconds;
        self.target = ProjectLevelTarget::new(project.id.clone(), level.id.clone());
        self.can_edit_timeline = project.can_edit_timeline();
        self.theme.project_default =
            Theme::from_name(&project.default_theme).unwrap_or(Theme::Dark);
        self.expanded_workspaces
            .insert(project.workspace_id.clone());
        self.expanded_projects.insert(project.id.clone());
        self.catalog_project = Some(project);
        self.explorer_action = None;
        self.auto_open_initial_level = false;
        self.phase = Phase::Editor;
        self.selection = None;
        self.isolated_blind = None;
        self.shape_catalog.clear();
        self.selected_shape_id = None;
        self.active_shape_template_id = None;
        self.shape_draft_name.clear();
        self.shape_draft_mask = 1;
        self.shape_catalog_pending = true;
        self.image_catalog.clear();
        self.image_catalog_pending = false;
        self.image_draft_name.clear();
        self.image_file = None;
        self.image_import_error = None;
        self.server_events.clear();
        self.remote_blame.clear();
        self.remote_entity_blame.clear();
        self.seen_commands.clear();
        self.server_sequence = None;
        self.server_hash = None;
        self.sync_resize_fields();
        self.frame_grid();
        self.canvas_dirty = true;
        self.start_connection(ctx);
        let project_id = self.target.project_id.to_string();
        let request_project_id = project_id.clone();
        let link = ctx.link().clone();
        wasm_bindgen_futures::spawn_local(async move {
            link.send_message(Msg::ShapeCatalogLoaded {
                project_id: request_project_id,
                result: RestClient.list_shape_catalog(&project_id).await,
            });
        });
        self.load_image_catalog(ctx);
        self.load_prepared_catalog(ctx);
    }

    fn unbind_level(&mut self) {
        self.invalidate_image_studio();
        self.invalidate_rpc();
        self.advance_level_intent();
        self.reset_level_configuration();
        if let Some(user) = self.user.as_ref() {
            self.model = EditorModel::blank(&user.id, &self.session_id);
        }
        self.target = ProjectLevelTarget::new(String::new(), String::new());
        self.workspace_name = self
            .workspaces
            .iter()
            .find(|workspace| workspace.id == self.selected_workspace_id)
            .map(|workspace| workspace.name.clone())
            .unwrap_or_else(|| "No workspace".to_owned());
        self.project_display_name = "No project".to_owned();
        self.level_display_name = "No level".to_owned();
        self.level_duration_seconds = 0.0;
        self.can_edit_timeline = false;
        self.catalog_project = None;
        self.selection = None;
        self.isolated_blind = None;
        self.shape_catalog.clear();
        self.selected_shape_id = None;
        self.active_shape_template_id = None;
        self.image_catalog.clear();
        self.server_events.clear();
        self.remote_blame.clear();
        self.remote_entity_blame.clear();
        self.server_sequence = None;
        self.server_hash = None;
        self.left_tab = LeftTab::Explorer;
        self.frame_grid();
        self.canvas_dirty = true;
    }

    fn advance_level_intent(&mut self) {
        self.level_intent_generation = self.level_intent_generation.wrapping_add(1);
    }

    fn reset_level_configuration(&mut self) {
        self.level_configuration_generation = self.level_configuration_generation.wrapping_add(1);
        self.level_configuration_open = false;
        self.level_configuration_pending = false;
        self.level_configuration_error = None;
        self.level_configuration_name.clear();
        self.level_configuration_duration.clear();
        if self.right_tab == RightTab::LevelConfiguration {
            self.right_tab = RightTab::Inspector;
        }
    }

    fn load_image_catalog(&mut self, ctx: &Context<Self>) {
        if self.phase != Phase::Editor
            || self.image_catalog_pending
            || self.target.project_id.as_str().is_empty()
        {
            return;
        }
        self.image_catalog_pending = true;
        let scope = self.image_scope().0;
        let project_id = self.target.project_id.to_string();
        let request_project_id = project_id.clone();
        let link = ctx.link().clone();
        wasm_bindgen_futures::spawn_local(async move {
            link.send_message(Msg::ImageCatalogLoaded {
                scope,
                project_id: request_project_id,
                result: RestClient.list_image_catalog(&project_id).await,
            });
        });
    }

    fn select_pasted_image(&mut self, event: ClipboardEvent) -> bool {
        if self.studio_modal != Some(StudioModal::Image)
            || self.image_catalog_pending
            || !self.can_edit_timeline
            || event_target_is_text_entry(event.target())
        {
            return false;
        }
        let Some(file) = event
            .clipboard_data()
            .and_then(|clipboard| clipboard.files())
            .and_then(|files| {
                (0..files.length())
                    .filter_map(|index| files.get(index))
                    .find(|file| supported_image_media_type(file).is_some())
            })
        else {
            return false;
        };
        event.prevent_default();
        self.select_image_file(Some(file))
    }

    fn select_image_file(&mut self, file: Option<File>) -> bool {
        let Some(file) = file else {
            return false;
        };
        const MAX_UPLOAD_BYTES: f64 = 512.0 * 1024.0;
        if supported_image_media_type(&file).is_none() {
            self.image_file = None;
            self.image_import_error = Some("Choose a PNG, JPEG, or WebP image.".to_owned());
            return true;
        }
        if file.size() > MAX_UPLOAD_BYTES {
            self.image_file = None;
            self.image_import_error = Some("Image files must be 512 KiB or smaller.".to_owned());
            return true;
        }
        let file_name = file.name();
        self.image_draft_name = file_name
            .rsplit_once('.')
            .map_or(file_name.as_str(), |(stem, _)| stem)
            .trim()
            .to_owned();
        if self.image_draft_name.is_empty() {
            self.image_draft_name = "Imported image".to_owned();
        }
        self.image_file = Some(file);
        self.image_import_error = None;
        true
    }

    fn import_image(&mut self, ctx: &Context<Self>) -> bool {
        if self.image_catalog_pending || !self.can_edit_timeline {
            return false;
        }
        let Some(file) = self.image_file.clone() else {
            self.image_import_error = Some("Choose an image first.".to_owned());
            return true;
        };
        let Some(media_type) = supported_image_media_type(&file) else {
            self.image_file = None;
            self.image_import_error = Some("Choose a PNG, JPEG, or WebP image.".to_owned());
            return true;
        };
        let name = self.image_draft_name.trim().to_owned();
        if name.is_empty() {
            self.image_import_error = Some("Give the shared template a name.".to_owned());
            return true;
        }
        self.image_catalog_pending = true;
        self.image_import_error = None;
        let (scope,generation) = self.image_scope();
        let project_id = self.target.project_id.to_string();
        let request_project_id = project_id.clone();
        let link = ctx.link().clone();
        wasm_bindgen_futures::spawn_local(async move {
            link.send_message(Msg::ImageImported {
                scope, generation,
                project_id: request_project_id,
                result: RestClient
                    .upload_image(&project_id, &name, file, media_type)
                    .await,
            });
        });
        true
    }

    fn accept_level_configuration(&mut self, configuration: LevelConfiguration) {
        self.level_display_name = configuration.name.clone();
        self.level_duration_seconds = configuration.duration_seconds;
        self.level_configuration_name = configuration.name.clone();
        self.level_configuration_duration = configuration.duration_seconds.to_string();
        for levels in self.catalog_levels.values_mut() {
            if let Some(level) = levels
                .iter_mut()
                .find(|level| level.id == self.target.level_id.as_str())
            {
                level.name.clone_from(&configuration.name);
                level.duration_seconds = configuration.duration_seconds;
            }
        }
    }

    fn save_level_configuration(&mut self, ctx: &Context<Self>) -> bool {
        if !self.level_configuration_open
            || self.level_configuration_pending
            || !self.can_edit_timeline
        {
            return false;
        }
        let name = self.level_configuration_name.trim().to_owned();
        let Ok(duration_seconds) = self.level_configuration_duration.parse::<f64>() else {
            self.level_configuration_error =
                Some("Duration must be a nonnegative number".to_owned());
            return true;
        };
        if name.is_empty() || !duration_seconds.is_finite() || duration_seconds < 0.0 {
            self.level_configuration_error =
                Some("Name is required and duration must be nonnegative".to_owned());
            return true;
        }
        self.level_configuration_pending = true;
        self.level_configuration_error = None;
        let target = self.target.clone();
        let generation = self.level_configuration_generation;
        let project_id = target.project_id.to_string();
        let level_id = target.level_id.to_string();
        let link = ctx.link().clone();
        wasm_bindgen_futures::spawn_local(async move {
            link.send_message(Msg::LevelConfigurationUpdated {
                target,
                generation,
                result: RestClient
                    .update_level_configuration(&project_id, &level_id, &name, duration_seconds)
                    .await,
            });
        });
        true
    }

    fn handle_authenticated_api_error(&mut self, error: ApiError) -> bool {
        self.request_pending = false;
        if error.is_unauthorized() {
            self.clear_session();
            self.form_error = Some("Your session expired. Authenticate again.".to_owned());
        } else {
            self.form_error = Some(api_error_message(&error));
        }
        true
    }

    fn clear_session(&mut self) {
        self.invalidate_image_studio();
        self.invalidate_rpc();
        self.catalog_request_generation = self.catalog_request_generation.wrapping_add(1);
        self.advance_level_intent();
        self.reset_level_configuration();
        self.disconnect_canvas_resize_observer();
        self.phase = Phase::Authentication;
        self.user = None;
        self.workspaces.clear();
        self.projects.clear();
        self.catalog_project = None;
        self.catalog_levels.clear();
        self.catalog_invitations.clear();
        self.workspace_draft_name.clear();
        self.explorer_action = None;
        self.expanded_workspaces.clear();
        self.expanded_projects.clear();
        self.auto_open_initial_level = false;
        self.project_configuration = None;
        self.configuration_project = None;
        self.target = ProjectLevelTarget::new(String::new(), String::new());
        self.workspace_name.clear();
        self.project_display_name.clear();
        self.level_display_name.clear();
        self.level_duration_seconds = 0.0;
        self.level_configuration_open = false;
        self.can_edit_timeline = false;
        self.inspector_collapsed.clear();
        self.auth_password.clear();
    }

    fn invalidate_rpc(&mut self) {
        if self.mode == Mode::Brush {
            self.set_mode(Mode::Select);
        }
        self.connection_attempt += 1;
        self.rpc = None;
        self.reconnect_timer = None;
        self.reconnect_failures = 0;
        self.rpc_state = RpcState::Offline;
        self.presence.clear();
        self.desired_cursor = None;
        self.last_sent_cursor = None;
        self.cursor_timer = None;
        self.cursor_in_flight = false;
        self.image_request_interrupted();
        self.pending_commands.clear();
        self.pending_cells.clear();
        self.pending_entities.clear();
        self.pending_placements.clear();
        self.pending_grid = None;
        self.pending_edge_view = None;
        self.blind_gesture = None;
        self.entity_drag = None;
        self.pan_gesture = None;
        self.map_resize_gesture = None;
        self.marquee_gesture = None;
        self.placement_drag = None;
        self.key_locker_assignment = None;
        self.isolated_blind = None;
        self.server_sequence = None;
        self.server_hash = None;
        self.server_events.clear();
        self.remote_blame.clear();
        self.remote_entity_blame.clear();
        self.seen_commands.clear();
    }

    fn open_sand_studio(&mut self) -> bool {
        let Some(entity_id) = self.selected_entity_id().cloned() else {
            return false;
        };
        if self
            .model
            .timeline()
            .snapshot()
            .entity(&entity_id)
            .is_none_or(|entity| entity.as_blind().is_none())
        {
            return false;
        }
        let mut changed = self.set_mode(Mode::Brush);
        if self.isolated_blind.as_ref() != Some(&entity_id) {
            self.isolated_blind = Some(entity_id);
            changed = true;
        }
        self.workspace_tool = WorkspaceTool::Paint;
        self.blind_brush_tool = BlindBrushTool::Paint;
        self.canvas_dirty = true;
        changed
    }

    fn set_mode(&mut self, mode: Mode) -> bool {
        if self.mode == mode {
            return false;
        }
        let previous_mode = self.mode;
        self.distribution = DistributionState::default();
        if self.right_tab == RightTab::AdvancedDistribution { self.right_tab = RightTab::Inspector; }
        if mode == Mode::Brush {
            self.left_tab_before_sand_studio = Some(self.left_tab);
            self.left_panel_layout_before_sand_studio = Some(self.left_panel_layout);
            self.left_tab = LeftTab::SandStudio;
            self.left_panel_layout = PanelLayout::Docked;
        } else if previous_mode == Mode::Brush {
            self.left_tab = self
                .left_tab_before_sand_studio
                .take()
                .filter(|tab| *tab != LeftTab::SandStudio)
                .unwrap_or(LeftTab::ShapeTemplates);
            if let Some(layout) = self.left_panel_layout_before_sand_studio.take() {
                self.left_panel_layout = layout;
            }
        }
        self.mode = mode;
        if mode == Mode::Distribution && !matches!(self.selection, Some(Selection::Entities(_))) {
            self.selection = None;
        }
        self.workspace_tool = WorkspaceTool::default_for(mode);
        self.blind_brush_tool = BlindBrushTool::Paint;
        self.drag_kind = None;
        self.last_drag = None;
        self.blind_gesture = None;
        self.entity_drag = None;
        self.placement_drag = None;
        self.pan_gesture = None;
        self.map_resize_gesture = None;
        self.marquee_gesture = None;
        self.key_locker_assignment = None;
        if mode != Mode::Brush {
            self.isolated_blind = None;
        }
        if mode.is_parity_gated() {
            self.push_toast(
                format!("{} mode is visible but parity gated", mode.label()),
                "info",
            );
        }
        self.canvas_dirty = true;
        self.palette_open = false;
        true
    }

    fn set_workspace_tool(&mut self, tool: WorkspaceTool) -> bool {
        if !WorkspaceTool::for_mode(self.mode).contains(&tool) || self.workspace_tool == tool {
            return false;
        }
        self.workspace_tool = tool;
        self.drag_kind = None;
        self.last_drag = None;
        self.blind_gesture = None;
        self.entity_drag = None;
        self.placement_drag = None;
        self.map_resize_gesture = None;
        self.marquee_gesture = None;
        self.key_locker_assignment = None;
        match tool {
            WorkspaceTool::Paint => self.blind_brush_tool = BlindBrushTool::Paint,
            WorkspaceTool::Fill => self.blind_brush_tool = BlindBrushTool::FloodFill,
            _ => {}
        }
        self.canvas_dirty = true;
        true
    }

    fn select_and_focus_inspector(&mut self, selection: Selection) {
        self.selection = Some(selection);
        if self.mode == Mode::Distribution {
            if !matches!(self.selection, Some(Selection::Entities(_))) { self.selection = None; }
            self.sync_distribution_selection();
        } else {
            self.right_tab = RightTab::Inspector;
        }
    }

    fn select_entities_and_focus_inspector(&mut self, entity_ids: Vec<EntityId>) {
        self.select_and_focus_inspector(Selection::Entities(entity_ids));
    }

    fn set_panel_layout(&mut self, side: SidebarSide, layout: PanelLayout) -> bool {
        let current = match side {
            SidebarSide::Left => &mut self.left_panel_layout,
            SidebarSide::Right => &mut self.right_panel_layout,
        };
        if *current == layout {
            return false;
        }
        *current = layout;
        self.sidebar_resize = None;
        self.canvas_dirty = true;
        true
    }

    fn inspector_section_is_collapsed(&self, section: InspectorSection) -> bool {
        self.inspector_collapsed.contains(&section)
    }

    fn toggle_inspector_section(&mut self, section: InspectorSection) -> bool {
        if !self.inspector_collapsed.insert(section) {
            self.inspector_collapsed.remove(&section);
        }
        if let Some(user) = self.user.as_ref() {
            persist_inspector_collapsed_sections(&user.id, &self.inspector_collapsed);
        }
        true
    }

    fn view_inspector_section_header(
        &self,
        ctx: &Context<Self>,
        section: InspectorSection,
        title: &'static str,
    ) -> Html {
        let collapsed = self.inspector_section_is_collapsed(section);
        html! {
            <div class="inspector-section-heading">
                <h2>{title}</h2>
                <button
                    class="inspector-section-toggle"
                    type="button"
                    aria-controls={section.content_id()}
                    aria-expanded={(!collapsed).to_string()}
                    onclick={ctx.link().callback(move |_| Msg::ToggleInspectorSection(section))}
                >{if collapsed { "Expand" } else { "Collapse" }}</button>
            </div>
        }
    }

    fn begin_placement_drag(&mut self, kind: PlacementKind, shape: Shape) -> bool {
        if self.mode != Mode::Select
            || self.workspace_tool != WorkspaceTool::Place
            || !self.can_edit_timeline
            || !matches!(self.rpc_state, RpcState::Online)
        {
            return false;
        }
        self.placement_drag = Some(PlacementDrag {
            kind,
            shape,
            hover: None,
        });
        self.canvas_dirty = true;
        true
    }

    fn end_placement_drag(&mut self) -> bool {
        let changed = self.placement_drag.take().is_some();
        self.canvas_dirty |= changed;
        changed
    }

    fn canvas_drag_over(&mut self, event: DragEvent) -> bool {
        if self.placement_drag.is_none() {
            return false;
        }
        event.prevent_default();
        if let Some(data_transfer) = event.data_transfer() {
            data_transfer.set_drop_effect("copy");
        }
        let hover = self
            .canvas_position_from_client(event.client_x(), event.client_y())
            .and_then(|(x, y)| self.point_from_canvas_position(x, y));
        let changed = self
            .placement_drag
            .as_ref()
            .is_some_and(|drag| drag.hover != hover);
        if let Some(drag) = self.placement_drag.as_mut() {
            drag.hover = hover;
        }
        self.canvas_dirty |= changed;
        changed
    }

    fn canvas_drop(&mut self, ctx: &Context<Self>, event: DragEvent) -> bool {
        if self.placement_drag.is_none() {
            return false;
        }
        event.prevent_default();
        let point = self
            .canvas_position_from_client(event.client_x(), event.client_y())
            .and_then(|(x, y)| self.point_from_canvas_position(x, y));
        let Some(drag) = self.placement_drag.take() else {
            return false;
        };
        self.canvas_dirty = true;
        let Some(point) = point else {
            self.push_toast("Drop the template inside the level".to_owned(), "info");
            return true;
        };
        self.place_shape_at(ctx, drag.kind, drag.shape, point)
    }

    fn canvas_down(&mut self, ctx: &Context<Self>, event: PointerEvent) -> bool {
        if self.image_studio.apply.is_some() { return self.image_pointer_down(event); }
        if self.mode == Mode::Distribution && event.button() == 0 { return self.distribution_pointer_down(event); }
        event.prevent_default();
        if let Some(root) = self.root_ref.cast::<HtmlElement>() {
            let _ = root.focus();
        }
        if event.button() == 1 || event.button() == 2 {
            let Some((start_x, start_y)) = self.canvas_position(&event) else {
                return false;
            };
            self.capture_pointer(event.pointer_id());
            self.pan_gesture = Some(PanGesture {
                pointer_id: event.pointer_id(),
                start_x,
                start_y,
                offset_x: self.viewport.offset_x,
                offset_y: self.viewport.offset_y,
            });
            return true;
        }
        if event.button() == 0
            && self.mode == Mode::Map
            && self.workspace_tool == WorkspaceTool::Resize
            && self.can_edit_timeline
            && matches!(self.rpc_state, RpcState::Online)
            && self.pending_commands.is_empty()
        {
            let Some((start_x, start_y)) = self.canvas_position(&event) else {
                return false;
            };
            if let Some(edge) = self.map_resize_edge_at(start_x, start_y) {
                let snapshot = self.model.timeline().snapshot();
                let size = snapshot.size();
                let (_, anchor) = plan_content_aware_edge_resize(snapshot, edge, 0);
                self.capture_pointer(event.pointer_id());
                self.map_resize_gesture = Some(MapResizeGesture {
                    pointer_id: event.pointer_id(),
                    edge,
                    start_x,
                    start_y,
                    start_size: size,
                    preview_size: size,
                    anchor,
                    cell_size: CELL_SIZE * self.viewport.scale,
                });
                self.canvas_dirty = true;
                return true;
            }
        }
        if self.mode == Mode::Brush {
            return self.blind_brush_down(event);
        }
        if self.key_locker_assignment.is_some() {
            if event.button() == 0
                && !event.shift_key()
                && !event.ctrl_key()
                && !event.meta_key()
                && !event.alt_key()
            {
                return self.complete_key_locker_assignment(ctx, event);
            }
            self.key_locker_assignment = None;
        }
        let Some(point) = self.point_from_pointer(&event) else {
            return false;
        };
        self.capture_pointer(event.pointer_id());
        if self.mode == Mode::Select {
            let duplicate_drag = event.ctrl_key() && event.shift_key() && !event.meta_key();
            let additive =
                !duplicate_drag && (event.shift_key() || event.ctrl_key() || event.meta_key());
            if let Some(entity) = self.model.timeline().snapshot().entity_at(point) {
                let entity_id = entity.id().clone();
                if duplicate_drag
                    && !matches!(&self.selection, Some(Selection::Entities(entity_ids)) if entity_ids.contains(&entity_id))
                {
                    self.select_entities_and_focus_inspector(vec![entity_id.clone()]);
                } else if !duplicate_drag {
                    let selection = select_entity(self.selection.as_ref(), entity_id, additive);
                    if let Some(Selection::Entities(entity_ids)) = selection {
                        self.select_entities_and_focus_inspector(entity_ids);
                    } else {
                        self.selection = selection;
                    }
                }
                if event.button() == 0 && (!additive || duplicate_drag) && self.can_edit_timeline {
                    let origins = self
                        .selected_entity_ids()
                        .iter()
                        .filter_map(|entity_id| {
                            self.model
                                .timeline()
                                .snapshot()
                                .entity(entity_id)
                                .map(|entity| (entity_id.clone(), entity.origin()))
                        })
                        .collect::<Vec<_>>();
                    if !origins.is_empty()
                        && origins
                            .iter()
                            .all(|(entity_id, _)| !self.pending_entities.contains_key(entity_id))
                    {
                        let Some((start_canvas_x, start_canvas_y)) = self.canvas_position(&event)
                        else {
                            return true;
                        };
                        self.entity_drag = Some(EntityDrag {
                            pointer_id: event.pointer_id(),
                            start: point,
                            current: point,
                            origins,
                            duplicate: duplicate_drag,
                            rotation_steps: 0,
                            start_canvas_x,
                            start_canvas_y,
                            active: false,
                            pointer_on_grid: true,
                        });
                    }
                }
            } else if event.button() == 0 {
                let Some((canvas_x, canvas_y)) = self.canvas_position(&event) else {
                    return false;
                };
                if !additive {
                    self.selection = None;
                }
                self.marquee_gesture = Some(MarqueeGesture {
                    pointer_id: event.pointer_id(),
                    start: point,
                    current: point,
                    start_canvas_x: canvas_x,
                    start_canvas_y: canvas_y,
                    current_canvas_x: canvas_x,
                    current_canvas_y: canvas_y,
                    additive,
                    active: false,
                });
            }
        } else {
            self.select_and_focus_inspector(Selection::Cell(point));
        }
        self.last_drag = Some(point);
        self.canvas_dirty = true;

        if self.mode != Mode::Map || self.workspace_tool != WorkspaceTool::Cells {
            self.drag_kind = None;
            return true;
        }

        let Some(current) = self.effective_cell(point) else {
            return true;
        };
        let kind = self.map_paint_kind.unwrap_or(match current {
            CellKind::Floor => CellKind::Wall,
            CellKind::Wall => CellKind::Floor,
        });
        self.drag_kind = Some(kind);
        self.apply_cell_stamp(ctx, point, self.selected_shape_template(), kind);
        true
    }

    fn canvas_move(&mut self, ctx: &Context<Self>, event: PointerEvent) -> bool {
        if self.image_studio.apply.is_some() { return self.image_pointer_move(event); }
        let cursor = self.point_from_pointer(&event);
        self.queue_cursor(ctx, cursor);
        if let Some(pan) = self.pan_gesture.as_ref() {
            if pan.pointer_id != event.pointer_id() {
                return false;
            }
            let Some((x, y)) = self.canvas_position(&event) else {
                return false;
            };
            self.viewport.offset_x = pan.offset_x + x - pan.start_x;
            self.viewport.offset_y = pan.offset_y + y - pan.start_y;
            self.canvas_dirty = true;
            return true;
        }
        if let Some(gesture) = self.map_resize_gesture.as_ref() {
            if gesture.pointer_id != event.pointer_id() {
                return false;
            }
            let Some((x, y)) = self.canvas_position(&event) else {
                return false;
            };
            let outward_pixels = match gesture.edge {
                MapResizeEdge::Top => gesture.start_y - y,
                MapResizeEdge::Right => x - gesture.start_x,
                MapResizeEdge::Bottom => y - gesture.start_y,
                MapResizeEdge::Left => gesture.start_x - x,
            };
            let outward_cells = (outward_pixels / gesture.cell_size).round() as i32;
            let (preview_size, anchor) = plan_content_aware_edge_resize(
                self.model.timeline().snapshot(),
                gesture.edge,
                outward_cells,
            );
            if preview_size == gesture.preview_size {
                return false;
            }
            let gesture = self
                .map_resize_gesture
                .as_mut()
                .expect("resize gesture exists");
            gesture.preview_size = preview_size;
            gesture.anchor = anchor;
            self.canvas_dirty = true;
            return true;
        }
        if let Some(drag) = self.entity_drag.as_ref() {
            if drag.pointer_id != event.pointer_id() {
                return false;
            }
            let Some((canvas_x, canvas_y)) = self.canvas_position(&event) else {
                return false;
            };
            let active = drag.active
                || (canvas_x - drag.start_canvas_x).hypot(canvas_y - drag.start_canvas_y) >= 4.0;
            if !active {
                return false;
            }
            let point = self.point_from_canvas_position(canvas_x, canvas_y);
            if point.is_none() {
                if drag.active && !drag.pointer_on_grid {
                    return false;
                }
                let drag = self.entity_drag.as_mut().expect("drag exists");
                drag.active = true;
                drag.pointer_on_grid = false;
                self.canvas_dirty = true;
                return true;
            }
            let point = point.expect("point was checked");
            if drag.current == point && drag.active && drag.pointer_on_grid {
                return false;
            }
            let drag = self.entity_drag.as_mut().expect("drag exists");
            drag.current = point;
            drag.active = true;
            drag.pointer_on_grid = true;
            self.canvas_dirty = true;
            return true;
        }
        if let Some(gesture) = self.marquee_gesture.as_ref() {
            if gesture.pointer_id != event.pointer_id() {
                return false;
            }
            let Some((canvas_x, canvas_y)) = self.canvas_position(&event) else {
                return false;
            };
            let active = gesture.active
                || (canvas_x - gesture.start_canvas_x).hypot(canvas_y - gesture.start_canvas_y)
                    >= 4.0;
            let current = self
                .point_from_canvas_position(canvas_x, canvas_y)
                .unwrap_or(gesture.current);
            if gesture.active == active
                && gesture.current == current
                && gesture.current_canvas_x == canvas_x
                && gesture.current_canvas_y == canvas_y
            {
                return false;
            }
            let gesture = self.marquee_gesture.as_mut().expect("marquee exists");
            gesture.current = current;
            gesture.current_canvas_x = canvas_x;
            gesture.current_canvas_y = canvas_y;
            gesture.active = active;
            self.canvas_dirty = true;
            return true;
        }
        if self.blind_gesture.is_some() {
            return self.blind_brush_move(event);
        }
        let Some(kind) = self.drag_kind else {
            return false;
        };
        let Some(point) = self.point_from_pointer(&event) else {
            return false;
        };
        if self.last_drag == Some(point) {
            return false;
        }
        self.last_drag = Some(point);
        self.select_and_focus_inspector(Selection::Cell(point));
        self.apply_cell_stamp(ctx, point, self.selected_shape_template(), kind);
        true
    }

    fn canvas_up(&mut self, ctx: &Context<Self>, event: PointerEvent) -> bool {
        if self.image_studio.apply.is_some() { return self.image_pointer_up(event, false); }
        if self
            .pan_gesture
            .as_ref()
            .is_some_and(|pan| pan.pointer_id == event.pointer_id())
        {
            self.release_pointer(event.pointer_id());
            self.pan_gesture = None;
            return true;
        }
        if self
            .map_resize_gesture
            .as_ref()
            .is_some_and(|gesture| gesture.pointer_id == event.pointer_id())
        {
            self.canvas_move(ctx, event.clone());
            self.release_pointer(event.pointer_id());
            let gesture = self
                .map_resize_gesture
                .take()
                .expect("resize gesture exists");
            self.canvas_dirty = true;
            if gesture.preview_size == gesture.start_size {
                return true;
            }
            return self.submit_resize_grid(
                ctx,
                gesture.preview_size,
                gesture.anchor,
                Some(gesture.edge),
            );
        }
        if self
            .entity_drag
            .as_ref()
            .is_some_and(|drag| drag.pointer_id == event.pointer_id())
        {
            self.canvas_move(ctx, event.clone());
            self.release_pointer(event.pointer_id());
            let drag = self.entity_drag.take().expect("drag exists");
            self.canvas_dirty = true;
            if !drag.active {
                return true;
            }
            let delta_x = i32::from(drag.current.x) - i32::from(drag.start.x);
            let delta_y = i32::from(drag.current.y) - i32::from(drag.start.y);
            if delta_x == 0 && delta_y == 0 && drag.rotation_steps == 0 {
                return true;
            }
            if !self.entity_drag_preview(&drag).valid {
                self.push_toast(
                    "Element cannot snap to occupied, blocked, or out-of-bounds cells".to_owned(),
                    "warning",
                );
                return true;
            }
            if drag.duplicate {
                return self.duplicate_entities_from_drag(ctx, drag);
            }
            if drag.rotation_steps != 0 {
                return self.transform_entities_from_drag(ctx, drag);
            }
            return self.move_entities_from_origins(ctx, drag.origins, delta_x, delta_y);
        }
        if self
            .marquee_gesture
            .as_ref()
            .is_some_and(|gesture| gesture.pointer_id == event.pointer_id())
        {
            self.canvas_move(ctx, event.clone());
            self.release_pointer(event.pointer_id());
            let gesture = self.marquee_gesture.take().expect("marquee exists");
            if gesture.active {
                let mut entity_ids = if gesture.additive {
                    self.selected_entity_ids().to_vec()
                } else {
                    Vec::new()
                };
                for entity_id in entity_ids_in_rect(
                    self.model.timeline().snapshot(),
                    gesture.start,
                    gesture.current,
                ) {
                    if !entity_ids.contains(&entity_id) {
                        entity_ids.push(entity_id);
                    }
                }
                if !entity_ids.is_empty() {
                    if self.mode == Mode::Distribution {
                        self.selection = Some(Selection::Entities(entity_ids));
                        self.sync_distribution_selection();
                    } else {
                        self.select_entities_and_focus_inspector(entity_ids);
                    }
                } else if !gesture.additive {
                    self.selection = None;
                }
            } else if !gesture.additive {
                if self.mode == Mode::Distribution { self.selection = None; }
                else { self.select_and_focus_inspector(Selection::Cell(gesture.current)); }
            }
            if self.mode == Mode::Distribution { self.sync_distribution_selection(); }
            self.canvas_dirty = true;
            return true;
        }
        if self.blind_gesture.is_some() {
            return self.finish_blind_gesture(ctx, event);
        }
        self.release_pointer(event.pointer_id());
        self.drag_kind = None;
        self.last_drag = None;
        false
    }

    fn canvas_cancel(&mut self, event: PointerEvent) -> bool {
        if self.image_studio.apply.is_some() { return self.image_pointer_up(event, true); }
        self.release_pointer(event.pointer_id());
        self.drag_kind = None;
        self.last_drag = None;
        let changed = self.blind_gesture.take().is_some()
            | self.entity_drag.take().is_some()
            | self.pan_gesture.take().is_some()
            | self.map_resize_gesture.take().is_some()
            | self.marquee_gesture.take().is_some();
        self.canvas_dirty |= changed;
        changed
    }

    fn blind_brush_down(&mut self, event: PointerEvent) -> bool {
        if let Some(gesture) = self.blind_gesture.as_ref() {
            if matches!(gesture.operation, BlindGestureOperation::FloodFill { .. })
                && event.button() == 2
            {
                let pointer_id = gesture.pointer_id;
                self.release_pointer(pointer_id);
                self.blind_gesture = None;
                self.canvas_dirty = true;
                return true;
            }
            return false;
        }
        let Some(entity_id) = self.selected_entity_id().cloned() else {
            self.push_toast("Select a Pool before using Brush mode".to_owned(), "info");
            return true;
        };
        let Some(entity) = self.model.timeline().snapshot().entity(&entity_id).cloned() else {
            self.selection = None;
            return true;
        };
        let Some(blind) = entity.as_blind() else {
            self.push_toast("Brush mode requires a selected Pool".to_owned(), "info");
            return true;
        };
        if !self.can_edit_timeline {
            self.push_toast(
                "Editing requires the exact edit_timeline capability".to_owned(),
                "warning",
            );
            return true;
        }
        if !matches!(self.rpc_state, RpcState::Online) {
            self.push_toast(
                "Wait for collaboration sync before editing".to_owned(),
                "info",
            );
            return true;
        }
        if self.pending_entities.contains_key(&entity_id) {
            return false;
        }
        let Some(pixel) = self.blind_pixel_from_pointer(&event, &entity) else {
            return true;
        };

        let operation = match (self.blind_brush_tool, event.button(), event.shift_key()) {
            (BlindBrushTool::Paint, 0, true) => BlindGestureOperation::Erase,
            (BlindBrushTool::Paint, 0, false) => BlindGestureOperation::Paint {
                color_index: self.blind_color_index,
            },
            (BlindBrushTool::FloodFill, 0, _) => {
                let pixels: BTreeSet<_> = blind
                    .fill_region(entity.shape(), pixel)
                    .into_iter()
                    .collect();
                if pixels.is_empty() {
                    return true;
                }
                self.capture_pointer(event.pointer_id());
                self.blind_gesture = Some(BlindGesture {
                    pointer_id: event.pointer_id(),
                    entity_id,
                    operation: BlindGestureOperation::FloodFill {
                        start: pixel,
                        color_index: self.blind_color_index,
                    },
                    partition: pixels.clone(),
                    pixels,
                    last_pixel: None,
                });
                self.canvas_dirty = true;
                return true;
            }
            _ => return true,
        };

        self.capture_pointer(event.pointer_id());
        let partition = blind
            .paintable_partition(entity.shape(), pixel)
            .into_iter()
            .collect();
        self.blind_gesture = Some(BlindGesture {
            pointer_id: event.pointer_id(),
            entity_id,
            operation,
            pixels: BTreeSet::from([pixel]),
            partition,
            last_pixel: Some(pixel),
        });
        self.canvas_dirty = true;
        true
    }

    fn blind_brush_move(&mut self, event: PointerEvent) -> bool {
        let Some(gesture) = self.blind_gesture.as_ref() else {
            return false;
        };
        if gesture.pointer_id != event.pointer_id()
            || matches!(gesture.operation, BlindGestureOperation::FloodFill { .. })
        {
            return false;
        }
        let entity_id = gesture.entity_id.clone();
        let last_pixel = gesture.last_pixel;
        let Some(entity) = self.model.timeline().snapshot().entity(&entity_id) else {
            self.blind_gesture = None;
            self.selection = None;
            self.canvas_dirty = true;
            return true;
        };
        let Some(pixels_per_cell) = entity.as_blind().map(|blind| blind.pixels_per_cell()) else {
            self.blind_gesture = None;
            return true;
        };
        let Some(pixel) = self.blind_pixel_from_pointer(&event, entity) else {
            if let Some(gesture) = self.blind_gesture.as_mut() {
                gesture.last_pixel = None;
            }
            return false;
        };
        if last_pixel == Some(pixel) {
            return false;
        }

        let segment = last_pixel.map_or_else(
            || vec![pixel],
            |last| rasterize_blind_segment_in_tile(last, pixel, pixels_per_cell),
        );
        let continuous = segment
            .iter()
            .all(|sample| gesture.partition.contains(sample));
        let samples = if continuous { segment } else { vec![pixel] };
        let Some(gesture) = self.blind_gesture.as_mut() else {
            return false;
        };
        gesture.pixels.extend(
            samples
                .into_iter()
                .filter(|sample| gesture.partition.contains(sample)),
        );
        gesture.last_pixel = Some(pixel);
        self.canvas_dirty = true;
        true
    }

    fn finish_blind_gesture(&mut self, ctx: &Context<Self>, event: PointerEvent) -> bool {
        let Some(gesture) = self.blind_gesture.as_ref() else {
            return false;
        };
        if gesture.pointer_id != event.pointer_id() {
            return false;
        }
        if !matches!(gesture.operation, BlindGestureOperation::FloodFill { .. }) {
            self.blind_brush_move(event.clone());
        }
        self.release_pointer(event.pointer_id());
        let Some(gesture) = self.blind_gesture.take() else {
            return false;
        };
        self.canvas_dirty = true;

        let entity_id = gesture.entity_id;
        let envelope = match gesture.operation {
            BlindGestureOperation::Paint { color_index } => {
                let stroke = BlindStroke::new(gesture.pixels.into_iter().collect())
                    .expect("a browser stroke stays within the core pixel bound");
                self.model.prepare_paint_blind_stroke(
                    entity_id.clone(),
                    color_index,
                    stroke,
                    now_ms(),
                )
            }
            BlindGestureOperation::Erase => {
                let stroke = BlindStroke::new(gesture.pixels.into_iter().collect())
                    .expect("a browser stroke stays within the core pixel bound");
                self.model
                    .prepare_erase_blind_stroke(entity_id.clone(), stroke, now_ms())
            }
            BlindGestureOperation::FloodFill { start, color_index } => self
                .model
                .prepare_flood_fill_blind(entity_id.clone(), start, color_index, now_ms()),
        };
        self.submit_entity_command(ctx, envelope, [entity_id], Vec::new())
    }

    fn capture_pointer(&self, pointer_id: i32) {
        if let Some(canvas) = self.canvas_ref.cast::<HtmlCanvasElement>() {
            let _ = canvas.set_pointer_capture(pointer_id);
        }
    }

    fn release_pointer(&self, pointer_id: i32) {
        if let Some(canvas) = self.canvas_ref.cast::<HtmlCanvasElement>() {
            let _ = canvas.release_pointer_capture(pointer_id);
        }
    }

    fn point_from_pointer(&self, event: &PointerEvent) -> Option<GridPoint> {
        let (x, y) = self.canvas_position(event)?;
        self.point_from_canvas_position(x, y)
    }

    fn canvas_position(&self, event: &PointerEvent) -> Option<(f64, f64)> {
        self.canvas_position_from_client(event.client_x(), event.client_y())
    }

    fn canvas_position_from_client(&self, client_x: i32, client_y: i32) -> Option<(f64, f64)> {
        let canvas = self.canvas_ref.cast::<HtmlCanvasElement>()?;
        let bounds = canvas.get_bounding_client_rect();
        if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
            return None;
        }
        let x = (f64::from(client_x) - bounds.left()) * f64::from(canvas.width()) / bounds.width();
        let y = (f64::from(client_y) - bounds.top()) * f64::from(canvas.height()) / bounds.height();
        Some((x, y))
    }

    fn point_from_canvas_position(&self, x: f64, y: f64) -> Option<GridPoint> {
        self.viewport
            .point_at(x, y, self.model.timeline().snapshot().size(), CELL_SIZE)
    }

    fn map_resize_edge_at(&self, x: f64, y: f64) -> Option<MapResizeEdge> {
        const HANDLE_RADIUS: f64 = 13.0;
        [
            MapResizeEdge::Top,
            MapResizeEdge::Right,
            MapResizeEdge::Bottom,
            MapResizeEdge::Left,
        ]
        .into_iter()
        .find(|edge| {
            let (handle_x, handle_y) =
                self.map_resize_handle_position(*edge, self.model.timeline().snapshot().size());
            (x - handle_x).abs() <= HANDLE_RADIUS && (y - handle_y).abs() <= HANDLE_RADIUS
        })
    }

    fn map_resize_handle_position(&self, edge: MapResizeEdge, size: GridSize) -> (f64, f64) {
        let width = f64::from(size.width()) * CELL_SIZE * self.viewport.scale;
        let height = f64::from(size.height()) * CELL_SIZE * self.viewport.scale;
        let left = self.viewport.offset_x;
        let top = self.viewport.offset_y;
        match edge {
            MapResizeEdge::Top => (left + width / 2.0, top),
            MapResizeEdge::Right => (left + width, top + height / 2.0),
            MapResizeEdge::Bottom => (left + width / 2.0, top + height),
            MapResizeEdge::Left => (left, top + height / 2.0),
        }
    }

    fn blind_pixel_from_pointer(
        &self,
        event: &PointerEvent,
        entity: &PlaceableEntity,
    ) -> Option<BlindPixel> {
        let blind = entity.as_blind()?;
        let (x, y) = self.canvas_position(event)?;
        let world_cell = self.point_from_canvas_position(x, y)?;
        let (cell_left, cell_top) = self.viewport.cell_origin(world_cell, CELL_SIZE);
        let cell_x = u8::try_from(world_cell.x.checked_sub(entity.origin().x)?).ok()?;
        let cell_y = u8::try_from(world_cell.y.checked_sub(entity.origin().y)?).ok()?;
        let cell = ShapeCell::new(cell_x, cell_y);
        blind.tile_for_cell(entity.shape(), cell)?;
        let geometry = pool_tile_geometry(
            shape_boundary_edges(entity.shape(), cell),
            CELL_SIZE * self.viewport.scale,
            BLIND_INSET * self.viewport.scale,
        );
        let resolution = blind.pixels_per_cell();
        let (pixel_x, pixel_y_from_top) =
            geometry.pixel_at(x - cell_left, y - cell_top, resolution)?;
        blind_pixel_from_top_left_sample(entity, world_cell, pixel_x, pixel_y_from_top)
    }

    fn apply_cell_stamp(
        &mut self,
        ctx: &Context<Self>,
        origin: GridPoint,
        shape: Shape,
        kind: CellKind,
    ) -> bool {
        if !self.can_edit_timeline {
            self.push_toast(
                "Editing requires the exact edit_timeline capability".to_owned(),
                "warning",
            );
            return true;
        }
        if !matches!(self.rpc_state, RpcState::Online) {
            self.push_toast(
                "Wait for collaboration sync before editing".to_owned(),
                "info",
            );
            return true;
        }

        let size = self.model.timeline().snapshot().size();
        let Some(points) = shape_world_points(origin, shape) else {
            return false;
        };
        let points = points
            .into_iter()
            .filter(|point| point.x < size.width() && point.y < size.height())
            .filter(|point| !self.pending_cells.contains_key(point))
            .filter(|point| self.effective_cell(*point) != Some(kind))
            .collect::<Vec<_>>();
        if points.is_empty() {
            return false;
        }
        if kind == CellKind::Wall
            && let Some(point) = points
                .iter()
                .copied()
                .find(|point| self.model.timeline().snapshot().entity_at(*point).is_some())
        {
            self.push_toast(
                format!(
                    "The brush footprint overlaps an entity at ({}, {})",
                    point.x, point.y
                ),
                "warning",
            );
            return true;
        }

        let Some(rpc) = self.rpc.clone() else {
            return false;
        };
        let edits = points
            .iter()
            .copied()
            .map(|point| CellEdit::new(point, kind))
            .collect();
        let command = self.model.prepare_set_cells(edits, now_ms());
        let command_id = command.metadata.id.to_string();
        self.pending_commands.insert(command_id.clone());
        for point in points {
            self.pending_cells.insert(
                point,
                PendingCell {
                    kind,
                    command_id: command_id.clone(),
                },
            );
        }
        self.canvas_dirty = true;

        let request = ApplyCommandRequest {
            target: self.target.clone(),
            command,
        };
        let link = ctx.link().clone();
        let attempt = self.connection_attempt;
        wasm_bindgen_futures::spawn_local(async move {
            link.send_message(Msg::RpcApplyFinished {
                attempt,
                command_id,
                result: Box::new(rpc.apply_command(request).await),
            });
        });
        true
    }

    fn resize_grid(&mut self, ctx: &Context<Self>) -> bool {
        if !self.can_edit_timeline {
            self.push_toast(
                "Editing requires the exact edit_timeline capability".to_owned(),
                "warning",
            );
            return true;
        }
        if !matches!(self.rpc_state, RpcState::Online)
            || self.pending_grid.is_some()
            || self.blind_gesture.is_some()
            || self.entity_drag.is_some()
            || self.drag_kind.is_some()
        {
            return false;
        }
        let (Ok(width), Ok(height)) = (
            self.resize_width.parse::<u16>(),
            self.resize_height.parse::<u16>(),
        ) else {
            self.push_toast(
                "Grid dimensions must be whole numbers".to_owned(),
                "warning",
            );
            return true;
        };
        let Ok(size) = GridSize::new(width, height) else {
            self.push_toast(
                "Grid dimensions must each be between 1 and 256".to_owned(),
                "warning",
            );
            return true;
        };
        self.submit_resize_grid(ctx, size, visual_anchor_to_grid(self.resize_anchor), None)
    }

    fn submit_resize_grid(
        &mut self,
        ctx: &Context<Self>,
        size: GridSize,
        anchor: GridAnchor,
        edge_view: Option<MapResizeEdge>,
    ) -> bool {
        if !matches!(self.rpc_state, RpcState::Online)
            || self.pending_grid.is_some()
            || size == self.model.timeline().snapshot().size()
        {
            return false;
        }
        let Some(rpc) = self.rpc.clone() else {
            return false;
        };
        let envelope = self.model.prepare_resize_grid(size, anchor, now_ms());
        let command_id = envelope.metadata.id.to_string();
        self.pending_commands.insert(command_id.clone());
        self.pending_grid = Some(command_id.clone());
        self.pending_edge_view = edge_view;
        let request = ApplyCommandRequest {
            target: self.target.clone(),
            command: envelope,
        };
        let link = ctx.link().clone();
        let attempt = self.connection_attempt;
        wasm_bindgen_futures::spawn_local(async move {
            link.send_message(Msg::RpcApplyFinished {
                attempt,
                command_id,
                result: Box::new(rpc.apply_command(request).await),
            });
        });
        true
    }

    fn place_shape_at(
        &mut self,
        ctx: &Context<Self>,
        kind: PlacementKind,
        shape: Shape,
        point: GridPoint,
    ) -> bool {
        if self.mode != Mode::Select {
            return false;
        }
        if !self.can_edit_timeline {
            self.push_toast(
                "Editing requires the exact edit_timeline capability".to_owned(),
                "warning",
            );
            return true;
        }
        if !matches!(self.rpc_state, RpcState::Online) {
            self.push_toast(
                "Wait for collaboration sync before editing".to_owned(),
                "info",
            );
            return true;
        }
        let Some(placement_points) = shape_world_points(point, shape) else {
            self.push_toast(
                "Template footprint exceeds the level bounds".to_owned(),
                "warning",
            );
            return true;
        };
        if !self.can_place_shape(point, shape) {
            self.push_toast(
                "The complete template footprint requires unoccupied floor cells".to_owned(),
                "warning",
            );
            return true;
        }

        let single_cell = Shape::new(1, 1, 1).expect("single-cell template is valid");
        let envelope = match (kind, shape == single_cell) {
            (PlacementKind::Block, true) => self.model.prepare_place_default_block(point, now_ms()),
            (PlacementKind::Blind, true) => self.model.prepare_place_default_blind(point, now_ms()),
            (PlacementKind::Block, false) => self.model.prepare_place_block(point, shape, now_ms()),
            (PlacementKind::Blind, false) => self.model.prepare_place_blind(point, shape, now_ms()),
        };
        let LevelCommand::PlaceEntity { entity } = &envelope.command else {
            unreachable!("default placement preparation returns PlaceEntity");
        };
        let entity_id = entity.id().clone();
        self.select_and_focus_inspector(Selection::Cell(point));
        self.submit_entity_command(ctx, envelope, [entity_id], placement_points)
    }

    fn can_place_shape(&self, point: GridPoint, shape: Shape) -> bool {
        let size = self.model.timeline().snapshot().size();
        shape_world_points(point, shape).is_some_and(|points| {
            points.into_iter().all(|sample| {
                sample.x < size.width()
                    && sample.y < size.height()
                    && self.effective_cell(sample) == Some(CellKind::Floor)
                    && self.model.timeline().snapshot().entity_at(sample).is_none()
                    && !self.pending_placements.contains_key(&sample)
            })
        })
    }

    fn selected_shape_template(&self) -> Shape {
        self.active_shape_template_id
            .as_ref()
            .and_then(|shape_id| {
                self.shape_catalog
                    .iter()
                    .find(|entry| &entry.id == shape_id)
            })
            .and_then(|entry| {
                Shape::new(
                    entry.shape.width,
                    entry.shape.height,
                    entry.shape.occupied_mask,
                )
                .ok()
            })
            .unwrap_or_else(|| Shape::new(1, 1, 1).expect("single-cell template is valid"))
    }

    fn select_shape_draft(&mut self, shape_id: &str) -> bool {
        let Some(entry) = self.shape_catalog.iter().find(|entry| entry.id == shape_id) else {
            return false;
        };
        let Ok(shape) = Shape::new(
            entry.shape.width,
            entry.shape.height,
            entry.shape.occupied_mask,
        ) else {
            self.push_toast("Project shape is invalid".to_owned(), "warning");
            return true;
        };
        self.selected_shape_id = Some(entry.id.clone());
        self.shape_draft_name = entry.name.clone();
        self.shape_draft_mask = designer_mask_from_shape(shape);
        true
    }

    fn save_shape_draft(&mut self, ctx: &Context<Self>, update: bool) -> bool {
        if self.shape_catalog_pending || !self.can_edit_timeline {
            return false;
        }
        let name = self.shape_draft_name.trim().to_owned();
        if name.is_empty() {
            self.push_toast("Name the shape before saving".to_owned(), "info");
            return true;
        }
        let Ok(shape) = shape_from_designer_mask(self.shape_draft_mask) else {
            self.push_toast(
                "Shape must be non-empty and connected by cell edges".to_owned(),
                "warning",
            );
            return true;
        };
        let definition = ShapeDefinition {
            width: shape.width(),
            height: shape.height(),
            occupied_mask: shape.occupied_mask(),
        };
        let project_id = self.target.project_id.to_string();
        let response_project_id = project_id.clone();
        let selected_shape_id = self.selected_shape_id.clone();
        if update && selected_shape_id.is_none() {
            return false;
        }
        self.shape_catalog_pending = true;
        let link = ctx.link().clone();
        wasm_bindgen_futures::spawn_local(async move {
            let result = if let Some(shape_id) = selected_shape_id.filter(|_| update) {
                RestClient
                    .update_shape_catalog_entry(&project_id, &shape_id, &name, definition)
                    .await
            } else {
                RestClient
                    .create_shape_catalog_entry(&project_id, &name, definition)
                    .await
            };
            link.send_message(Msg::ShapeSaved {
                project_id: response_project_id,
                result,
            });
        });
        true
    }

    fn delete_shape_draft(&mut self, ctx: &Context<Self>) -> bool {
        if self.shape_catalog_pending || !self.can_edit_timeline {
            return false;
        }
        let Some(shape_id) = self.selected_shape_id.clone() else {
            return false;
        };
        let project_id = self.target.project_id.to_string();
        let response_project_id = project_id.clone();
        let response_shape_id = shape_id.clone();
        self.shape_catalog_pending = true;
        let link = ctx.link().clone();
        wasm_bindgen_futures::spawn_local(async move {
            link.send_message(Msg::ShapeDeleted {
                project_id: response_project_id,
                shape_id: response_shape_id,
                result: RestClient
                    .delete_shape_catalog_entry(&project_id, &shape_id)
                    .await,
            });
        });
        true
    }

    fn move_selected_entity_with_keyboard(
        &mut self,
        ctx: &Context<Self>,
        direction: MoveDirection,
    ) -> bool {
        if self.mode != Mode::Select {
            return false;
        }
        let (delta_x, delta_y) = direction.offset();
        self.move_selected_entities(ctx, i32::from(delta_x), i32::from(delta_y))
    }

    fn edit_selected_entity(&mut self, ctx: &Context<Self>, action: EntityAction) -> bool {
        if self.mode != Mode::Select {
            return false;
        }
        let entity_ids = self.selected_entity_ids().to_vec();
        if entity_ids.is_empty() {
            return false;
        }
        if !self.can_edit_timeline {
            self.push_toast(
                "Editing requires the exact edit_timeline capability".to_owned(),
                "warning",
            );
            return true;
        }
        if !matches!(self.rpc_state, RpcState::Online) {
            self.push_toast(
                "Wait for collaboration sync before editing".to_owned(),
                "info",
            );
            return true;
        }
        if entity_ids
            .iter()
            .any(|entity_id| self.pending_entities.contains_key(entity_id))
        {
            return false;
        }
        let entities = entity_ids
            .iter()
            .filter_map(|entity_id| self.model.timeline().snapshot().entity(entity_id).cloned())
            .collect::<Vec<_>>();
        if entities.len() != entity_ids.len() {
            self.selection = None;
            return true;
        }

        let (envelope, placement_points) = match action {
            EntityAction::Delete => (
                self.model
                    .prepare_delete_entities(entity_ids.clone(), now_ms()),
                Vec::new(),
            ),
            EntityAction::RotateClockwise | EntityAction::FlipHorizontal => {
                let transforms = entities
                    .iter()
                    .map(|entity| match action {
                        EntityAction::RotateClockwise => {
                            EntityTransform::new(entity.id().clone()).rotated_clockwise(1)
                        }
                        EntityAction::FlipHorizontal => {
                            EntityTransform::new(entity.id().clone()).flipped_horizontal()
                        }
                        EntityAction::Delete => unreachable!(),
                    })
                    .collect::<Vec<_>>();
                let transformed = entities
                    .into_iter()
                    .map(|entity| match action {
                        EntityAction::RotateClockwise => entity.rotated_clockwise(),
                        EntityAction::FlipHorizontal => entity.flipped_horizontal(),
                        EntityAction::Delete => unreachable!(),
                    })
                    .collect::<Vec<_>>();
                let placement_points = transformed
                    .iter()
                    .flat_map(|entity| {
                        shape_world_points(entity.origin(), entity.shape()).unwrap_or_default()
                    })
                    .collect();
                (
                    self.model.prepare_transform_entities(transforms, now_ms()),
                    placement_points,
                )
            }
        };
        self.submit_entity_command(ctx, envelope, entity_ids, placement_points)
    }

    fn set_pool_resolution(&mut self, ctx: &Context<Self>, pixels_per_cell: u8) -> bool {
        if !(1..=32).contains(&pixels_per_cell) {
            self.push_toast(
                "Pool resolution must be between 1 and 32".to_owned(),
                "warning",
            );
            return true;
        }
        let entity_ids = self.selected_entity_ids().to_vec();
        if entity_ids.is_empty() || self.mode != Mode::Select {
            return false;
        }
        if !self.can_edit_timeline || !matches!(self.rpc_state, RpcState::Online) {
            self.push_toast(
                "Wait for collaboration sync before editing".to_owned(),
                "info",
            );
            return true;
        }
        if entity_ids
            .iter()
            .any(|entity_id| self.pending_entities.contains_key(entity_id))
        {
            return false;
        }
        let entities = entity_ids
            .iter()
            .filter_map(|entity_id| self.model.timeline().snapshot().entity(entity_id).cloned())
            .collect::<Vec<_>>();
        if entities.len() != entity_ids.len()
            || entities.iter().any(|entity| entity.as_blind().is_none())
        {
            return false;
        }
        if entities.iter().all(|entity| {
            entity
                .as_blind()
                .is_some_and(|blind| blind.pixels_per_cell() == pixels_per_cell)
        }) {
            return false;
        }
        let placement_points = entities
            .iter()
            .flat_map(|entity| {
                shape_world_points(entity.origin(), entity.shape()).unwrap_or_default()
            })
            .collect();
        let envelope =
            self.model
                .prepare_set_blind_resolutions(entity_ids.clone(), pixels_per_cell, now_ms());
        self.submit_entity_command(ctx, envelope, entity_ids, placement_points)
    }

    fn set_block_layer_capacity(
        &mut self,
        ctx: &Context<Self>,
        layer_index: usize,
        value: &str,
    ) -> bool {
        let value = value.trim();
        let capacity = if value.eq_ignore_ascii_case("unlimited") {
            CollectCapacity::Unlimited
        } else {
            let Ok(capacity) = value.parse::<u32>() else {
                self.push_toast(
                    "Capacity must be a non-negative number or 'unlimited'".to_owned(),
                    "warning",
                );
                return true;
            };
            CollectCapacity::Finite(capacity)
        };
        let entity_ids = self.selected_entity_ids().to_vec();
        if entity_ids.is_empty()
            || self.mode != Mode::Select
            || !self.can_edit_timeline
            || !matches!(self.rpc_state, RpcState::Online)
            || entity_ids
                .iter()
                .any(|entity_id| self.pending_entities.contains_key(entity_id))
        {
            return false;
        }
        let all_editable = entity_ids.iter().all(|entity_id| {
            self.model
                .timeline()
                .snapshot()
                .entity(entity_id)
                .and_then(|entity| match entity.kind() {
                    PlaceableEntityKind::Block(block) => block
                        .collect_layers()
                        .get(layer_index)
                        .map(|layer| !layer.is_locked()),
                    PlaceableEntityKind::Blind(_) => None,
                })
                .unwrap_or(false)
        });
        if !all_editable {
            self.push_toast(
                "Select Blocks with matching, unlocked capacity layers to batch-edit them"
                    .to_owned(),
                "info",
            );
            return true;
        }
        let envelope = self.model.prepare_set_block_layer_capacity(
            entity_ids.clone(),
            layer_index,
            capacity,
            now_ms(),
        );
        self.submit_entity_command(ctx, envelope, entity_ids, Vec::new())
    }

    fn add_block_collect_layer(&mut self, ctx: &Context<Self>) -> bool {
        let Some(entity_id) = self.selected_entity_id().cloned() else {
            return false;
        };
        if self.mode != Mode::Select
            || !self.can_edit_timeline
            || !matches!(self.rpc_state, RpcState::Online)
            || self.pending_entities.contains_key(&entity_id)
        {
            return false;
        }
        let Some(entity) = self.model.timeline().snapshot().entity(&entity_id) else {
            return false;
        };
        let PlaceableEntityKind::Block(block) = entity.kind() else {
            return false;
        };
        let mut layers = block.collect_layers().to_vec();
        layers.push(CollectLayer::new(
            1,
            None,
            CollectCapacity::Unlimited,
            false,
        ));
        let envelope =
            self.model
                .prepare_set_block_collect_layers(entity_id.clone(), layers, now_ms());
        self.submit_entity_command(ctx, envelope, [entity_id], Vec::new())
    }

    fn remove_block_collect_layer(&mut self, ctx: &Context<Self>, layer_index: usize) -> bool {
        let Some(entity_id) = self.selected_entity_id().cloned() else {
            return false;
        };
        if self.mode != Mode::Select
            || !self.can_edit_timeline
            || !matches!(self.rpc_state, RpcState::Online)
            || self.pending_entities.contains_key(&entity_id)
        {
            return false;
        }
        let Some(entity) = self.model.timeline().snapshot().entity(&entity_id) else {
            return false;
        };
        let PlaceableEntityKind::Block(block) = entity.kind() else {
            return false;
        };
        let mut layers = block.collect_layers().to_vec();
        let Some(layer) = layers.get(layer_index) else {
            return false;
        };
        if layer.is_locked() {
            self.push_toast(
                "Locked capacity layers cannot be removed".to_owned(),
                "info",
            );
            return true;
        }
        layers.remove(layer_index);
        let envelope =
            self.model
                .prepare_set_block_collect_layers(entity_id.clone(), layers, now_ms());
        self.submit_entity_command(ctx, envelope, [entity_id], Vec::new())
    }

    fn move_selected_entities(&mut self, ctx: &Context<Self>, delta_x: i32, delta_y: i32) -> bool {
        let origins = self
            .selected_entity_ids()
            .iter()
            .filter_map(|entity_id| {
                self.model
                    .timeline()
                    .snapshot()
                    .entity(entity_id)
                    .map(|entity| (entity_id.clone(), entity.origin()))
            })
            .collect::<Vec<_>>();
        self.move_entities_from_origins(ctx, origins, delta_x, delta_y)
    }

    fn move_entities_from_origins(
        &mut self,
        ctx: &Context<Self>,
        origins: Vec<(EntityId, GridPoint)>,
        delta_x: i32,
        delta_y: i32,
    ) -> bool {
        if origins.is_empty() || self.mode != Mode::Select {
            return false;
        }
        if !self.can_edit_timeline {
            self.push_toast(
                "Editing requires the exact edit_timeline capability".to_owned(),
                "warning",
            );
            return true;
        }
        if !matches!(self.rpc_state, RpcState::Online) {
            self.push_toast(
                "Wait for collaboration sync before editing".to_owned(),
                "info",
            );
            return true;
        }
        if origins
            .iter()
            .any(|(entity_id, _)| self.pending_entities.contains_key(entity_id))
        {
            return false;
        }
        let Ok(delta_x) = i16::try_from(delta_x) else {
            return false;
        };
        let Ok(delta_y) = i16::try_from(delta_y) else {
            return false;
        };
        let snapshot = self.model.timeline().snapshot();
        let mut moves = Vec::with_capacity(origins.len());
        let mut entity_ids = Vec::with_capacity(origins.len());
        let mut placement_points = Vec::new();
        let mut previews = Vec::with_capacity(origins.len());
        for (entity_id, origin) in origins {
            let (Some(x), Some(y)) = (
                origin.x.checked_add_signed(delta_x),
                origin.y.checked_add_signed(delta_y),
            ) else {
                self.push_toast(
                    "Move rejected: selection would leave the grid".to_owned(),
                    "warning",
                );
                return true;
            };
            let Some(entity) = snapshot.entity(&entity_id) else {
                self.selection = None;
                return true;
            };
            let destination = GridPoint::new(x, y);
            placement_points
                .extend(shape_world_points(destination, entity.shape()).unwrap_or_default());
            entity_ids.push(entity_id.clone());
            previews.push(entity.moved_to(destination));
            moves.push(EntityMove::new(entity_id, destination));
        }
        let envelope = self.model.prepare_move_entities(moves, now_ms());
        self.submit_entity_command_with_previews(
            ctx,
            envelope,
            entity_ids,
            placement_points,
            previews,
        )
    }

    fn duplicate_entities_from_drag(&mut self, ctx: &Context<Self>, drag: EntityDrag) -> bool {
        if drag.origins.is_empty() || self.mode != Mode::Select {
            return false;
        }
        if !self.can_edit_timeline {
            self.push_toast(
                "Editing requires the exact edit_timeline capability".to_owned(),
                "warning",
            );
            return true;
        }
        if !matches!(self.rpc_state, RpcState::Online) {
            self.push_toast(
                "Wait for collaboration sync before editing".to_owned(),
                "info",
            );
            return true;
        }
        if drag
            .origins
            .iter()
            .any(|(entity_id, _)| self.pending_entities.contains_key(entity_id))
        {
            return false;
        }
        let snapshot = self.model.timeline().snapshot();
        let mut duplicates = Vec::with_capacity(drag.origins.len());
        let mut placement_points = Vec::new();
        for (entity_id, origin) in &drag.origins {
            let Some(source) = snapshot.entity(entity_id) else {
                return false;
            };
            let Some(copy) = dragged_entity_preview(
                source,
                *origin,
                drag.start,
                drag.current,
                drag.rotation_steps,
            ) else {
                self.push_toast(
                    "Duplicate rejected: selection would leave the grid".to_owned(),
                    "warning",
                );
                return true;
            };
            let destination = copy.origin();
            placement_points
                .extend(shape_world_points(destination, copy.shape()).unwrap_or_default());
            duplicates.push((copy, destination));
        }
        let (envelope, duplicate_ids) = self.model.prepare_duplicate_entities(duplicates, now_ms());
        let previews = match &envelope.command {
            LevelCommand::PlaceEntities { entities } => entities.clone(),
            _ => Vec::new(),
        };
        let submitted = self.submit_entity_command_with_previews(
            ctx,
            envelope,
            duplicate_ids,
            placement_points,
            previews,
        );
        if submitted {
            self.push_toast(
                "Duplicated selection (decorators are not copied)".to_owned(),
                "info",
            );
        }
        submitted
    }

    fn transform_entities_from_drag(&mut self, ctx: &Context<Self>, drag: EntityDrag) -> bool {
        if drag.origins.is_empty() || self.mode != Mode::Select {
            return false;
        }
        if !self.can_edit_timeline {
            self.push_toast(
                "Editing requires the exact edit_timeline capability".to_owned(),
                "warning",
            );
            return true;
        }
        if !matches!(self.rpc_state, RpcState::Online) {
            self.push_toast(
                "Wait for collaboration sync before editing".to_owned(),
                "info",
            );
            return true;
        }
        if drag
            .origins
            .iter()
            .any(|(entity_id, _)| self.pending_entities.contains_key(entity_id))
        {
            return false;
        }
        let snapshot = self.model.timeline().snapshot();
        let mut transforms = Vec::with_capacity(drag.origins.len());
        let mut placement_points = Vec::new();
        let mut entity_ids = Vec::with_capacity(drag.origins.len());
        let mut previews = Vec::with_capacity(drag.origins.len());
        for (entity_id, origin) in &drag.origins {
            let Some(source) = snapshot.entity(entity_id) else {
                return false;
            };
            let Some(preview) = dragged_entity_preview(
                source,
                *origin,
                drag.start,
                drag.current,
                drag.rotation_steps,
            ) else {
                self.push_toast(
                    "Transform rejected: selection would leave the grid".to_owned(),
                    "warning",
                );
                return true;
            };
            let destination = preview.origin();
            placement_points
                .extend(shape_world_points(destination, preview.shape()).unwrap_or_default());
            entity_ids.push(entity_id.clone());
            previews.push(preview);
            transforms.push(
                EntityTransform::new(entity_id.clone())
                    .moved_to(destination)
                    .rotated_clockwise(drag.rotation_steps),
            );
        }
        let envelope = self.model.prepare_transform_entities(transforms, now_ms());
        self.submit_entity_command_with_previews(
            ctx,
            envelope,
            entity_ids,
            placement_points,
            previews,
        )
    }

    fn edit_selected_decorator(&mut self, ctx: &Context<Self>, action: DecoratorAction) -> bool {
        if self.mode != Mode::Select {
            return false;
        }
        let Some(entity_id) = self.selected_entity_id().cloned() else {
            return false;
        };
        let supports_action = self
            .model
            .timeline()
            .snapshot()
            .entity(&entity_id)
            .is_some_and(|entity| match &action {
                DecoratorAction::ToggleGlass => entity.as_blind().is_some(),
                DecoratorAction::ToggleIce
                | DecoratorAction::CycleDirection
                | DecoratorAction::SetDirection(_)
                | DecoratorAction::BeginKeyLocker => {
                    matches!(entity.kind(), PlaceableEntityKind::Block(_))
                }
            });
        if !supports_action {
            return false;
        }
        if !self.can_edit_timeline {
            self.push_toast(
                "Editing requires the exact edit_timeline capability".to_owned(),
                "warning",
            );
            return true;
        }
        if !matches!(self.rpc_state, RpcState::Online) {
            self.push_toast(
                "Wait for collaboration sync before editing".to_owned(),
                "info",
            );
            return true;
        }
        if self.pending_entities.contains_key(&entity_id) {
            return false;
        }

        if action == DecoratorAction::BeginKeyLocker {
            self.key_locker_assignment =
                (self.key_locker_assignment.as_ref() != Some(&entity_id)).then_some(entity_id);
            self.canvas_dirty = true;
            return true;
        }

        self.key_locker_assignment = None;
        let envelope = match action {
            DecoratorAction::ToggleIce => {
                self.model.prepare_toggle_ice(entity_id.clone(), now_ms())
            }
            DecoratorAction::ToggleGlass => {
                self.model.prepare_toggle_glass(entity_id.clone(), now_ms())
            }
            DecoratorAction::CycleDirection => self
                .model
                .prepare_cycle_direction(entity_id.clone(), now_ms()),
            DecoratorAction::SetDirection(mode) => {
                self.model
                    .prepare_set_direction(entity_id.clone(), mode, now_ms())
            }
            DecoratorAction::BeginKeyLocker => unreachable!(),
        };
        self.submit_decorator_command(ctx, envelope, [entity_id])
    }

    fn set_selected_ice_count(&mut self, ctx: &Context<Self>, value: &str) -> bool {
        let Ok(blocking_count) = value.parse::<u32>() else {
            return false;
        };
        if self.mode != Mode::Select
            || !self.can_edit_timeline
            || !matches!(self.rpc_state, RpcState::Online)
        {
            return false;
        }
        let Some(entity_id) = self.selected_entity_id().cloned() else {
            return false;
        };
        if self.pending_entities.contains_key(&entity_id)
            || !self
                .model
                .timeline()
                .snapshot()
                .entity(&entity_id)
                .is_some_and(|entity| matches!(entity.kind(), PlaceableEntityKind::Block(_)))
        {
            return false;
        }
        let envelope = self
            .model
            .prepare_set_ice(entity_id.clone(), blocking_count, now_ms());
        self.submit_decorator_command(ctx, envelope, [entity_id])
    }

    fn set_selected_glass_count(&mut self, ctx: &Context<Self>, value: &str) -> bool {
        let Ok(blocking_count) = value.parse::<u32>() else {
            return false;
        };
        if self.mode != Mode::Select
            || !self.can_edit_timeline
            || !matches!(self.rpc_state, RpcState::Online)
        {
            return false;
        }
        let Some(entity_id) = self.selected_entity_id().cloned() else {
            return false;
        };
        if self.pending_entities.contains_key(&entity_id)
            || self
                .model
                .timeline()
                .snapshot()
                .entity(&entity_id)
                .is_none_or(|entity| entity.as_blind().is_none())
        {
            return false;
        }
        let envelope = self
            .model
            .prepare_set_glass(entity_id.clone(), blocking_count, now_ms());
        self.submit_decorator_command(ctx, envelope, [entity_id])
    }

    fn complete_key_locker_assignment(&mut self, ctx: &Context<Self>, event: PointerEvent) -> bool {
        let Some(key_entity_id) = self.key_locker_assignment.clone() else {
            return false;
        };
        let Some(point) = self.point_from_pointer(&event) else {
            self.push_toast("Choose a Block inside the level".to_owned(), "info");
            return true;
        };
        let snapshot = self.model.timeline().snapshot();
        let Some(target) = snapshot.entity_at(point) else {
            self.push_toast("Locker target must be a Block".to_owned(), "info");
            return true;
        };
        let lock_entity_id = target.id().clone();
        if key_entity_id == lock_entity_id
            || !matches!(target.kind(), PlaceableEntityKind::Block(_))
        {
            self.push_toast("Choose a different Block as the Locker".to_owned(), "info");
            return true;
        }
        let current_relation_id =
            key_locker_for_key(snapshot, &key_entity_id).map(|decorator| decorator.id().clone());
        if key_locker_for_lock(snapshot, &lock_entity_id)
            .is_some_and(|decorator| Some(decorator.id()) != current_relation_id.as_ref())
        {
            self.push_toast(
                "That Block is already another Key's Locker".to_owned(),
                "warning",
            );
            return true;
        }
        if self.pending_entities.contains_key(&key_entity_id)
            || self.pending_entities.contains_key(&lock_entity_id)
        {
            return false;
        }

        self.key_locker_assignment = None;
        let envelope = self.model.prepare_assign_key_locker(
            key_entity_id.clone(),
            lock_entity_id.clone(),
            now_ms(),
        );
        self.submit_decorator_command(ctx, envelope, [key_entity_id, lock_entity_id])
    }

    fn submit_entity_command(
        &mut self,
        ctx: &Context<Self>,
        envelope: CommandEnvelope,
        entity_ids: impl IntoIterator<Item = EntityId>,
        placement_points: Vec<GridPoint>,
    ) -> bool {
        self.submit_entity_command_with_previews(
            ctx,
            envelope,
            entity_ids,
            placement_points,
            Vec::new(),
        )
    }

    fn submit_entity_command_with_previews(
        &mut self,
        ctx: &Context<Self>,
        envelope: CommandEnvelope,
        entity_ids: impl IntoIterator<Item = EntityId>,
        placement_points: Vec<GridPoint>,
        previews: Vec<PlaceableEntity>,
    ) -> bool {
        let Some(rpc) = self
            .rpc
            .clone()
            .filter(|_| matches!(self.rpc_state, RpcState::Online))
        else {
            return false;
        };
        let command_id = envelope.metadata.id.to_string();
        let mut previews = previews
            .into_iter()
            .map(|entity| (entity.id().clone(), entity))
            .collect::<BTreeMap<_, _>>();
        self.pending_commands.insert(command_id.clone());
        for entity_id in entity_ids {
            self.pending_entities.insert(
                entity_id.clone(),
                PendingEntity {
                    command_id: command_id.clone(),
                    preview: previews.remove(&entity_id),
                },
            );
        }
        for point in placement_points {
            self.pending_placements.insert(point, command_id.clone());
        }
        let request = ApplyCommandRequest {
            target: self.target.clone(),
            command: envelope,
        };
        let link = ctx.link().clone();
        let attempt = self.connection_attempt;
        wasm_bindgen_futures::spawn_local(async move {
            link.send_message(Msg::RpcApplyFinished {
                attempt,
                command_id,
                result: Box::new(rpc.apply_command(request).await),
            });
        });
        self.canvas_dirty = true;
        true
    }

    fn submit_decorator_command(
        &mut self,
        ctx: &Context<Self>,
        envelope: CommandEnvelope,
        entity_ids: impl IntoIterator<Item = EntityId>,
    ) -> bool {
        let Some(rpc) = self
            .rpc
            .clone()
            .filter(|_| matches!(self.rpc_state, RpcState::Online))
        else {
            return false;
        };
        let command_id = envelope.metadata.id.to_string();
        self.pending_commands.insert(command_id.clone());
        for entity_id in entity_ids {
            self.pending_entities.insert(
                entity_id,
                PendingEntity {
                    command_id: command_id.clone(),
                    preview: None,
                },
            );
        }
        let request = ApplyCommandRequest {
            target: self.target.clone(),
            command: envelope,
        };
        let link = ctx.link().clone();
        let attempt = self.connection_attempt;
        wasm_bindgen_futures::spawn_local(async move {
            link.send_message(Msg::RpcApplyFinished {
                attempt,
                command_id,
                result: Box::new(rpc.apply_command(request).await),
            });
        });
        self.canvas_dirty = true;
        true
    }

    fn undo(&mut self, ctx: &Context<Self>) -> bool {
        if self.blind_gesture.is_some() {
            self.push_toast(
                "Finish or cancel the active Brush gesture first".to_owned(),
                "info",
            );
            return true;
        }
        self.key_locker_assignment = None;
        if !self.can_edit_timeline {
            self.push_toast(
                "Undo requires the exact edit_timeline capability".to_owned(),
                "warning",
            );
            return true;
        }
        if !matches!(self.rpc_state, RpcState::Online) {
            self.push_toast("Wait for collaboration sync before undo".to_owned(), "info");
            return true;
        }
        let Some(rpc) = self.rpc.clone() else {
            return false;
        };
        let metadata = self.model.prepare_undo(now_ms());
        let command_id = metadata.id.to_string();
        self.pending_commands.insert(command_id.clone());
        let request = UndoLatestRequest {
            target: self.target.clone(),
            metadata,
        };
        let link = ctx.link().clone();
        let attempt = self.connection_attempt;
        wasm_bindgen_futures::spawn_local(async move {
            link.send_message(Msg::RpcUndoFinished {
                attempt,
                command_id,
                result: Box::new(rpc.undo_latest(request).await),
            });
        });
        self.open_menu = None;
        self.palette_open = false;
        true
    }

    fn key_down(&mut self, ctx: &Context<Self>, event: KeyboardEvent) -> bool {
        if self.distribution_modal_open() {
            if event.key() == "Escape" {
                event.prevent_default();
                return self.distribution_message(ctx, DistributionMsg::CloseDivision);
            }
            return false;
        }
        if self.image_studio.apply.is_some() {
            if event.key() == "Escape" {
                event.prevent_default();
                if self.studio_modal.is_some() { self.cancel_conversion(); self.studio_modal = None; }
                else { self.cancel_apply_image(); }
                return true;
            }
            // Inputs retain browser editing; every workspace shortcut is suspended.
            return false;
        }
        if self.explorer_action.is_some() && event.key() == "Escape" {
            event.prevent_default();
            self.explorer_action = None;
            self.form_error = None;
            self.advance_level_intent();
            return true;
        }
        if self.studio_modal.is_some() {
            if event.key() == "Escape" {
                event.prevent_default();
                self.cancel_conversion();
                self.studio_modal = None;
                return true;
            }
            // The root shell receives bubbling key events from the focused studio. Do not
            // let workspace shortcuts mutate the obscured canvas while a modal is open.
            return false;
        }
        let scope = if self.palette_open {
            ShortcutScope::Palette
        } else if event_target_is_text_entry(event.target()) {
            ShortcutScope::TextEntry
        } else {
            ShortcutScope::Workspace
        };
        if scope == ShortcutScope::Workspace
            && event.key().eq_ignore_ascii_case("r")
            && !event.ctrl_key()
            && !event.meta_key()
            && !event.alt_key()
            && !event.shift_key()
            && self.entity_drag.as_ref().is_some_and(|drag| drag.active)
        {
            event.prevent_default();
            if let Some(drag) = self.entity_drag.as_mut() {
                drag.rotation_steps = (drag.rotation_steps + 1) % 4;
            }
            self.canvas_dirty = true;
            return true;
        }
        if scope == ShortcutScope::Workspace
            && event.key() == "Delete"
            && !event.ctrl_key()
            && !event.meta_key()
            && !event.alt_key()
            && self.mode == Mode::Select
        {
            event.prevent_default();
            return self.edit_selected_entity(ctx, EntityAction::Delete);
        }
        if scope == ShortcutScope::Workspace
            && !event.ctrl_key()
            && !event.meta_key()
            && !event.alt_key()
            && !event.shift_key()
        {
            if let Some(tool) = WorkspaceTool::from_key(self.mode, &event.key()) {
                event.prevent_default();
                return self.set_workspace_tool(tool);
            }
        }
        if scope == ShortcutScope::Workspace
            && self.blind_gesture.is_some()
            && event.key() == "Escape"
        {
            event.prevent_default();
            self.blind_gesture = None;
            self.canvas_dirty = true;
            return true;
        }
        if scope == ShortcutScope::Workspace
            && self.key_locker_assignment.is_some()
            && event.key() == "Escape"
        {
            event.prevent_default();
            self.key_locker_assignment = None;
            self.canvas_dirty = true;
            return true;
        }
        if scope == ShortcutScope::Workspace
            && self.mode == Mode::Select
            && self.workspace_tool == WorkspaceTool::Decorate
            && !event.ctrl_key()
            && !event.meta_key()
            && !event.alt_key()
            && !event.shift_key()
        {
            let action = match event.key().to_ascii_lowercase().as_str() {
                "i" => Some(DecoratorAction::ToggleIce),
                "g" => Some(DecoratorAction::ToggleGlass),
                "d" => Some(DecoratorAction::CycleDirection),
                "k" => Some(DecoratorAction::BeginKeyLocker),
                _ => None,
            };
            if let Some(action) = action {
                event.prevent_default();
                return self.edit_selected_decorator(ctx, action);
            }
        }
        if scope == ShortcutScope::Workspace
            && self.mode == Mode::Brush
            && !event.ctrl_key()
            && !event.meta_key()
            && !event.alt_key()
            && self.blind_gesture.is_none()
        {
            let key = event.key().to_ascii_lowercase();
            let brush_change = match key.as_str() {
                "b" => {
                    self.blind_brush_tool = BlindBrushTool::Paint;
                    self.workspace_tool = WorkspaceTool::Paint;
                    true
                }
                "f" => {
                    self.blind_brush_tool = BlindBrushTool::FloodFill;
                    self.workspace_tool = WorkspaceTool::Fill;
                    true
                }
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" => {
                    self.blind_color_index = key.parse().expect("matched ASCII brush color");
                    true
                }
                "0" => {
                    self.blind_color_index = 10;
                    true
                }
                _ => false,
            };
            if brush_change {
                event.prevent_default();
                return true;
            }
        }
        let Some(shortcut) = resolve_shortcut(
            &event.key(),
            event.ctrl_key() || event.meta_key(),
            event.shift_key(),
            scope,
        ) else {
            return false;
        };
        if matches!(shortcut, Shortcut::MoveSelection(_))
            && (self.mode != Mode::Select
                || self.workspace_tool != WorkspaceTool::Transform
                || self.selected_entity_ids().is_empty())
        {
            return false;
        }
        event.prevent_default();

        match shortcut {
            Shortcut::SelectMode(mode) => self.set_mode(mode),
            Shortcut::MoveSelection(direction) => {
                self.move_selected_entity_with_keyboard(ctx, direction)
            }
            Shortcut::Undo => self.undo(ctx),
            Shortcut::TogglePalette => {
                self.palette_open = !self.palette_open;
                self.palette_query.clear();
                self.open_menu = None;
                true
            }
            Shortcut::CloseOverlay => {
                let changed = self.palette_open || self.open_menu.is_some();
                self.palette_open = false;
                self.open_menu = None;
                changed
            }
            Shortcut::PaletteSubmit => {
                let Some(command) = self.filtered_palette_commands().first().copied() else {
                    return false;
                };
                self.run_palette(ctx, command)
            }
        }
    }

    fn run_palette(&mut self, ctx: &Context<Self>, command: PaletteCommand) -> bool {
        self.palette_open = false;
        match command {
            PaletteCommand::Select => self.set_mode(Mode::Select),
            PaletteCommand::Map => self.set_mode(Mode::Map),
            PaletteCommand::Brush => self.set_mode(Mode::Brush),
            PaletteCommand::Sandbox => self.set_mode(Mode::Sandbox),
            PaletteCommand::Undo => self.undo(ctx),
            PaletteCommand::ToggleTheme => {
                self.theme.toggle();
                self.canvas_dirty = true;
                self.push_toast(
                    format!(
                        "Using {} theme (user override)",
                        self.theme.active().label()
                    ),
                    "info",
                );
                true
            }
            PaletteCommand::Reconnect => {
                self.start_connection(ctx);
                true
            }
        }
    }

    fn filtered_palette_commands(&self) -> Vec<PaletteCommand> {
        let query = self.palette_query.trim().to_ascii_lowercase();
        PaletteCommand::ALL
            .into_iter()
            .filter(|command| {
                query.is_empty() || command.label().to_ascii_lowercase().contains(&query)
            })
            .collect()
    }

    fn start_connection(&mut self, ctx: &Context<Self>) {
        if self.target.project_id.as_str().is_empty() || self.target.level_id.as_str().is_empty() {
            self.rpc_state = RpcState::Offline;
            return;
        }
        self.reconnect_timer = None;
        self.connection_attempt += 1;
        self.rpc = None;
        self.rpc_state = RpcState::Connecting;
        self.presence.clear();
        self.last_sent_cursor = None;
        self.cursor_timer = None;
        self.cursor_in_flight = false;
        self.image_request_interrupted();
        self.pending_commands.clear();
        self.pending_cells.clear();
        self.pending_entities.clear();
        self.pending_placements.clear();
        self.pending_grid = None;
        self.pending_edge_view = None;
        self.blind_gesture = None;
        self.entity_drag = None;
        self.pan_gesture = None;
        self.map_resize_gesture = None;
        self.marquee_gesture = None;
        self.key_locker_assignment = None;
        self.canvas_dirty = true;
        spawn_connection(ctx, self.connection_attempt, self.target.clone());
    }

    fn handle_rpc_update(&mut self, ctx: &Context<Self>, update: RpcUpdate) -> bool {
        match update {
            RpcUpdate::Snapshot(snapshot) => self.accept_server_snapshot(ctx, snapshot),
            RpcUpdate::History {
                target,
                through_sequence,
                events,
            } => self.accept_server_history(ctx, target, through_sequence, events),
            RpcUpdate::Subscription(LevelSubscriptionItem::Snapshot { snapshot }) => {
                self.accept_server_snapshot(ctx, *snapshot)
            }
            RpcUpdate::Subscription(LevelSubscriptionItem::Event { event }) => {
                self.accept_server_event(ctx, *event)
            }
            RpcUpdate::Subscription(LevelSubscriptionItem::ResyncRequired {
                target,
                missed_events,
            }) => {
                if target == self.target {
                    self.request_resync(ctx, format!("subscription missed {missed_events} events"));
                }
                true
            }
            RpcUpdate::Presence(item) => {
                let notice = match &item {
                    LevelPresenceItem::Joined { participant, .. } => {
                        Some((format!("{} joined the level", participant.actor), "info"))
                    }
                    LevelPresenceItem::Left { presence_id, .. } => {
                        self.presence.participant(presence_id).map(|participant| {
                            (format!("{} left the level", participant.actor), "info")
                        })
                    }
                    _ => None,
                };
                let changed = self.presence.apply(&self.target, item);
                if changed {
                    self.canvas_dirty = true;
                    if let Some((message, level)) = notice {
                        self.push_toast(message, level);
                    }
                }
                changed
            }
            RpcUpdate::Closed(error) => {
                if matches!(self.rpc_state, RpcState::Offline) {
                    return false;
                }
                self.rpc = None;
                self.presence.clear();
                self.image_request_interrupted();
        self.pending_commands.clear();
                self.pending_cells.clear();
                self.pending_entities.clear();
                self.pending_placements.clear();
                self.pending_grid = None;
                self.pending_edge_view = None;
                self.blind_gesture = None;
                self.entity_drag = None;
                self.pan_gesture = None;
                self.map_resize_gesture = None;
                self.marquee_gesture = None;
                self.placement_drag = None;
                self.key_locker_assignment = None;
                self.canvas_dirty = true;
                self.schedule_reconnect(ctx, error);
                true
            }
        }
    }

    fn queue_cursor(&mut self, ctx: &Context<Self>, cursor: Option<GridPoint>) -> bool {
        if self.desired_cursor == cursor {
            return false;
        }
        self.desired_cursor = cursor;
        if matches!(self.rpc_state, RpcState::Online) {
            self.schedule_cursor_flush(ctx);
        }
        false
    }

    fn schedule_cursor_flush(&mut self, ctx: &Context<Self>) {
        if self.cursor_timer.is_some() || self.cursor_in_flight {
            return;
        }
        let attempt = self.connection_attempt;
        let link = ctx.link().clone();
        self.cursor_timer = Some(Timeout::new(50, move || {
            link.send_message(Msg::FlushCursor(attempt));
        }));
    }

    fn flush_cursor(&mut self, ctx: &Context<Self>) {
        if self.cursor_in_flight
            || !matches!(self.rpc_state, RpcState::Online)
            || self.last_sent_cursor == Some(self.desired_cursor)
        {
            return;
        }
        let Some(rpc) = self.rpc.clone() else {
            return;
        };
        let cursor = self.desired_cursor;
        let request = UpdateLevelCursorRequest {
            target: self.target.clone(),
            cursor,
        };
        let attempt = self.connection_attempt;
        let link = ctx.link().clone();
        self.cursor_in_flight = true;
        wasm_bindgen_futures::spawn_local(async move {
            link.send_message(Msg::CursorSent {
                attempt,
                cursor,
                result: rpc.update_level_cursor(request).await,
            });
        });
    }

    fn schedule_reconnect(&mut self, ctx: &Context<Self>, reason: String) {
        self.rpc = None;
        self.rpc_state = RpcState::Offline;
        self.reconnect_failures = self.reconnect_failures.saturating_add(1);
        let delay_ms = reconnect_delay_ms(self.reconnect_failures);
        let attempt = self.connection_attempt;
        let link = ctx.link().clone();
        self.reconnect_timer = Some(Timeout::new(delay_ms, move || {
            link.send_message(Msg::RetryConnection(attempt));
        }));
        self.push_toast(
            format!(
                "Live session unavailable; editor is read-only and retries in {:.0}s: {reason}",
                f64::from(delay_ms) / 1_000.0
            ),
            "warning",
        );
    }

    fn accept_server_snapshot(
        &mut self,
        ctx: &Context<Self>,
        response: LevelSnapshotResponse,
    ) -> bool {
        if response.target != self.target {
            self.connection_attempt += 1;
            self.rpc = None;
            self.image_request_interrupted();
        self.pending_commands.clear();
            self.pending_cells.clear();
            self.pending_entities.clear();
            self.pending_placements.clear();
            self.canvas_dirty = true;
            self.schedule_reconnect(
                ctx,
                format!(
                    "snapshot referenced unexpected target {} / {}",
                    response.target.project_id, response.target.level_id
                ),
            );
            return true;
        }
        let computed_hash = response.snapshot.content_hash().to_string();
        if computed_hash != response.level_hash {
            self.connection_attempt += 1;
            self.rpc = None;
            self.image_request_interrupted();
        self.pending_commands.clear();
            self.pending_cells.clear();
            self.pending_entities.clear();
            self.pending_placements.clear();
            self.canvas_dirty = true;
            self.schedule_reconnect(
                ctx,
                "server snapshot hash did not match oreak-core".to_owned(),
            );
            return true;
        }
        if self
            .server_sequence
            .is_some_and(|sequence| response.server_sequence < sequence)
        {
            return false;
        }
        if self.server_sequence == Some(response.server_sequence)
            && self.server_hash.as_deref() == Some(response.level_hash.as_str())
            && self.model.timeline().snapshot().content_hash().to_string() == response.level_hash
        {
            return false;
        }

        let previous_size = self.model.timeline().snapshot().size();
        if let Err(error) = self.model.replace_snapshot(response.snapshot) {
            self.connection_attempt += 1;
            self.rpc = None;
            self.image_request_interrupted();
        self.pending_commands.clear();
            self.pending_cells.clear();
            self.pending_entities.clear();
            self.pending_placements.clear();
            self.canvas_dirty = true;
            self.schedule_reconnect(
                ctx,
                format!("server snapshot was rejected locally: {error}"),
            );
            return true;
        }
        self.blind_gesture = None;
        self.key_locker_assignment = None;
        self.server_sequence = Some(response.server_sequence);
        self.server_hash = Some(response.level_hash);
        self.image_request_interrupted();
        self.pending_commands.clear();
        self.pending_cells.clear();
        self.pending_entities.clear();
        self.pending_placements.clear();
        self.pending_grid = None;
        self.pending_edge_view = None;
        self.clear_missing_entity_selection();
        self.sync_resize_fields();
        if self.model.timeline().snapshot().size() != previous_size {
            self.frame_grid();
        }
        self.canvas_dirty = true;
        self.persist_draft();
        true
    }

    fn accept_server_history(
        &mut self,
        ctx: &Context<Self>,
        target: ProjectLevelTarget,
        through_sequence: u64,
        events: Vec<HistoryEvent>,
    ) -> bool {
        let event_tip = events.last().map_or(0, |event| event.sequence);
        if target != self.target
            || self.server_sequence != Some(through_sequence)
            || event_tip != through_sequence
        {
            return self.reject_server_history(
                ctx,
                "history did not match the subscribed target and snapshot sequence".to_owned(),
            );
        }
        let projection = match hydrate_history(self.model.timeline().snapshot(), &events) {
            Ok(projection) => projection,
            Err(error) => return self.reject_server_history(ctx, error),
        };

        self.server_events = events;
        self.seen_commands = projection.seen_commands;
        self.remote_blame = projection.cell_blame;
        self.remote_entity_blame = projection.entity_blame;
        self.canvas_dirty = true;
        true
    }

    fn reject_server_history(&mut self, ctx: &Context<Self>, error: String) -> bool {
        self.connection_attempt += 1;
        self.rpc = None;
        self.image_request_interrupted();
        self.pending_commands.clear();
        self.pending_cells.clear();
        self.pending_entities.clear();
        self.pending_placements.clear();
        self.pending_grid = None;
        self.pending_edge_view = None;
        self.canvas_dirty = true;
        self.schedule_reconnect(ctx, format!("server history hydration failed: {error}"));
        true
    }

    fn accept_server_event(&mut self, ctx: &Context<Self>, update: LevelEvent) -> bool {
        if update.target != self.target {
            self.request_resync(ctx, "received an event for the wrong level".to_owned());
            return true;
        }

        let command_id = update.event.metadata.id.to_string();
        if self.seen_commands.contains(&command_id) {
            self.remove_pending(&command_id);
            return true;
        }

        let current_sequence = self.server_sequence.unwrap_or(0);
        if update.server_sequence <= current_sequence {
            self.seen_commands.insert(command_id.clone());
            self.remove_pending(&command_id);
            return true;
        }
        if update.server_sequence != current_sequence + 1 {
            self.request_resync(
                ctx,
                format!(
                    "server sequence jumped from {current_sequence} to {}",
                    update.server_sequence
                ),
            );
            return true;
        }

        let previous_snapshot = self.model.timeline().snapshot().clone();
        self.blind_gesture = None;
        self.entity_drag = None;
        self.marquee_gesture = None;
        self.key_locker_assignment = None;
        match self.model.apply_server_event(&update.event) {
            Ok(ModelChange::Applied { .. }) => {}
            Ok(ModelChange::NoChange) => {
                self.request_resync(ctx, "server event became a local no-op".to_owned());
                return true;
            }
            Err(error) => {
                self.request_resync(ctx, format!("server event was rejected locally: {error}"));
                return true;
            }
        }

        let computed_hash = self.model.timeline().snapshot().content_hash().to_string();
        if computed_hash != update.level_hash {
            let _ = self.model.replace_snapshot(previous_snapshot);
            self.request_resync(
                ctx,
                format!(
                    "hash mismatch at server sequence {}",
                    update.server_sequence
                ),
            );
            return true;
        }

        let previous_size = previous_snapshot.size();
        let current_size = self.model.timeline().snapshot().size();
        let command_edge =
            map_resize_edge_from_command(&update.event.command, previous_size, current_size);
        self.server_sequence = Some(update.server_sequence);
        self.server_hash = Some(update.level_hash);
        self.server_events.push(update.event);
        let projection =
            match hydrate_history(self.model.timeline().snapshot(), &self.server_events) {
                Ok(projection) => projection,
                Err(error) => {
                    self.request_resync(ctx, format!("history projection failed: {error}"));
                    return true;
                }
            };
        self.seen_commands = projection.seen_commands;
        self.remote_blame = projection.cell_blame;
        self.remote_entity_blame = projection.entity_blame;
        let edge_view = (self.pending_grid.as_deref() == Some(command_id.as_str()))
            .then(|| self.pending_edge_view.take())
            .flatten()
            .filter(|edge| map_resize_edge_changes_only_axis(*edge, previous_size, current_size))
            .or(command_edge)
            .map(|edge| (edge, previous_size, current_size));
        self.remove_pending(&command_id);
        self.clear_missing_entity_selection();
        if self.model.timeline().snapshot().size() != previous_snapshot.size() {
            self.map_resize_gesture = None;
            self.marquee_gesture = None;
            self.entity_drag = None;
            self.placement_drag = None;
            self.sync_resize_fields();
            if let Some((edge, before, after)) = edge_view {
                let cell_size = CELL_SIZE * self.viewport.scale;
                match edge {
                    MapResizeEdge::Top => {
                        self.viewport.offset_y -=
                            (f64::from(after.height()) - f64::from(before.height())) * cell_size;
                    }
                    MapResizeEdge::Left => {
                        self.viewport.offset_x +=
                            (f64::from(before.width()) - f64::from(after.width())) * cell_size;
                    }
                    MapResizeEdge::Right | MapResizeEdge::Bottom => {}
                }
                self.translate_cell_selection_after_edge_resize(edge, before, after);
            } else {
                self.frame_grid();
            }
        }
        self.canvas_dirty = true;
        self.persist_draft();
        true
    }

    fn handle_apply_response(
        &mut self,
        ctx: &Context<Self>,
        command_id: String,
        result: Result<ApplyCommandResponse, String>,
    ) -> bool {
        self.distribution_command_finished(&command_id, result.as_ref().err());
        match result {
            Ok(response) => match response.result {
                ApplyCommandResult::Applied { event } => self.accept_server_event(
                    ctx,
                    LevelEvent {
                        target: response.target,
                        server_sequence: response.server_sequence,
                        level_hash: response.level_hash,
                        event: *event,
                    },
                ),
                ApplyCommandResult::NoChange => {
                    self.remove_pending(&command_id);
                    let current_sequence = self.server_sequence.unwrap_or(0);
                    if response.target != self.target {
                        self.request_resync(
                            ctx,
                            "no-op response referenced the wrong level".to_owned(),
                        );
                    } else if response.server_sequence > current_sequence {
                        self.request_resync(
                            ctx,
                            format!(
                                "no-op response skipped server sequence {current_sequence} to {}",
                                response.server_sequence
                            ),
                        );
                    } else if response.server_sequence == current_sequence
                        && self.model.timeline().snapshot().content_hash().to_string()
                            != response.level_hash
                    {
                        self.request_resync(
                            ctx,
                            "no-op response exposed a hash mismatch".to_owned(),
                        );
                    } else if response.server_sequence == current_sequence {
                        self.server_hash = Some(response.level_hash);
                    }
                    true
                }
            },
            Err(error) => {
                self.remove_pending(&command_id);
                self.push_toast(error, "warning");
                self.canvas_dirty = true;
                true
            }
        }
    }

    fn handle_undo_response(
        &mut self,
        ctx: &Context<Self>,
        command_id: String,
        result: Result<UndoLatestResponse, String>,
    ) -> bool {
        match result {
            Ok(response) => self.accept_server_event(
                ctx,
                LevelEvent {
                    target: response.target,
                    server_sequence: response.server_sequence,
                    level_hash: response.level_hash,
                    event: response.event,
                },
            ),
            Err(error) => {
                self.remove_pending(&command_id);
                self.push_toast(error, "warning");
                true
            }
        }
    }

    fn request_resync(&mut self, ctx: &Context<Self>, reason: String) {
        if matches!(self.rpc_state, RpcState::Resyncing | RpcState::Connecting) {
            return;
        }
        if self.rpc.is_none() {
            self.image_request_interrupted();
        self.pending_commands.clear();
            self.pending_cells.clear();
            self.pending_entities.clear();
            self.pending_placements.clear();
            self.pending_grid = None;
            self.canvas_dirty = true;
            self.schedule_reconnect(ctx, format!("resync unavailable: {reason}"));
            return;
        }
        self.push_toast(format!("Resync requested: {reason}"), "info");
        self.start_connection(ctx);
        self.rpc_state = RpcState::Resyncing;
    }

    fn remove_pending(&mut self, command_id: &str) {
        self.pending_commands.remove(command_id);
        self.pending_cells
            .retain(|_, pending| pending.command_id != command_id);
        self.pending_entities
            .retain(|_, pending| pending.command_id != command_id);
        self.pending_placements
            .retain(|_, pending_command| pending_command != command_id);
        if self.pending_grid.as_deref() == Some(command_id) {
            self.pending_grid = None;
            self.pending_edge_view = None;
        }
        self.canvas_dirty = true;
    }

    fn effective_cell(&self, point: GridPoint) -> Option<CellKind> {
        self.pending_cells
            .get(&point)
            .map(|pending| pending.kind)
            .or_else(|| self.model.cell(point).ok())
    }

    fn translate_cell_selection_after_edge_resize(
        &mut self,
        edge: MapResizeEdge,
        before: GridSize,
        after: GridSize,
    ) {
        let Some(Selection::Cell(point)) = self.selection else {
            return;
        };
        let (delta_x, delta_y) = match edge {
            MapResizeEdge::Top => (0, i32::from(after.height()) - i32::from(before.height())),
            MapResizeEdge::Left => (i32::from(after.width()) - i32::from(before.width()), 0),
            MapResizeEdge::Right | MapResizeEdge::Bottom => (0, 0),
        };
        self.selection = point
            .x
            .checked_add_signed(delta_x as i16)
            .zip(point.y.checked_add_signed(delta_y as i16))
            .filter(|(x, y)| *x < after.width() && *y < after.height())
            .map(|(x, y)| Selection::Cell(GridPoint::new(x, y)));
    }

    fn frame_grid(&mut self) {
        self.viewport = ViewportTransform::frame_rect(
            self.model.timeline().snapshot().size(),
            f64::from(self.canvas_width),
            f64::from(self.canvas_height),
            BOARD_ORIGIN,
            CELL_SIZE,
        );
        self.canvas_dirty = true;
    }

    fn canvas_resized(&mut self, width: u32, height: u32) -> bool {
        let width = width.clamp(1, 4096);
        let height = height.clamp(1, 4096);
        if self.canvas_size_initialized
            && self.canvas_width == width
            && self.canvas_height == height
        {
            return false;
        }
        let previous_width = self.canvas_width;
        let previous_height = self.canvas_height;
        self.canvas_width = width;
        self.canvas_height = height;
        if let Some(canvas) = self.canvas_ref.cast::<HtmlCanvasElement>() {
            canvas.set_width(width);
            canvas.set_height(height);
        }
        if self.canvas_size_initialized {
            self.viewport.offset_x += (f64::from(width) - f64::from(previous_width)) / 2.0;
            self.viewport.offset_y += (f64::from(height) - f64::from(previous_height)) / 2.0;
        } else {
            self.canvas_size_initialized = true;
            self.frame_grid();
        }
        self.canvas_dirty = true;
        true
    }

    fn ensure_canvas_resize_observer(&mut self, ctx: &Context<Self>) {
        if self.canvas_resize_observer.is_some() {
            return;
        }
        let Some(canvas) = self.canvas_ref.cast::<HtmlCanvasElement>() else {
            return;
        };
        let canvas_ref = self.canvas_ref.clone();
        let link = ctx.link().clone();
        let callback = Closure::<dyn FnMut()>::new(move || {
            let Some(canvas) = canvas_ref.cast::<HtmlCanvasElement>() else {
                return;
            };
            link.send_message(Msg::CanvasResized(
                u32::try_from(canvas.client_width().max(1)).unwrap_or(CANVAS_WIDTH),
                u32::try_from(canvas.client_height().max(1)).unwrap_or(CANVAS_HEIGHT),
            ));
        });
        let Ok(observer) = ResizeObserver::new(callback.as_ref().unchecked_ref()) else {
            return;
        };
        observer.observe(canvas.unchecked_ref::<Element>());
        self.canvas_resize_callback = Some(callback);
        self.canvas_resize_observer = Some(observer);
    }

    fn disconnect_canvas_resize_observer(&mut self) {
        if let Some(observer) = self.canvas_resize_observer.take() {
            observer.disconnect();
        }
        self.canvas_resize_callback = None;
        self.canvas_size_initialized = false;
    }

    fn begin_sidebar_resize(&mut self, side: SidebarSide, event: PointerEvent) -> bool {
        if event.button() != 0 {
            return false;
        }
        event.prevent_default();
        if let Some(root) = self.root_ref.cast::<HtmlElement>() {
            let _ = root.set_pointer_capture(event.pointer_id());
        }
        self.sidebar_resize = Some(SidebarResize {
            side,
            pointer_id: event.pointer_id(),
            start_client_x: f64::from(event.client_x()),
            start_width: match side {
                SidebarSide::Left => self.left_sidebar_width,
                SidebarSide::Right => self.right_sidebar_width,
            },
        });
        true
    }

    fn resize_sidebar(&mut self, event: PointerEvent) -> bool {
        let Some(resize) = self.sidebar_resize.as_ref() else {
            return false;
        };
        if resize.pointer_id != event.pointer_id() {
            return false;
        }
        let delta = f64::from(event.client_x()) - resize.start_client_x;
        let width = match resize.side {
            SidebarSide::Left => resize.start_width + delta,
            SidebarSide::Right => resize.start_width - delta,
        }
        .clamp(180.0, 480.0);
        let target = match resize.side {
            SidebarSide::Left => &mut self.left_sidebar_width,
            SidebarSide::Right => &mut self.right_sidebar_width,
        };
        if (*target - width).abs() < f64::EPSILON {
            return false;
        }
        *target = width;
        // CSS-grid width changes may reset/stretch the canvas before ResizeObserver
        // delivers its next callback. Force the current render pass to repaint it.
        self.canvas_dirty = true;
        true
    }

    fn end_sidebar_resize(&mut self, event: PointerEvent) -> bool {
        if self
            .sidebar_resize
            .as_ref()
            .is_none_or(|resize| resize.pointer_id != event.pointer_id())
        {
            return false;
        }
        if let Some(root) = self.root_ref.cast::<HtmlElement>() {
            let _ = root.release_pointer_capture(event.pointer_id());
        }
        self.sidebar_resize = None;
        true
    }

    fn sync_resize_fields(&mut self) {
        let size = self.model.timeline().snapshot().size();
        self.resize_width = size.width().to_string();
        self.resize_height = size.height().to_string();
    }

    fn zoom_canvas(&mut self, factor: f64) -> bool {
        if self.blind_gesture.is_some() || self.entity_drag.is_some() || self.drag_kind.is_some() {
            return false;
        }
        let center_x = f64::from(self.canvas_width) / 2.0;
        let center_y = f64::from(self.canvas_height) / 2.0;
        self.viewport = self
            .viewport
            .zoom_about(self.viewport.scale * factor, center_x, center_y);
        self.canvas_dirty = true;
        true
    }

    fn canvas_wheel(&mut self, event: WheelEvent) -> bool {
        event.prevent_default();
        if self.blind_gesture.is_some() || self.entity_drag.is_some() || self.drag_kind.is_some() {
            return false;
        }
        let Some(canvas) = self.canvas_ref.cast::<HtmlCanvasElement>() else {
            return false;
        };
        let bounds = canvas.get_bounding_client_rect();
        if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
            return false;
        }
        let x = (f64::from(event.client_x()) - bounds.left()) * f64::from(canvas.width())
            / bounds.width();
        let y = (f64::from(event.client_y()) - bounds.top()) * f64::from(canvas.height())
            / bounds.height();
        let factor = (-event.delta_y() * 0.0015).exp();
        self.viewport = self.viewport.zoom_about(self.viewport.scale * factor, x, y);
        self.canvas_dirty = true;
        true
    }

    fn preview_entity(&self, entity: &PlaceableEntity) -> PlaceableEntity {
        if let Some(projected) = self.image_projection(entity) { return projected; }
        let pending = self
            .pending_entities
            .get(entity.id())
            .and_then(|pending| pending.preview.as_ref());
        resolved_entity_preview(entity, None, pending)
    }

    fn entity_drag_preview(&self, drag: &EntityDrag) -> EntityDragPreview {
        let snapshot = self.model.timeline().snapshot();
        let dragged_ids = drag
            .origins
            .iter()
            .map(|(entity_id, _)| entity_id.clone())
            .collect::<BTreeSet<_>>();
        let entities = drag
            .origins
            .iter()
            .filter_map(|(entity_id, origin)| {
                snapshot.entity(entity_id).and_then(|entity| {
                    dragged_entity_preview(
                        entity,
                        *origin,
                        drag.start,
                        drag.current,
                        drag.rotation_steps,
                    )
                })
            })
            .collect::<Vec<_>>();
        let size = snapshot.size();
        let mut occupied = BTreeSet::new();
        let mut valid = drag.pointer_on_grid && entities.len() == drag.origins.len();
        for entity in &entities {
            let Some(points) = shape_world_points(entity.origin(), entity.shape()) else {
                valid = false;
                continue;
            };
            for point in points {
                let collides = snapshot
                    .entity_at(point)
                    .is_some_and(|occupant| drag.duplicate || !dragged_ids.contains(occupant.id()));
                if point.x >= size.width()
                    || point.y >= size.height()
                    || self.effective_cell(point) != Some(CellKind::Floor)
                    || self.pending_placements.contains_key(&point)
                    || collides
                    || !occupied.insert(point)
                {
                    valid = false;
                }
            }
        }
        EntityDragPreview { entities, valid }
    }

    fn activity_events(&self) -> &[HistoryEvent] {
        if matches!(self.rpc_state, RpcState::Online | RpcState::Resyncing) {
            &self.server_events
        } else {
            self.model.timeline().events()
        }
    }

    fn selected_cell(&self) -> Option<GridPoint> {
        match self.selection.as_ref() {
            Some(Selection::Cell(point)) => Some(*point),
            _ => None,
        }
    }

    fn selected_entity_id(&self) -> Option<&EntityId> {
        match self.selection.as_ref() {
            Some(Selection::Entities(entity_ids)) if entity_ids.len() == 1 => entity_ids.first(),
            _ => None,
        }
    }

    fn selected_entity_ids(&self) -> &[EntityId] {
        match self.selection.as_ref() {
            Some(Selection::Entities(entity_ids)) => entity_ids,
            _ => &[],
        }
    }

    fn selected_entities(&self) -> Vec<&PlaceableEntity> {
        let snapshot = self.model.timeline().snapshot();
        self.selected_entity_ids()
            .iter()
            .filter_map(|entity_id| snapshot.entity(entity_id))
            .collect()
    }

    fn selected_entity(&self) -> Option<&PlaceableEntity> {
        self.model
            .timeline()
            .snapshot()
            .entity(self.selected_entity_id()?)
    }

    fn blame_for_selection(&self) -> Option<BlameEntry> {
        match self.selection.as_ref()? {
            Selection::Cell(point) => self.blame_for_cell(*point),
            Selection::Entities(entity_ids) if entity_ids.len() == 1 => {
                self.blame_for_entity(&entity_ids[0])
            }
            Selection::Entities(_) => None,
        }
    }

    fn blame_for_cell(&self, point: GridPoint) -> Option<BlameEntry> {
        let local = self.model.timeline().blame_cell(point);
        self.resolve_blame(local, self.remote_blame.get(&point))
    }

    fn blame_for_entity(&self, entity_id: &EntityId) -> Option<BlameEntry> {
        let local = self.model.timeline().blame_entity(entity_id);
        self.resolve_blame(local, self.remote_entity_blame.get(entity_id))
    }

    fn resolve_blame(
        &self,
        local: Option<BlameEntry>,
        remote: Option<&BlameEntry>,
    ) -> Option<BlameEntry> {
        if matches!(self.rpc_state, RpcState::Online | RpcState::Resyncing) {
            remote.cloned()
        } else if local
            .as_ref()
            .is_some_and(|entry| entry.actor.as_str() == self.model.actor().as_str())
        {
            local
        } else {
            remote.cloned().or(local)
        }
    }

    fn history_for_selection(&self, selection: &Selection) -> Vec<&HistoryEvent> {
        let target = match selection {
            Selection::Cell(point) => LevelTarget::Cell(*point),
            Selection::Entities(entity_ids) if entity_ids.len() == 1 => {
                LevelTarget::Entity(entity_ids[0].clone())
            }
            Selection::Entities(_) => return Vec::new(),
        };
        if matches!(self.rpc_state, RpcState::Online | RpcState::Resyncing) {
            self.server_events
                .iter()
                .filter(|event| event.changes.iter().any(|change| change.target == target))
                .collect()
        } else {
            match selection {
                Selection::Cell(point) => self.model.timeline().history_for_cell(*point),
                Selection::Entities(entity_ids) => {
                    self.model.timeline().history_for_entity(&entity_ids[0])
                }
            }
        }
    }

    fn clear_missing_entity_selection(&mut self) {
        let size = self.model.timeline().snapshot().size();
        if self
            .isolated_blind
            .as_ref()
            .is_some_and(|entity_id| self.model.timeline().snapshot().entity(entity_id).is_none())
        {
            self.isolated_blind = None;
        }
        match self.selection.as_mut() {
            Some(Selection::Entities(entity_ids)) => {
                entity_ids.retain(|entity_id| {
                    self.model.timeline().snapshot().entity(entity_id).is_some()
                });
                if entity_ids.is_empty() {
                    self.selection = None;
                }
            }
            Some(Selection::Cell(point)) if point.x >= size.width() || point.y >= size.height() => {
                self.selection = None;
            }
            _ => {}
        }
    }

    fn push_toast(&mut self, message: String, tone: &'static str) {
        let id = self.next_toast_id;
        self.next_toast_id += 1;
        self.toasts.push(Toast {
            id,
            message,
            tone,
            occurred_at_ms: now_ms(),
        });
        self.active_toasts.insert(id);
        if self.toasts.len() > 200 {
            let removed = self.toasts.remove(0);
            self.active_toasts.remove(&removed.id);
        }
    }

    fn persist_draft(&self) {
        let Some(user) = self.user.as_ref() else {
            return;
        };
        let key = draft_storage_key(
            &user.id,
            self.target.project_id.as_str(),
            self.target.level_id.as_str(),
        );
        let Ok(json) = serde_json::to_string(self.model.timeline().snapshot()) else {
            return;
        };
        if let Some(store) = storage() {
            let _ = store.set_item(&key, &json);
        }
    }

    fn draw_canvas(&self) -> Result<(), JsValue> {
        let canvas = self
            .canvas_ref
            .cast::<HtmlCanvasElement>()
            .ok_or_else(|| JsValue::from_str("canvas is unavailable"))?;
        let context = canvas
            .get_context("2d")?
            .ok_or_else(|| JsValue::from_str("2D canvas context is unavailable"))?
            .dyn_into::<CanvasRenderingContext2d>()?;
        let palette = CanvasPalette::for_theme(self.theme.active());
        let canvas_width = f64::from(canvas.width());
        let canvas_height = f64::from(canvas.height());

        context.set_fill_style_str(palette.background);
        context.fill_rect(0.0, 0.0, canvas_width, canvas_height);

        context.set_stroke_style_str(palette.guide);
        context.set_line_width(1.0);
        for guide in (14..canvas.width()).step_by(16) {
            let guide = f64::from(guide) + 0.5;
            context.begin_path();
            context.move_to(guide, 0.0);
            context.line_to(guide, canvas_height);
            context.stroke();
        }
        for guide in (14..canvas.height()).step_by(16) {
            let guide = f64::from(guide) + 0.5;
            context.begin_path();
            context.move_to(0.0, guide);
            context.line_to(canvas_width, guide);
            context.stroke();
        }

        context.set_font(&self.ui_settings.canvas_font(11, false));
        context.set_text_align("center");
        context.set_text_baseline("middle");

        context.save();
        context.set_transform(
            self.viewport.scale,
            0.0,
            0.0,
            self.viewport.scale,
            self.viewport.offset_x - BOARD_ORIGIN * self.viewport.scale,
            self.viewport.offset_y - BOARD_ORIGIN * self.viewport.scale,
        )?;

        let size = self.model.timeline().snapshot().size();
        for y in 0..size.height() {
            for x in 0..size.width() {
                let point = GridPoint::new(x, y);
                let left = BOARD_ORIGIN + f64::from(x) * CELL_SIZE;
                let top = BOARD_ORIGIN + f64::from(y) * CELL_SIZE;
                let kind = self.effective_cell(point).unwrap_or(CellKind::Floor);

                context.set_fill_style_str(match kind {
                    CellKind::Floor => palette.floor,
                    CellKind::Wall => palette.wall,
                });
                context.fill_rect(left + 1.0, top + 1.0, CELL_SIZE - 2.0, CELL_SIZE - 2.0);

                if kind == CellKind::Floor {
                    context.set_fill_style_str(palette.floor_mark);
                    context.fill_rect(left + 8.0, top + 8.0, 3.0, 3.0);
                } else {
                    context.set_stroke_style_str(palette.wall_detail);
                    context.set_line_width(1.0);
                    context.begin_path();
                    context.move_to(left + 12.0, top + CELL_SIZE - 12.0);
                    context.line_to(left + CELL_SIZE - 12.0, top + 12.0);
                    context.stroke();
                }

                context.set_stroke_style_str(palette.grid);
                context.set_line_width(1.0);
                context.stroke_rect(left + 0.5, top + 0.5, CELL_SIZE - 1.0, CELL_SIZE - 1.0);

                if self.pending_cells.contains_key(&point) {
                    context.set_stroke_style_str(palette.selection);
                    context.set_line_width(2.0);
                    context.stroke_rect(left + 6.0, top + 6.0, CELL_SIZE - 12.0, CELL_SIZE - 12.0);
                }
            }
        }

        for entity in self.model.timeline().snapshot().entities() {
            if self
                .isolated_blind
                .as_ref()
                .is_some_and(|entity_id| entity.id() != entity_id)
            {
                continue;
            }
            let display = self.preview_entity(entity);
            context.save();
            if self.distribution.hover.is_some() { context.set_global_alpha(0.18); }
            self.draw_entity(&context, &palette, &display)?;
            context.restore();
        }
        if self.isolated_blind.is_none() {
            let snapshot = self.model.timeline().snapshot();
            let pending_duplicates = self
                .pending_entities
                .values()
                .filter_map(|pending| pending.preview.as_ref())
                .filter(|preview| snapshot.entity(preview.id()).is_none());
            context.save();
            context.set_global_alpha(0.72);
            for entity in pending_duplicates {
                self.draw_entity(&context, &palette, entity)?;
            }
            context.restore();

            if let Some(drag) = self.entity_drag.as_ref().filter(|drag| drag.active) {
                let preview = self.entity_drag_preview(drag);
                self.draw_entity_drag_ghost(&context, &palette, &preview);
            }
        }
        if self.isolated_blind.is_none() {
            context.save();
            if self.distribution.hover.is_some() { context.set_global_alpha(0.18); }
            self.draw_decorators(&context);
            context.restore();
        }
        self.draw_blind_gesture_preview(&context, &palette);
        self.draw_placement_preview(&context, &palette);
        self.draw_image_overlay(&context);
        self.draw_distribution_highlight(&context);

        context.set_fill_style_str(palette.label);
        for index in 0..size.width() {
            let center = BOARD_ORIGIN + f64::from(index) * CELL_SIZE + CELL_SIZE / 2.0;
            context.fill_text(&format!("{index:02}"), center, 23.0)?;
        }
        for index in 0..size.height() {
            let center = BOARD_ORIGIN + f64::from(index) * CELL_SIZE + CELL_SIZE / 2.0;
            context.fill_text(&format!("{index:02}"), 22.0, center)?;
        }

        match self.selection.as_ref() {
            Some(Selection::Cell(point)) => {
                draw_selection_cell(&context, &palette, *point);
            }
            Some(Selection::Entities(entity_ids)) => {
                context.set_stroke_style_str(palette.selection);
                context.set_line_width(3.0);
                for entity_id in entity_ids {
                    if let Some(entity) = self.model.timeline().snapshot().entity(entity_id) {
                        let display = self.preview_entity(entity);
                        let origin = display.origin();
                        for cell in display.shape().occupied_cells() {
                            let left =
                                BOARD_ORIGIN + f64::from(origin.x + u16::from(cell.x)) * CELL_SIZE;
                            let top =
                                BOARD_ORIGIN + f64::from(origin.y + u16::from(cell.y)) * CELL_SIZE;
                            draw_shape_boundary(
                                &context,
                                left,
                                top,
                                3.5,
                                shape_boundary_edges(display.shape(), cell),
                            );
                        }
                    }
                }
            }
            None => {}
        }

        for participant in self
            .presence
            .participants()
            .filter(|participant| !self.presence.is_self(&participant.id))
        {
            let Some(point) = participant.cursor else {
                continue;
            };
            let x = BOARD_ORIGIN + f64::from(point.x) * CELL_SIZE + CELL_SIZE / 2.0;
            let y = BOARD_ORIGIN + f64::from(point.y) * CELL_SIZE + CELL_SIZE / 2.0;
            let color = presence_color(participant.actor.as_str());
            context.set_fill_style_str(color);
            context.begin_path();
            context.arc(x, y, 7.0, 0.0, std::f64::consts::TAU)?;
            context.fill();
            context.set_stroke_style_str(palette.background);
            context.set_line_width(2.0);
            context.stroke();
            context.set_text_align("left");
            context.set_font(&self.ui_settings.canvas_font(10, true));
            context.fill_text(participant.actor.as_str(), x + 11.0, y - 10.0)?;
        }

        context.restore();
        if let Some(gesture) = self
            .marquee_gesture
            .as_ref()
            .filter(|gesture| gesture.active)
        {
            let left = gesture.start_canvas_x.min(gesture.current_canvas_x);
            let top = gesture.start_canvas_y.min(gesture.current_canvas_y);
            let width = (gesture.current_canvas_x - gesture.start_canvas_x).abs();
            let height = (gesture.current_canvas_y - gesture.start_canvas_y).abs();
            context.save();
            context.set_fill_style_str(palette.selection);
            context.set_global_alpha(0.12);
            context.fill_rect(left, top, width, height);
            context.set_global_alpha(1.0);
            context.set_stroke_style_str(palette.selection);
            context.set_line_width(1.5);
            context
                .set_line_dash(&js_sys::Array::of2(&5.into(), &3.into()))
                .ok();
            context.stroke_rect(left + 0.5, top + 0.5, width, height);
            context.set_line_dash(&js_sys::Array::new()).ok();
            context.restore();
        }
        if self.image_studio.apply.is_none() { self.draw_map_resize_handles(&context, &palette); }

        context.set_text_align("right");
        context.set_fill_style_str(palette.label);
        context.fill_text(
            &format!(
                "{} / {} EVENTS",
                self.mode.label().to_ascii_uppercase(),
                self.activity_events().len()
            ),
            canvas_width - 14.0,
            canvas_height - 14.0,
        )?;
        Ok(())
    }

    fn draw_map_resize_handles(&self, context: &CanvasRenderingContext2d, palette: &CanvasPalette) {
        if self.mode != Mode::Map || self.workspace_tool != WorkspaceTool::Resize {
            return;
        }
        let current_size = self.model.timeline().snapshot().size();
        let (size, left, top) = if let Some(gesture) = self.map_resize_gesture.as_ref() {
            let old_width = f64::from(gesture.start_size.width()) * gesture.cell_size;
            let old_height = f64::from(gesture.start_size.height()) * gesture.cell_size;
            let new_width = f64::from(gesture.preview_size.width()) * gesture.cell_size;
            let new_height = f64::from(gesture.preview_size.height()) * gesture.cell_size;
            let old_left = self.viewport.offset_x;
            let old_top = self.viewport.offset_y;
            let left = if gesture.edge == MapResizeEdge::Left {
                old_left + old_width - new_width
            } else {
                old_left
            };
            let top = if gesture.edge == MapResizeEdge::Top {
                old_top + old_height - new_height
            } else {
                old_top
            };
            (gesture.preview_size, left, top)
        } else {
            (current_size, self.viewport.offset_x, self.viewport.offset_y)
        };
        let width = f64::from(size.width()) * CELL_SIZE * self.viewport.scale;
        let height = f64::from(size.height()) * CELL_SIZE * self.viewport.scale;
        context.save();
        context.set_stroke_style_str(palette.selection);
        context.set_fill_style_str(palette.background);
        context.set_line_width(2.0);
        context
            .set_line_dash(&js_sys::Array::of2(&6.into(), &4.into()))
            .ok();
        context.stroke_rect(left, top, width, height);
        context.set_line_dash(&js_sys::Array::new()).ok();
        let handles = [
            (left + width / 2.0, top),
            (left + width, top + height / 2.0),
            (left + width / 2.0, top + height),
            (left, top + height / 2.0),
        ];
        for (x, y) in handles {
            context.fill_rect(x - 7.0, y - 7.0, 14.0, 14.0);
            context.stroke_rect(x - 6.5, y - 6.5, 13.0, 13.0);
        }
        context.set_fill_style_str(palette.selection);
        context.set_font(&self.ui_settings.canvas_font(10, false));
        context.set_text_align("center");
        let _ = context.fill_text(
            &format!("{} x {}", size.width(), size.height()),
            left + width / 2.0,
            top - 15.0,
        );
        context.restore();
    }

    fn draw_entity_drag_ghost(
        &self,
        context: &CanvasRenderingContext2d,
        palette: &CanvasPalette,
        preview: &EntityDragPreview,
    ) {
        let color = if preview.valid {
            palette.selection
        } else {
            palette.invalid
        };
        context.save();
        context.set_global_alpha(0.62);
        context.set_fill_style_str(color);
        context.set_stroke_style_str(color);
        context.set_line_width(3.0);
        for entity in &preview.entities {
            for cell in entity.shape().occupied_cells() {
                let left =
                    BOARD_ORIGIN + f64::from(entity.origin().x + u16::from(cell.x)) * CELL_SIZE;
                let top =
                    BOARD_ORIGIN + f64::from(entity.origin().y + u16::from(cell.y)) * CELL_SIZE;
                context.fill_rect(left + 8.0, top + 8.0, CELL_SIZE - 16.0, CELL_SIZE - 16.0);
                draw_shape_boundary(
                    context,
                    left,
                    top,
                    5.0,
                    shape_boundary_edges(entity.shape(), cell),
                );
            }
        }
        context.restore();
    }

    fn draw_placement_preview(&self, context: &CanvasRenderingContext2d, palette: &CanvasPalette) {
        let Some(drag) = self.placement_drag.as_ref() else {
            return;
        };
        let Some(origin) = drag.hover else {
            return;
        };
        let valid = self.can_place_shape(origin, drag.shape);
        context.save();
        context.set_global_alpha(0.72);
        context.set_fill_style_str(if valid {
            palette.selection
        } else {
            palette.invalid
        });
        context.set_stroke_style_str(if valid {
            palette.selection
        } else {
            palette.invalid
        });
        context.set_line_width(3.0);
        for cell in drag.shape.occupied_cells() {
            let left =
                BOARD_ORIGIN + f64::from(origin.x.saturating_add(u16::from(cell.x))) * CELL_SIZE;
            let top =
                BOARD_ORIGIN + f64::from(origin.y.saturating_add(u16::from(cell.y))) * CELL_SIZE;
            context.fill_rect(left + 5.0, top + 5.0, CELL_SIZE - 10.0, CELL_SIZE - 10.0);
            draw_shape_boundary(
                context,
                left,
                top,
                3.0,
                shape_boundary_edges(drag.shape, cell),
            );
        }
        context.restore();
    }

    fn draw_entity(
        &self,
        context: &CanvasRenderingContext2d,
        palette: &CanvasPalette,
        entity: &PlaceableEntity,
    ) -> Result<(), JsValue> {
        match entity.kind() {
            PlaceableEntityKind::Block(block) => {
                let color_index = block
                    .collect_layers()
                    .first()
                    .map_or(1, |layer| layer.color_index());
                let label_cell = entity.shape().occupied_cells().next();
                for cell in entity.shape().occupied_cells() {
                    let left =
                        BOARD_ORIGIN + f64::from(entity.origin().x + u16::from(cell.x)) * CELL_SIZE;
                    let top =
                        BOARD_ORIGIN + f64::from(entity.origin().y + u16::from(cell.y)) * CELL_SIZE;
                    let edges = shape_boundary_edges(entity.shape(), cell);
                    let x = left + if edges.left { 7.0 } else { 0.0 };
                    let y = top + if edges.top { 7.0 } else { 0.0 };
                    let width = CELL_SIZE
                        - if edges.left { 7.0 } else { 0.0 }
                        - if edges.right { 7.0 } else { 0.0 };
                    let height = CELL_SIZE
                        - if edges.top { 7.0 } else { 0.0 }
                        - if edges.bottom { 7.0 } else { 0.0 };
                    context.set_fill_style_str(block_color(color_index));
                    context.fill_rect(x, y, width, height);
                    context.set_stroke_style_str(palette.block_outline);
                    context.set_line_width(3.0);
                    draw_shape_boundary(context, left, top, 7.5, edges);
                    if label_cell == Some(cell) {
                        context.set_fill_style_str(palette.entity_label);
                        context.set_font(&self.ui_settings.canvas_font(12, true));
                        context.fill_text("B", left + CELL_SIZE / 2.0, top + CELL_SIZE / 2.0)?;
                    }
                }
            }
            PlaceableEntityKind::Blind(blind) => {
                let label_cell = entity.shape().occupied_cells().next();
                for (cell, tile) in entity.shape().occupied_cells().zip(blind.tiles()) {
                    let left =
                        BOARD_ORIGIN + f64::from(entity.origin().x + u16::from(cell.x)) * CELL_SIZE;
                    let top =
                        BOARD_ORIGIN + f64::from(entity.origin().y + u16::from(cell.y)) * CELL_SIZE;
                    let edges = shape_boundary_edges(entity.shape(), cell);
                    let geometry = pool_tile_geometry(edges, CELL_SIZE, BLIND_INSET);
                    let rect_x = left + geometry.x;
                    let rect_y = top + geometry.y;
                    let width = geometry.width;
                    let height = geometry.height;
                    context.set_fill_style_str(palette.blind_base);
                    context.fill_rect(rect_x, rect_y, width, height);

                    let resolution = tile.pixels_per_cell();
                    let pixel_width = width / f64::from(resolution);
                    let pixel_height = height / f64::from(resolution);
                    let mut painted = false;
                    for y in 0..resolution {
                        for x in 0..resolution {
                            let Some(color_index) = tile.color(x, y) else {
                                continue;
                            };
                            if color_index == 0 {
                                continue;
                            }
                            painted = true;
                            let canvas_y = resolution - 1 - y;
                            context.set_fill_style_str(blind_color(color_index));
                            context.fill_rect(
                                rect_x + f64::from(x) * pixel_width,
                                rect_y + f64::from(canvas_y) * pixel_height,
                                pixel_width + 0.25,
                                pixel_height + 0.25,
                            );
                        }
                    }
                    context.set_stroke_style_str(palette.blind_outline);
                    context.set_line_width(2.0);
                    draw_shape_boundary(context, left, top, BLIND_INSET + 0.5, edges);
                    if !painted {
                        context.set_stroke_style_str(palette.blind_detail);
                        context.set_line_width(1.0);
                        for offset in [20.0_f64, 38.0, 56.0] {
                            context.begin_path();
                            context.move_to(rect_x + 5.0, rect_y + offset.min(height - 5.0));
                            context.line_to(rect_x + offset.min(width - 5.0), rect_y + 5.0);
                            context.stroke();
                        }
                    }
                    if label_cell == Some(cell) {
                        context.set_fill_style_str(palette.entity_label);
                        context.set_font(&self.ui_settings.canvas_font(9, true));
                        context.fill_text("PL", left + CELL_SIZE / 2.0, top + CELL_SIZE / 2.0)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn draw_blind_gesture_preview(
        &self,
        context: &CanvasRenderingContext2d,
        palette: &CanvasPalette,
    ) {
        let Some(gesture) = self.blind_gesture.as_ref() else {
            return;
        };
        let Some(entity) = self.model.timeline().snapshot().entity(&gesture.entity_id) else {
            return;
        };
        let Some(blind) = entity.as_blind() else {
            return;
        };
        let resolution = u16::from(blind.pixels_per_cell());
        let preview_color = match gesture.operation {
            BlindGestureOperation::Paint { color_index }
            | BlindGestureOperation::FloodFill { color_index, .. } => blind_color(color_index),
            BlindGestureOperation::Erase => palette.blind_base,
        };
        context.set_fill_style_str(preview_color);
        for pixel in &gesture.pixels {
            let cell_x = pixel.x / resolution;
            let cell_y = pixel.y / resolution;
            let local_x = pixel.x % resolution;
            let local_y = pixel.y % resolution;
            let canvas_y = resolution - 1 - local_y;
            let (Ok(shape_x), Ok(shape_y)) = (u8::try_from(cell_x), u8::try_from(cell_y)) else {
                continue;
            };
            let cell = ShapeCell::new(shape_x, shape_y);
            let geometry = pool_tile_geometry(
                shape_boundary_edges(entity.shape(), cell),
                CELL_SIZE,
                BLIND_INSET,
            );
            let pixel_width = geometry.width / f64::from(resolution);
            let pixel_height = geometry.height / f64::from(resolution);
            let left = BOARD_ORIGIN
                + f64::from(entity.origin().x + cell_x) * CELL_SIZE
                + geometry.x
                + f64::from(local_x) * pixel_width;
            let top = BOARD_ORIGIN
                + f64::from(entity.origin().y + cell_y) * CELL_SIZE
                + geometry.y
                + f64::from(canvas_y) * pixel_height;
            context.fill_rect(left, top, pixel_width + 0.25, pixel_height + 0.25);
        }
    }

    fn draw_decorators(&self, context: &CanvasRenderingContext2d) {
        let snapshot = self.model.timeline().snapshot();
        context.save();
        context.set_text_align("center");
        context.set_text_baseline("middle");
        context.set_font(&self.ui_settings.canvas_font(10, true));
        for decorator in snapshot.decorators() {
            match decorator.kind() {
                DecoratorKind::Ice {
                    entity,
                    blocking_count,
                } if *blocking_count > 0 => {
                    let Some(owner) = snapshot.entity(entity) else {
                        continue;
                    };
                    let owner = self.preview_entity(owner);
                    context.set_stroke_style_str("#54caec");
                    context.set_line_width(2.0);
                    for cell in owner.shape().occupied_cells() {
                        let left = BOARD_ORIGIN
                            + f64::from(owner.origin().x + u16::from(cell.x)) * CELL_SIZE;
                        let top = BOARD_ORIGIN
                            + f64::from(owner.origin().y + u16::from(cell.y)) * CELL_SIZE;
                        context.stroke_rect(
                            left + 5.0,
                            top + 5.0,
                            CELL_SIZE - 10.0,
                            CELL_SIZE - 10.0,
                        );
                    }
                    if let Some(cell) = owner.shape().occupied_cells().next() {
                        let left = BOARD_ORIGIN
                            + f64::from(owner.origin().x + u16::from(cell.x)) * CELL_SIZE;
                        let top = BOARD_ORIGIN
                            + f64::from(owner.origin().y + u16::from(cell.y)) * CELL_SIZE;
                        context.set_fill_style_str("#102d39");
                        context.fill_rect(left + 9.0, top + 9.0, 24.0, 20.0);
                        context.set_stroke_style_str("#54caec");
                        context.stroke_rect(left + 9.5, top + 9.5, 23.0, 19.0);
                        context.set_fill_style_str("#bceffc");
                        let _ = context.fill_text(
                            &format!("I{blocking_count}"),
                            left + 21.0,
                            top + 19.0,
                        );
                    }
                }
                DecoratorKind::Glass {
                    entity,
                    blocking_count,
                } if *blocking_count > 0 => {
                    let Some(owner) = snapshot.entity(entity) else {
                        continue;
                    };
                    let owner = self.preview_entity(owner);
                    context.set_stroke_style_str("#a5e5f7");
                    context.set_line_width(2.0);
                    for cell in owner.shape().occupied_cells() {
                        let left = BOARD_ORIGIN
                            + f64::from(owner.origin().x + u16::from(cell.x)) * CELL_SIZE;
                        let top = BOARD_ORIGIN
                            + f64::from(owner.origin().y + u16::from(cell.y)) * CELL_SIZE;
                        context.stroke_rect(
                            left + 8.0,
                            top + 8.0,
                            CELL_SIZE - 16.0,
                            CELL_SIZE - 16.0,
                        );
                    }
                    if let Some(cell) = owner.shape().occupied_cells().next() {
                        let left = BOARD_ORIGIN
                            + f64::from(owner.origin().x + u16::from(cell.x)) * CELL_SIZE;
                        let top = BOARD_ORIGIN
                            + f64::from(owner.origin().y + u16::from(cell.y)) * CELL_SIZE;
                        context.set_fill_style_str("#14313e");
                        context.fill_rect(left + 9.0, top + 9.0, 24.0, 20.0);
                        context.set_stroke_style_str("#a5e5f7");
                        context.stroke_rect(left + 9.5, top + 9.5, 23.0, 19.0);
                        context.set_fill_style_str("#e0f8ff");
                        let _ = context.fill_text(
                            &format!("G{blocking_count}"),
                            left + 21.0,
                            top + 19.0,
                        );
                    }
                }
                DecoratorKind::Direction { entity, direction } => {
                    let Some(owner) = snapshot.entity(entity) else {
                        continue;
                    };
                    let owner = self.preview_entity(owner);
                    let Some(cell) = owner.shape().occupied_cells().next() else {
                        continue;
                    };
                    let left =
                        BOARD_ORIGIN + f64::from(owner.origin().x + u16::from(cell.x)) * CELL_SIZE;
                    let top =
                        BOARD_ORIGIN + f64::from(owner.origin().y + u16::from(cell.y)) * CELL_SIZE;
                    context.set_fill_style_str("#1d2831");
                    context.fill_rect(left + CELL_SIZE - 33.0, top + 9.0, 24.0, 20.0);
                    context.set_stroke_style_str("#f4f7fa");
                    context.stroke_rect(left + CELL_SIZE - 32.5, top + 9.5, 23.0, 19.0);
                    context.set_fill_style_str("#f4f7fa");
                    let label = match direction.mode() {
                        DirectionMode::Horizontal => "H",
                        DirectionMode::Vertical => "V",
                        DirectionMode::Disabled => continue,
                    };
                    let _ = context.fill_text(label, left + CELL_SIZE - 21.0, top + 19.0);
                }
                DecoratorKind::KeyLocker { entity, key } => {
                    let (Some(lock), Some(key_owner)) =
                        (snapshot.entity(entity), snapshot.entity(key))
                    else {
                        continue;
                    };
                    let lock = self.preview_entity(lock);
                    let key_owner = self.preview_entity(key_owner);
                    let (key_x, key_y) = entity_canvas_center(&key_owner);
                    let (lock_x, lock_y) = entity_canvas_center(&lock);
                    context.set_stroke_style_str("#e3b341");
                    context.set_line_width(2.0);
                    context.begin_path();
                    context.move_to(key_x, key_y);
                    context.line_to(lock_x, lock_y);
                    context.stroke();
                    draw_role_badge(context, key_x, key_y, "#4a3915", "#ffd15a", "K");
                    draw_role_badge(context, lock_x, lock_y, "#481c2a", "#ff7489", "L");
                    context.set_stroke_style_str("#ff7489");
                    for cell in lock.shape().occupied_cells() {
                        let left = BOARD_ORIGIN
                            + f64::from(lock.origin().x + u16::from(cell.x)) * CELL_SIZE;
                        let top = BOARD_ORIGIN
                            + f64::from(lock.origin().y + u16::from(cell.y)) * CELL_SIZE;
                        context.stroke_rect(
                            left + 11.0,
                            top + 11.0,
                            CELL_SIZE - 22.0,
                            CELL_SIZE - 22.0,
                        );
                    }
                }
                DecoratorKind::Ice { .. } | DecoratorKind::Glass { .. } => {}
            }
        }
        if let Some(key_entity_id) = self.key_locker_assignment.as_ref()
            && let Some(key) = snapshot.entity(key_entity_id)
        {
            let key = self.preview_entity(key);
            let (x, y) = entity_canvas_center(&key);
            context.set_stroke_style_str("#ffd15a");
            context.set_line_width(3.0);
            context.stroke_rect(x - 20.0, y - 20.0, 40.0, 40.0);
        }
        context.restore();
    }
}

fn entity_canvas_center(entity: &PlaceableEntity) -> (f64, f64) {
    let mut total_x = 0.0;
    let mut total_y = 0.0;
    let mut count = 0.0;
    for cell in entity.shape().occupied_cells() {
        total_x +=
            BOARD_ORIGIN + (f64::from(entity.origin().x + u16::from(cell.x)) + 0.5) * CELL_SIZE;
        total_y +=
            BOARD_ORIGIN + (f64::from(entity.origin().y + u16::from(cell.y)) + 0.5) * CELL_SIZE;
        count += 1.0;
    }
    (total_x / count, total_y / count)
}

fn draw_role_badge(
    context: &CanvasRenderingContext2d,
    x: f64,
    y: f64,
    background: &str,
    foreground: &str,
    label: &str,
) {
    context.set_fill_style_str(background);
    context.fill_rect(x - 11.0, y - 11.0, 22.0, 22.0);
    context.set_stroke_style_str(foreground);
    context.stroke_rect(x - 10.5, y - 10.5, 21.0, 21.0);
    context.set_fill_style_str(foreground);
    let _ = context.fill_text(label, x, y);
}

fn map_resize_edge_from_command(
    command: &LevelCommand,
    before: GridSize,
    after: GridSize,
) -> Option<MapResizeEdge> {
    let LevelCommand::ResizeGrid { anchor, .. } = command else {
        return None;
    };
    match (
        before.width() != after.width(),
        before.height() != after.height(),
        anchor,
    ) {
        (true, false, GridAnchor::Left) => Some(MapResizeEdge::Right),
        (true, false, GridAnchor::Right) => Some(MapResizeEdge::Left),
        (false, true, GridAnchor::Top) => Some(MapResizeEdge::Top),
        (false, true, GridAnchor::Bottom) => Some(MapResizeEdge::Bottom),
        _ => None,
    }
}

fn map_resize_edge_changes_only_axis(
    edge: MapResizeEdge,
    before: GridSize,
    after: GridSize,
) -> bool {
    match edge {
        MapResizeEdge::Top | MapResizeEdge::Bottom => before.width() == after.width(),
        MapResizeEdge::Right | MapResizeEdge::Left => before.height() == after.height(),
    }
}

fn template_drag_callback(
    ctx: &Context<App>,
    kind: PlacementKind,
    shape: Shape,
) -> Callback<DragEvent> {
    ctx.link().callback(move |event: DragEvent| {
        if let Some(data_transfer) = event.data_transfer() {
            data_transfer.set_effect_allowed("copy");
            let _ = data_transfer.set_data("text/plain", "oreak-entity-template");
        }
        Msg::BeginPlacementDrag(kind, shape)
    })
}

struct CanvasPalette {
    background: &'static str,
    guide: &'static str,
    floor: &'static str,
    floor_mark: &'static str,
    wall: &'static str,
    wall_detail: &'static str,
    grid: &'static str,
    label: &'static str,
    selection: &'static str,
    invalid: &'static str,
    block_outline: &'static str,
    blind_base: &'static str,
    blind_outline: &'static str,
    blind_detail: &'static str,
    entity_label: &'static str,
}

impl CanvasPalette {
    const fn for_theme(theme: Theme) -> Self {
        match theme {
            Theme::Dark => Self {
                background: "#0a0f14",
                guide: "#101922",
                floor: "#111a22",
                floor_mark: "#27343f",
                wall: "#315f5b",
                wall_detail: "#69e3c7",
                grid: "#344552",
                label: "#728492",
                selection: "#f2c94c",
                invalid: "#ff6b6b",
                block_outline: "#080c10",
                blind_base: "#302649",
                blind_outline: "#c89cff",
                blind_detail: "#8068a7",
                entity_label: "#ffffff",
            },
            Theme::Light => Self {
                background: "#dce3e8",
                guide: "#d3dbe0",
                floor: "#eef2f4",
                floor_mark: "#b8c5cc",
                wall: "#78b5aa",
                wall_detail: "#145c51",
                grid: "#8799a3",
                label: "#53636c",
                selection: "#9b6200",
                invalid: "#b52d3a",
                block_outline: "#3f2b27",
                blind_base: "#d7caea",
                blind_outline: "#67468c",
                blind_detail: "#9a82b6",
                entity_label: "#182329",
            },
        }
    }
}

fn draw_selection_cell(
    context: &CanvasRenderingContext2d,
    palette: &CanvasPalette,
    point: GridPoint,
) {
    let left = BOARD_ORIGIN + f64::from(point.x) * CELL_SIZE;
    let top = BOARD_ORIGIN + f64::from(point.y) * CELL_SIZE;
    context.set_stroke_style_str(palette.selection);
    context.set_line_width(3.0);
    context.stroke_rect(left + 2.0, top + 2.0, CELL_SIZE - 4.0, CELL_SIZE - 4.0);
    context.set_fill_style_str(palette.selection);
    context.fill_rect(left + 2.0, top + 2.0, 14.0, 3.0);
    context.fill_rect(left + 2.0, top + 2.0, 3.0, 14.0);
}

fn draw_shape_boundary(
    context: &CanvasRenderingContext2d,
    left: f64,
    top: f64,
    inset: f64,
    edges: ShapeBoundaryEdges,
) {
    let right = left + CELL_SIZE;
    let bottom = top + CELL_SIZE;
    context.begin_path();
    if edges.left {
        context.move_to(left + inset, top + if edges.top { inset } else { 0.0 });
        context.line_to(
            left + inset,
            bottom - if edges.bottom { inset } else { 0.0 },
        );
    }
    if edges.right {
        context.move_to(right - inset, top + if edges.top { inset } else { 0.0 });
        context.line_to(
            right - inset,
            bottom - if edges.bottom { inset } else { 0.0 },
        );
    }
    if edges.top {
        context.move_to(left + if edges.left { inset } else { 0.0 }, top + inset);
        context.line_to(right - if edges.right { inset } else { 0.0 }, top + inset);
    }
    if edges.bottom {
        context.move_to(left + if edges.left { inset } else { 0.0 }, bottom - inset);
        context.line_to(
            right - if edges.right { inset } else { 0.0 },
            bottom - inset,
        );
    }
    context.stroke();
}

fn entity_kind_label(kind: &PlaceableEntityKind) -> &'static str {
    match kind {
        PlaceableEntityKind::Block(_) => "Block",
        PlaceableEntityKind::Blind(_) => "Pool",
    }
}

fn view_entity_kind_details(entity: &PlaceableEntity, collapsed: bool, header: Html) -> Html {
    match entity.kind() {
        PlaceableEntityKind::Block(block) => {
            let colors = if block.collect_layers().is_empty() {
                "none".to_owned()
            } else {
                block
                    .collect_layers()
                    .iter()
                    .map(|layer| layer.color_index().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let locked = block
                .collect_layers()
                .iter()
                .filter(|layer| layer.is_locked())
                .count();
            html! {
                <section class="inspector-section">
                    { header }
                    {
                        if !collapsed {
                            html! {
                                <div id={InspectorSection::EntityData.content_id()} class="inspector-section-body">
                                    <div class="property-table">
                                        <div><span>{"Colors"}</span><code>{colors}</code></div>
                                        <div><span>{"Layers"}</span><strong>{block.collect_layers().len()}</strong></div>
                                        <div><span>{"Locked layers"}</span><strong>{locked}</strong></div>
                                    </div>
                                </div>
                            }
                        } else {
                            Html::default()
                        }
                    }
                </section>
            }
        }
        PlaceableEntityKind::Blind(blind) => {
            let painted = blind
                .tiles()
                .iter()
                .flat_map(|tile| tile.colors())
                .filter(|color| **color != 0)
                .count();
            let total = blind.tiles().len()
                * usize::from(blind.pixels_per_cell())
                * usize::from(blind.pixels_per_cell());
            let colors: BTreeSet<_> = blind
                .tiles()
                .iter()
                .flat_map(|tile| tile.colors().iter().copied())
                .filter(|color| *color != 0)
                .collect();
            let colors = if colors.is_empty() {
                "empty".to_owned()
            } else {
                colors
                    .into_iter()
                    .map(|color| color.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            html! {
                <section class="inspector-section">
                    { header }
                    {
                        if !collapsed {
                            html! {
                                <div id={InspectorSection::EntityData.content_id()} class="inspector-section-body">
                                    <div class="property-table">
                                        <div><span>{"Resolution"}</span><code>{format!("{} px/cell", blind.pixels_per_cell())}</code></div>
                                        <div><span>{"Tiles"}</span><strong>{blind.tiles().len()}</strong></div>
                                        <div><span>{"Painted"}</span><code>{format!("{painted} / {total} px")}</code></div>
                                        <div><span>{"Colors"}</span><code>{colors}</code></div>
                                    </div>
                                </div>
                            }
                        } else {
                            Html::default()
                        }
                    }
                </section>
            }
        }
    }
}

fn selection_status(selection: Option<&Selection>) -> String {
    match selection {
        Some(Selection::Cell(point)) => format!("CELL {:02}:{:02}", point.x, point.y),
        Some(Selection::Entities(entity_ids)) if entity_ids.len() == 1 => {
            format!("ENTITY {}", entity_ids[0])
        }
        Some(Selection::Entities(entity_ids)) => format!("GROUP {}", entity_ids.len()),
        None => "TARGET --".to_owned(),
    }
}

fn inspector_target_status(selection: Option<&Selection>) -> String {
    match selection {
        Some(Selection::Entities(entity_ids)) if entity_ids.len() == 1 => {
            format!("ENTITY {}", truncate_entity_id(&entity_ids[0]))
        }
        _ => selection_status(selection),
    }
}

fn truncate_entity_id(entity_id: &EntityId) -> String {
    const MAX_VISIBLE_CHARS: usize = 18;
    let value = entity_id.to_string();
    let char_count = value.chars().count();
    if char_count <= MAX_VISIBLE_CHARS {
        return value;
    }
    let prefix = value
        .chars()
        .take(MAX_VISIBLE_CHARS.saturating_sub(3))
        .collect::<String>();
    format!("{prefix}...")
}

fn shape_occupied_count(sample: &ShapeCatalogEntry) -> u32 {
    Shape::new(
        sample.shape.width,
        sample.shape.height,
        sample.shape.occupied_mask,
    )
    .map_or(0, |shape| shape.occupied_count())
}

fn shape_template_thumbnail(sample: &ShapeCatalogEntry) -> Html {
    let Ok(shape) = Shape::new(
        sample.shape.width,
        sample.shape.height,
        sample.shape.occupied_mask,
    ) else {
        return html! { <div class="shape-template-thumbnail invalid">{"INVALID"}</div> };
    };
    let width = shape.width();
    let height = shape.height();
    let cell_percent = 100.0 / f64::from(width.max(height));
    html! {
        <div
            class="shape-template-thumbnail"
            style={format!("--shape-template-columns: {width}; --shape-template-rows: {height}; --shape-template-cell: {cell_percent:.4}%;")}
            aria-hidden="true"
        >
            { for (0..height).flat_map(|y| (0..width).map(move |x| {
                let occupied = shape.contains(ShapeCell::new(x, y));
                html! { <span class={classes!(occupied.then_some("occupied"))}></span> }
            })) }
        </div>
    }
}

fn grid_anchor_label(anchor: GridAnchor) -> &'static str {
    match anchor {
        GridAnchor::TopLeft => "Top left",
        GridAnchor::Top => "Top",
        GridAnchor::TopRight => "Top right",
        GridAnchor::Left => "Left",
        GridAnchor::Center => "Center",
        GridAnchor::Right => "Right",
        GridAnchor::BottomLeft => "Bottom left",
        GridAnchor::Bottom => "Bottom",
        GridAnchor::BottomRight => "Bottom right",
    }
}

fn visual_anchor_to_grid(anchor: GridAnchor) -> GridAnchor {
    match anchor {
        GridAnchor::TopLeft => GridAnchor::BottomLeft,
        GridAnchor::Top => GridAnchor::Bottom,
        GridAnchor::TopRight => GridAnchor::BottomRight,
        GridAnchor::Left => GridAnchor::Left,
        GridAnchor::Center => GridAnchor::Center,
        GridAnchor::Right => GridAnchor::Right,
        GridAnchor::BottomLeft => GridAnchor::TopLeft,
        GridAnchor::Bottom => GridAnchor::Top,
        GridAnchor::BottomRight => GridAnchor::TopRight,
    }
}

fn collect_capacity_label(capacity: CollectCapacity) -> String {
    match capacity {
        CollectCapacity::Unlimited => "unlimited".to_owned(),
        CollectCapacity::Finite(capacity) => capacity.to_string(),
    }
}

fn block_color(color_index: u16) -> &'static str {
    match color_index {
        0 => "#80909c",
        1 => "#d97859",
        2 => "#e3b341",
        3 => "#62b97c",
        4 => "#4aa9c8",
        5 => "#607de0",
        6 => "#936bd4",
        7 => "#c662ae",
        8 => "#da5f72",
        9 => "#8d765f",
        10 => "#d5dce2",
        _ => "#596773",
    }
}

fn blind_color(color_index: u8) -> &'static str {
    color_index.checked_sub(1).and_then(|index|oreak_core::PALETTE_HEX.get(usize::from(index))).copied().unwrap_or("#000000")
}

fn spawn_bootstrap(ctx: &Context<App>) {
    let link = ctx.link().clone();
    wasm_bindgen_futures::spawn_local(async move {
        link.send_message(Msg::BootstrapFinished(RestClient.me().await));
    });
}

fn spawn_connection(ctx: &Context<App>, attempt: u64, target: ProjectLevelTarget) {
    let updates = ctx.link().callback(move |update| Msg::RpcUpdate {
        attempt,
        update: Box::new(update),
    });
    let link = ctx.link().clone();
    wasm_bindgen_futures::spawn_local(async move {
        link.send_message(Msg::RpcReady {
            attempt,
            result: RpcClient::connect_same_origin(target, updates).await,
        });
    });
}

fn browser_session_id() -> String {
    let mut bytes = [0_u8; 12];
    let generated = web_sys::window()
        .and_then(|window| window.crypto().ok())
        .is_some_and(|crypto| crypto.get_random_values_with_u8_array(&mut bytes).is_ok());
    if generated {
        return bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    }

    format!(
        "{:x}{:x}",
        Date::now() as u64,
        (js_sys::Math::random() * u64::MAX as f64) as u64
    )
}

fn actor_mark(actor: &str) -> String {
    actor
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(2)
        .collect::<String>()
        .to_ascii_uppercase()
}

fn presence_color(actor: &str) -> &'static str {
    let hash = actor.bytes().fold(0_u32, |hash, byte| {
        hash.wrapping_mul(31).wrapping_add(u32::from(byte))
    });
    ["#df8fff", "#69a8ff", "#f2b66d", "#65d6b4"][hash as usize % 4]
}

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

fn inspector_collapse_storage_key(user_id: &str) -> String {
    format!("{INSPECTOR_COLLAPSE_STORAGE_PREFIX}{user_id}")
}

fn load_inspector_collapsed_sections(user_id: &str) -> BTreeSet<InspectorSection> {
    storage()
        .and_then(|store| {
            store
                .get_item(&inspector_collapse_storage_key(user_id))
                .ok()
                .flatten()
        })
        .map(|value| parse_inspector_collapsed_sections(&value))
        .unwrap_or_default()
}

fn parse_inspector_collapsed_sections(value: &str) -> BTreeSet<InspectorSection> {
    if value.is_empty() {
        return BTreeSet::new();
    }
    value
        .split(',')
        .map(InspectorSection::from_storage_key)
        .collect::<Option<BTreeSet<_>>>()
        .unwrap_or_default()
}

fn persist_inspector_collapsed_sections(user_id: &str, sections: &BTreeSet<InspectorSection>) {
    let Some(store) = storage() else {
        return;
    };
    let value = sections
        .iter()
        .map(|section| section.storage_key())
        .collect::<Vec<_>>()
        .join(",");
    let _ = store.set_item(&inspector_collapse_storage_key(user_id), &value);
}

fn load_draft(key: &str) -> Option<LevelSnapshot> {
    let json = storage()?.get_item(key).ok().flatten()?;
    serde_json::from_str(&json).ok()
}

fn api_error_message(error: &ApiError) -> String {
    match error.retry_after_seconds {
        Some(seconds) => format!("{} Retry after {seconds}s.", error.message),
        None => error.message.clone(),
    }
}

fn now_ms() -> i64 {
    Date::now() as i64
}

fn format_time(milliseconds: i64) -> String {
    let date = Date::new(&JsValue::from_f64(milliseconds as f64));
    let iso = date.to_iso_string().as_string().unwrap_or_default();
    iso.get(11..19)
        .map(|time| format!("{time} UTC"))
        .unwrap_or_else(|| format!("{milliseconds} ms"))
}

fn cell_kind_label(kind: CellKind) -> &'static str {
    match kind {
        CellKind::Floor => "Floor",
        CellKind::Wall => "Wall",
    }
}

fn supported_image_media_type(file: &File) -> Option<&'static str> {
    supported_image_media_type_parts(&file.type_(), &file.name())
}

fn supported_image_media_type_parts(media_type: &str, file_name: &str) -> Option<&'static str> {
    match media_type.to_ascii_lowercase().as_str() {
        "image/png" => return Some("image/png"),
        "image/jpeg" | "image/jpg" => return Some("image/jpeg"),
        "image/webp" | "image/x-webp" => return Some("image/webp"),
        "" => {}
        _ => return None,
    }

    match file_name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => Some("image/png"),
        Some("jpg" | "jpeg") => Some("image/jpeg"),
        Some("webp") => Some("image/webp"),
        _ => None,
    }
}

fn event_target_is_text_entry(target: Option<web_sys::EventTarget>) -> bool {
    let Some(element) = target.and_then(|target| target.dyn_into::<Element>().ok()) else {
        return false;
    };
    matches!(element.tag_name().as_str(), "INPUT" | "TEXTAREA" | "SELECT")
        || element.get_attribute("contenteditable").as_deref() == Some("true")
}

fn view_history_event(event: &HistoryEvent) -> Html {
    let (title, detail) = match &event.command {
        LevelCommand::SetCell { point, kind } => (
            "Set cell",
            format!(
                "{:02}:{:02} -> {}",
                point.x,
                point.y,
                cell_kind_label(*kind)
            ),
        ),
        LevelCommand::SetCells { cells } => ("Stamp cells", format!("{} cells", cells.len())),
        LevelCommand::PlaceEntity { entity } => (
            "Place entity",
            format!(
                "{} @ {:02}:{:02}",
                entity.id(),
                entity.origin().x,
                entity.origin().y
            ),
        ),
        LevelCommand::PlaceEntities { entities } => {
            ("Duplicate entities", format!("{} entities", entities.len()))
        }
        LevelCommand::MoveEntity { entity_id, origin } => (
            "Move entity",
            format!("{entity_id} -> {:02}:{:02}", origin.x, origin.y),
        ),
        LevelCommand::MoveEntities { moves } => (
            "Move entities",
            format!("{} selected entities", moves.len()),
        ),
        LevelCommand::TransformEntities {
            transforms,
            entities,
        } => (
            "Transform entities",
            format!(
                "{} selected entities",
                if transforms.is_empty() {
                    entities.len()
                } else {
                    transforms.len()
                }
            ),
        ),
        LevelCommand::SetBlindResolutions {
            entity_ids,
            pixels_per_cell,
        } => (
            "Set Pool resolution",
            format!("{} Pools / {pixels_per_cell} px/cell", entity_ids.len()),
        ),
        LevelCommand::RotateEntityClockwise { entity_id } => {
            ("Rotate entity", entity_id.to_string())
        }
        LevelCommand::FlipEntityHorizontal { entity_id } => ("Flip entity", entity_id.to_string()),
        LevelCommand::SetBlockCollectLayers { entity_id, layers } => (
            "Set capacity layers",
            format!("{entity_id} / {} layers", layers.len()),
        ),
        LevelCommand::SetBlockLayerCapacity {
            entity_ids,
            layer_index,
            capacity,
        } => (
            "Set capacity",
            format!(
                "{} Blocks / layer {} / {}",
                entity_ids.len(),
                layer_index + 1,
                collect_capacity_label(*capacity)
            ),
        ),
        LevelCommand::DeleteEntity { entity_id } => ("Delete entity", entity_id.to_string()),
        LevelCommand::DeleteEntities { entity_ids } => (
            "Delete entities",
            format!("{} selected entities", entity_ids.len()),
        ),
        LevelCommand::ToggleIce { entity_id, .. } => ("Toggle Ice", entity_id.to_string()),
        LevelCommand::SetIce {
            entity_id,
            blocking_count,
            ..
        } => ("Set Ice", format!("{entity_id} / count {blocking_count}")),
        LevelCommand::ToggleGlass { entity_id, .. } => ("Toggle Glass", entity_id.to_string()),
        LevelCommand::SetGlass {
            entity_id,
            blocking_count,
            ..
        } => ("Set Glass", format!("{entity_id} / count {blocking_count}")),
        LevelCommand::CycleDirection { entity_id, .. } => {
            ("Cycle direction", entity_id.to_string())
        }
        LevelCommand::SetDirection {
            entity_id, mode, ..
        } => ("Set direction", format!("{entity_id} / {mode:?}")),
        LevelCommand::AssignKeyLocker {
            key_entity_id,
            lock_entity_id,
            ..
        } => (
            "Assign Key & Locker",
            format!("{key_entity_id} -> {lock_entity_id}"),
        ),
        LevelCommand::RemoveKeyLocker { decorator_id } => {
            ("Remove Key & Locker", decorator_id.to_string())
        }
        LevelCommand::PaintBlindStroke {
            entity_id,
            color_index,
            stroke,
        } => (
            "Paint Blind",
            format!(
                "{entity_id} / {} px / color {color_index}",
                stroke.pixels().len()
            ),
        ),
        LevelCommand::EraseBlindStroke { entity_id, stroke } => (
            "Erase Blind",
            format!("{entity_id} / {} px", stroke.pixels().len()),
        ),
        LevelCommand::FloodFillBlind {
            entity_id,
            start,
            color_index,
        } => (
            "Fill Blind",
            format!(
                "{entity_id} / {:02}:{:02} / color {color_index}",
                start.x, start.y
            ),
        ),
        LevelCommand::SetPoolDistributionGroups { entity_id, groups, .. } => ("Pool Division", format!("{} · {} groups", entity_id, groups.len())),
        LevelCommand::ApplyDistribution { request, .. } => ("Distribute", format!("{} Pools · {} layers", request.pools.len(), request.layers.len())),
        LevelCommand::RestoreEntity { entity } => (
            "Restore entity",
            format!(
                "{} @ {:02}:{:02}",
                entity.id(),
                entity.origin().x,
                entity.origin().y
            ),
        ),
        command => ("Level command", format!("{:?}", command.kind())),
    };
    let title = if event.reverts_sequence.is_some() {
        format!("Undo / {title}")
    } else {
        title.to_owned()
    };
    html! {
        <div class="event-row">
            <div class={classes!("event-node", event.reverts_sequence.is_some().then_some("undo"))}></div>
            <div class="event-copy">
                <div><strong>{title}</strong><code>{format!("#{:04}", event.sequence)}</code></div>
                <p>{detail}</p>
                <small>{format!("{} / {}", event.metadata.actor, format_time(event.metadata.occurred_at_ms))}</small>
            </div>
        </div>
    }
}

#[cfg(test)]
mod ui_settings_tests {
    use super::UiSettings;

    #[test]
    fn ui_settings_accept_only_supported_values_and_reset() {
        let mut settings = UiSettings::default();
        assert!(settings.set_font_size("2"));
        assert!(settings.set_scale("125"));
        assert!(!settings.set_font_size("8"));
        assert!(!settings.set_scale("175"));
        assert_eq!(settings.font_size_offset, 2);
        assert_eq!(settings.scale_percent, 125);
        assert!(settings.reset());
        assert_eq!(settings, UiSettings::default());
    }

    #[test]
    fn ui_scale_compensates_shell_dimensions() {
        let settings = UiSettings {
            font_size_offset: 1,
            scale_percent: 125,
        };
        let style = settings.shell_style(252.0, 296.0);
        assert!(style.contains("--left-sidebar-width: 201.60px"));
        assert!(style.contains("--right-sidebar-width: 236.80px"));
        assert!(style.contains("--canvas-min-width: 384.00px"));
        assert!(style.contains("--narrow-sidebar-width: 176.00px"));
        assert!(style.contains("--narrow-canvas-min-width: 368.00px"));
        assert!(style.contains("--ui-font-offset: 1px"));
        assert!(style.contains("width: 80.0000vw"));
        assert!(style.contains("height: 80.0000vh"));
        assert!(style.contains("--ui-scale: 1.25"));
        assert!(style.contains("zoom: var(--ui-scale)"));
        assert_eq!(
            settings.canvas_font(10, true),
            "bold 11px ui-monospace, SFMono-Regular, Consolas, monospace"
        );
    }
}

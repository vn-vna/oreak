use std::collections::{BTreeMap, BTreeSet};

use js_sys::Date;
use oreak_core::{
    BlameEntry, CellKind, GridPoint, HistoryEvent, LevelCommand, LevelSnapshot, LevelTarget,
};
use oreak_protocol::{
    ApplyCommandRequest, ApplyCommandResponse, ApplyCommandResult, LevelEvent,
    LevelSnapshotResponse, LevelSubscriptionItem, ProjectLevelTarget, UndoLatestRequest,
    UndoLatestResponse,
};
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{
    CanvasRenderingContext2d, Element, HtmlCanvasElement, HtmlElement, HtmlInputElement,
    KeyboardEvent, PointerEvent,
};
use yew::prelude::*;

use crate::api::{
    ApiError, LevelSummary, ProjectSummary, RestClient, UserSummary, WorkspaceSummary,
    draft_storage_key, projects_for_workspace,
};
use crate::model::{
    EditorModel, Mode, ModelChange, Shortcut, ShortcutScope, hydrate_history, resolve_shortcut,
};
use crate::rpc::{RpcClient, RpcUpdate};

const CANVAS_SIZE: u32 = 768;
const BOARD_ORIGIN: f64 = 44.0;
const CELL_SIZE: f64 = 85.0;
const THEME_STORAGE_KEY: &str = "oreak.theme.override";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Bootstrapping,
    Authentication,
    Catalog,
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
pub(crate) enum TopMenu {
    File,
    History,
    View,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RightTab {
    Activity,
    Blame,
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
            Self::Brush => "Switch mode: Brush (parity gated)",
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

pub struct App {
    phase: Phase,
    session_id: String,
    user: Option<UserSummary>,
    workspaces: Vec<WorkspaceSummary>,
    projects: Vec<ProjectSummary>,
    selected_workspace_id: String,
    catalog_project: Option<ProjectSummary>,
    levels: Vec<LevelSummary>,
    auth_mode: AuthMode,
    auth_email: String,
    auth_password: String,
    project_name: String,
    level_name: String,
    request_pending: bool,
    form_error: Option<String>,
    model: EditorModel,
    mode: Mode,
    selected: Option<GridPoint>,
    drag_kind: Option<CellKind>,
    last_drag: Option<GridPoint>,
    canvas_ref: NodeRef,
    root_ref: NodeRef,
    palette_input_ref: NodeRef,
    canvas_dirty: bool,
    theme: ThemeSettings,
    open_menu: Option<TopMenu>,
    right_tab: RightTab,
    palette_open: bool,
    palette_query: String,
    toasts: Vec<Toast>,
    next_toast_id: u32,
    target: ProjectLevelTarget,
    workspace_name: String,
    project_display_name: String,
    level_display_name: String,
    can_edit_timeline: bool,
    rpc: Option<RpcClient>,
    rpc_state: RpcState,
    connection_attempt: u64,
    server_sequence: Option<u64>,
    server_hash: Option<String>,
    server_events: Vec<HistoryEvent>,
    remote_blame: BTreeMap<GridPoint, BlameEntry>,
    seen_commands: BTreeSet<String>,
    pending_commands: BTreeSet<String>,
    pending_cells: BTreeMap<GridPoint, PendingCell>,
}

pub enum Msg {
    BootstrapFinished(Result<UserSummary, ApiError>),
    SetAuthMode(AuthMode),
    AuthEmail(String),
    AuthPassword(String),
    SubmitAuth,
    AuthFinished(Result<UserSummary, ApiError>),
    WorkspacesLoaded(Result<Vec<WorkspaceSummary>, ApiError>),
    ProjectsLoaded(Result<Vec<ProjectSummary>, ApiError>),
    SelectWorkspace(String),
    SelectProject(ProjectSummary),
    LevelsLoaded {
        project_id: String,
        result: Result<Vec<LevelSummary>, ApiError>,
    },
    ProjectName(String),
    CreateProject,
    ProjectCreated(Result<ProjectSummary, ApiError>),
    LevelName(String),
    CreateLevel,
    LevelCreated {
        project: ProjectSummary,
        result: Result<LevelSummary, ApiError>,
    },
    OpenLevel(LevelSummary),
    ShowCatalog,
    Logout,
    LogoutFinished(Result<(), ApiError>),
    SetMode(Mode),
    SetSelectedKind(CellKind),
    CanvasDown(PointerEvent),
    CanvasMove(PointerEvent),
    CanvasUp(PointerEvent),
    ClearSelection,
    KeyDown(KeyboardEvent),
    ToggleTheme,
    UseProjectTheme,
    Undo,
    ToggleMenu(TopMenu),
    SetRightTab(RightTab),
    TogglePalette,
    CloseOverlays,
    PaletteQuery(String),
    RunPalette(PaletteCommand),
    SaveDraft,
    ResetDraft,
    Reconnect,
    DismissToast(u32),
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
            levels: Vec::new(),
            auth_mode: AuthMode::Login,
            auth_email: String::new(),
            auth_password: String::new(),
            project_name: String::new(),
            level_name: String::new(),
            request_pending: false,
            form_error: None,
            model: EditorModel::blank("bootstrapping", &session_id),
            mode: Mode::Select,
            selected: None,
            drag_kind: None,
            last_drag: None,
            canvas_ref: NodeRef::default(),
            root_ref: NodeRef::default(),
            palette_input_ref: NodeRef::default(),
            canvas_dirty: true,
            theme: ThemeSettings::load(),
            open_menu: None,
            right_tab: RightTab::Activity,
            palette_open: false,
            palette_query: String::new(),
            toasts: Vec::new(),
            next_toast_id: 1,
            target: ProjectLevelTarget::new(String::new(), String::new()),
            workspace_name: String::new(),
            project_display_name: String::new(),
            level_display_name: String::new(),
            can_edit_timeline: false,
            rpc: None,
            rpc_state: RpcState::Offline,
            connection_attempt: 0,
            server_sequence: None,
            server_hash: None,
            server_events: Vec::new(),
            remote_blame: BTreeMap::new(),
            seen_commands: BTreeSet::new(),
            pending_commands: BTreeSet::new(),
            pending_cells: BTreeMap::new(),
        }
    }

    fn update(&mut self, ctx: &Context<Self>, msg: Self::Message) -> bool {
        match msg {
            Msg::BootstrapFinished(result) => {
                match result {
                    Ok(user) => self.enter_catalog(ctx, user),
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
                        self.enter_catalog(ctx, user);
                    }
                    Err(error) => {
                        self.form_error = Some(api_error_message(&error));
                    }
                }
                true
            }
            Msg::WorkspacesLoaded(result) => {
                match result {
                    Ok(workspaces) => {
                        self.workspaces = workspaces;
                        if self.selected_workspace_id.is_empty() {
                            self.selected_workspace_id = self
                                .user
                                .as_ref()
                                .map(|user| user.personal_workspace_id.clone())
                                .or_else(|| {
                                    self.workspaces
                                        .first()
                                        .map(|workspace| workspace.id.clone())
                                })
                                .unwrap_or_default();
                        }
                    }
                    Err(error) => return self.handle_authenticated_api_error(error),
                }
                self.request_pending = false;
                true
            }
            Msg::ProjectsLoaded(result) => {
                match result {
                    Ok(projects) => self.projects = projects,
                    Err(error) => return self.handle_authenticated_api_error(error),
                }
                self.request_pending = false;
                true
            }
            Msg::SelectWorkspace(workspace_id) => {
                self.selected_workspace_id = workspace_id;
                self.catalog_project = None;
                self.levels.clear();
                self.form_error = None;
                true
            }
            Msg::SelectProject(project) => {
                self.catalog_project = Some(project.clone());
                self.levels.clear();
                self.form_error = None;
                self.request_pending = true;
                let project_id = project.id.clone();
                let request_project_id = project_id.clone();
                let link = ctx.link().clone();
                wasm_bindgen_futures::spawn_local(async move {
                    link.send_message(Msg::LevelsLoaded {
                        project_id: request_project_id,
                        result: RestClient.list_levels(&project_id).await,
                    });
                });
                true
            }
            Msg::LevelsLoaded { project_id, result } => {
                if self
                    .catalog_project
                    .as_ref()
                    .map(|project| project.id.as_str())
                    != Some(project_id.as_str())
                {
                    return false;
                }
                self.request_pending = false;
                match result {
                    Ok(levels) => self.levels = levels,
                    Err(error) => return self.handle_authenticated_api_error(error),
                }
                true
            }
            Msg::ProjectName(value) => {
                self.project_name = value;
                true
            }
            Msg::CreateProject => {
                if self.request_pending || self.project_name.trim().is_empty() {
                    return false;
                }
                self.request_pending = true;
                self.form_error = None;
                let workspace_id = self.selected_workspace_id.clone();
                let name = self.project_name.trim().to_owned();
                let link = ctx.link().clone();
                wasm_bindgen_futures::spawn_local(async move {
                    link.send_message(Msg::ProjectCreated(
                        RestClient.create_project(&workspace_id, &name).await,
                    ));
                });
                true
            }
            Msg::ProjectCreated(result) => {
                self.request_pending = false;
                match result {
                    Ok(project) => {
                        self.project_name.clear();
                        self.projects.push(project.clone());
                        self.catalog_project = Some(project);
                        self.levels.clear();
                    }
                    Err(error) => return self.handle_authenticated_api_error(error),
                }
                true
            }
            Msg::LevelName(value) => {
                self.level_name = value;
                true
            }
            Msg::CreateLevel => {
                let Some(project) = self.catalog_project.clone() else {
                    return false;
                };
                if self.request_pending
                    || self.level_name.trim().is_empty()
                    || !project.can_edit_timeline()
                {
                    return false;
                }
                self.request_pending = true;
                self.form_error = None;
                let name = self.level_name.trim().to_owned();
                let project_id = project.id.clone();
                let link = ctx.link().clone();
                wasm_bindgen_futures::spawn_local(async move {
                    link.send_message(Msg::LevelCreated {
                        project,
                        result: RestClient.create_level(&project_id, &name).await,
                    });
                });
                true
            }
            Msg::LevelCreated { project, result } => {
                self.request_pending = false;
                match result {
                    Ok(level) => {
                        self.level_name.clear();
                        self.levels.push(level.clone());
                        self.open_level(ctx, project, level);
                    }
                    Err(error) => return self.handle_authenticated_api_error(error),
                }
                true
            }
            Msg::OpenLevel(level) => {
                let Some(project) = self.catalog_project.clone() else {
                    return false;
                };
                self.open_level(ctx, project, level);
                true
            }
            Msg::ShowCatalog => {
                self.invalidate_rpc();
                self.phase = Phase::Catalog;
                self.open_menu = None;
                self.palette_open = false;
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
            Msg::SetMode(mode) => self.set_mode(mode),
            Msg::SetSelectedKind(kind) => {
                let Some(point) = self.selected else {
                    return false;
                };
                self.apply_cell(ctx, point, kind)
            }
            Msg::CanvasDown(event) => self.canvas_down(ctx, event),
            Msg::CanvasMove(event) => self.canvas_move(ctx, event),
            Msg::CanvasUp(event) => {
                if let Some(canvas) = self.canvas_ref.cast::<HtmlCanvasElement>() {
                    let _ = canvas.release_pointer_capture(event.pointer_id());
                }
                self.drag_kind = None;
                self.last_drag = None;
                false
            }
            Msg::ClearSelection => {
                self.selected = None;
                self.canvas_dirty = true;
                true
            }
            Msg::KeyDown(event) => self.key_down(ctx, event),
            Msg::ToggleTheme => {
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
            Msg::UseProjectTheme => {
                self.theme.use_project_default();
                self.canvas_dirty = true;
                self.open_menu = None;
                self.push_toast("Using project-default theme".to_owned(), "info");
                true
            }
            Msg::Undo => self.undo(ctx),
            Msg::ToggleMenu(menu) => {
                self.open_menu = (self.open_menu != Some(menu)).then_some(menu);
                self.palette_open = false;
                true
            }
            Msg::SetRightTab(tab) => {
                self.right_tab = tab;
                true
            }
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
                if !self.can_edit_timeline {
                    self.open_menu = None;
                    self.push_toast(
                        "Reset requires the exact edit_timeline capability".to_owned(),
                        "warning",
                    );
                    return true;
                }
                if !matches!(self.rpc_state, RpcState::Offline) {
                    self.open_menu = None;
                    self.push_toast(
                        "Reset is local-only and disabled during collaboration".to_owned(),
                        "warning",
                    );
                    return true;
                }
                let snapshot = LevelSnapshot::new(8, 8).expect("the fixed editor grid is valid");
                self.model
                    .replace_snapshot(snapshot)
                    .expect("the blank snapshot is valid");
                self.selected = None;
                self.canvas_dirty = true;
                self.open_menu = None;
                self.persist_draft();
                self.push_toast("Local draft reset to 8 x 8 floor".to_owned(), "warning");
                true
            }
            Msg::Reconnect => {
                self.start_connection(ctx);
                true
            }
            Msg::DismissToast(id) => {
                self.toasts.retain(|toast| toast.id != id);
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
                        self.push_toast(
                            format!("Collaboration connected for {}", self.level_display_name),
                            "success",
                        );
                    }
                    Err(error) => {
                        self.rpc = None;
                        self.rpc_state = RpcState::Offline;
                        self.push_toast(
                            format!("RPC connection failed; using local draft: {error}"),
                            "warning",
                        );
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
        }
    }

    fn rendered(&mut self, _ctx: &Context<Self>, first_render: bool) {
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
        if self.phase == Phase::Editor && self.canvas_dirty {
            let _ = self.draw_canvas();
            self.canvas_dirty = false;
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
                            Phase::Catalog => self.view_catalog(ctx),
                            Phase::Editor => Html::default(),
                        }
                    }
                </div>
            };
        }
        html! {
            <div
                class="app-shell"
                data-theme={active_theme.name()}
                tabindex="0"
                ref={self.root_ref.clone()}
                onkeydown={ctx.link().callback(Msg::KeyDown)}
            >
                { self.view_header(ctx) }
                { self.view_mode_rail(ctx) }
                { self.view_inspector(ctx) }
                { self.view_canvas(ctx) }
                { self.view_activity(ctx) }
                { self.view_status() }
                { self.view_palette(ctx) }
                { self.view_toasts(ctx) }
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

    fn view_catalog(&self, ctx: &Context<Self>) -> Html {
        let projects = projects_for_workspace(&self.projects, &self.selected_workspace_id);
        let email = self
            .user
            .as_ref()
            .map(|user| user.email.as_str())
            .unwrap_or_default();
        html! {
            <main class="catalog-stage">
                <header class="catalog-header">
                    <div class="access-brand"><span class="mark-glyph">{"OR"}</span><strong>{"OREAK"}</strong><small>{"PROJECT CATALOG"}</small></div>
                    <div class="catalog-account"><span>{email}</span><button disabled={self.request_pending} onclick={ctx.link().callback(|_| Msg::Logout)}>{"Logout"}</button></div>
                </header>
                <aside class="workspace-index">
                    <div class="catalog-label"><span>{"WORKSPACES"}</span><code>{format!("{:02}", self.workspaces.len())}</code></div>
                    { for self.workspaces.iter().map(|workspace| {
                        let id = workspace.id.clone();
                        html! {
                            <button class={if self.selected_workspace_id == workspace.id { "workspace-row active" } else { "workspace-row" }} onclick={ctx.link().callback(move |_| Msg::SelectWorkspace(id.clone()))}>
                                <span>{workspace.name.clone()}</span><small>{format!("{} / {} / {} PRJ", workspace.kind, workspace.role, workspace.project_count)}</small>
                            </button>
                        }
                    }) }
                </aside>
                <section class="project-index">
                    <div class="catalog-label"><span>{"PROJECT INDEX"}</span><code>{format!("{:02}", projects.len())}</code></div>
                    <div class="project-grid">
                        { for projects.into_iter().map(|project| {
                            let selected = self.catalog_project.as_ref().is_some_and(|current| current.id == project.id);
                            let project_message = project.clone();
                            html! {
                                <button class={if selected { "project-card active" } else { "project-card" }} onclick={ctx.link().callback(move |_| Msg::SelectProject(project_message.clone()))}>
                                    <span class="project-code">{project.id.clone()}</span>
                                    <strong>{project.name.clone()}</strong>
                                    <small>{format!("{} LEVELS / MANIFEST V{}", project.level_count, project.plugin_manifest_format_version)}</small>
                                    <span class={if project.can_edit_timeline() { "capability edit" } else { "capability view" }}>{if project.can_edit_timeline() { "EDIT TIMELINE" } else { "VIEW ONLY" }}</span>
                                </button>
                            }
                        }) }
                    </div>
                    <div class="catalog-create">
                        <label class="technical-field"><span>{"NEW PROJECT / SELECTED WORKSPACE"}</span><input value={self.project_name.clone()} placeholder="Project name" oninput={ctx.link().callback(|event: InputEvent| Msg::ProjectName(event.target_unchecked_into::<HtmlInputElement>().value()))} /></label>
                        <button class="primary-action compact" disabled={self.request_pending || self.selected_workspace_id.is_empty() || self.project_name.trim().is_empty()} onclick={ctx.link().callback(|_| Msg::CreateProject)}><span>{"Create project"}</span><code>{"+P"}</code></button>
                    </div>
                    { self.form_error.as_ref().map(|error| html! { <div class="access-error catalog-error"><strong>{"API ERROR"}</strong><span>{error}</span></div> }).unwrap_or_default() }
                </section>
                <aside class="level-index">
                    <div class="catalog-label"><span>{"LEVELS"}</span><code>{format!("{:02}", self.levels.len())}</code></div>
                    {
                        if let Some(project) = &self.catalog_project {
                            html! {
                                <>
                                    <div class="level-project"><small>{"PROJECT"}</small><strong>{project.name.clone()}</strong><code>{project.id.clone()}</code></div>
                                    <div class="level-list">
                                        { for self.levels.iter().map(|level| {
                                            let level_message = level.clone();
                                            html! { <button onclick={ctx.link().callback(move |_| Msg::OpenLevel(level_message.clone()))}><span><strong>{level.name.clone()}</strong><small>{format!("{} REVISIONS", level.revision_count)}</small></span><code>{"OPEN >"}</code></button> }
                                        }) }
                                    </div>
                                    <div class="level-create">
                                        <label class="technical-field"><span>{"NEW LEVEL"}</span><input value={self.level_name.clone()} placeholder="Level name" disabled={!project.can_edit_timeline()} oninput={ctx.link().callback(|event: InputEvent| Msg::LevelName(event.target_unchecked_into::<HtmlInputElement>().value()))} /></label>
                                        <button class="primary-action compact" disabled={self.request_pending || !project.can_edit_timeline() || self.level_name.trim().is_empty()} onclick={ctx.link().callback(|_| Msg::CreateLevel)}><span>{"Create and open"}</span><code>{"+L"}</code></button>
                                    </div>
                                </>
                            }
                        } else {
                            html! { <div class="catalog-empty"><span>{"NO PROJECT TARGET"}</span><p>{"Select a project to inspect or create levels."}</p></div> }
                        }
                    }
                </aside>
            </main>
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
                    <button class="context-part" title="Switch workspace">
                        <small>{"WORKSPACE"}</small><span>{self.workspace_name.clone()}</span>
                    </button>
                    <span class="context-slash">{"/"}</span>
                    <button class="context-part" title="Switch project">
                        <small>{"PROJECT"}</small><span>{self.project_display_name.clone()}</span>
                    </button>
                    <span class="context-slash">{"/"}</span>
                    <button class="context-part context-level" title="Switch level">
                        <small>{"LEVEL"}</small><span>{self.level_display_name.clone()}</span>
                    </button>
                </div>
                <div class="header-tools">
                    <button class="header-nav" onclick={ctx.link().callback(|_| Msg::ShowCatalog)}>{"Catalog"}</button>
                    <button
                        class="icon-button command-trigger"
                        onclick={ctx.link().callback(|_| Msg::TogglePalette)}
                        title="Open command palette (Ctrl+K)"
                    >
                        <span>{"Command"}</span><kbd>{"Ctrl K"}</kbd>
                    </button>
                    <div class="presence-stack presence-unknown" aria-label="Collaborator presence unavailable">
                        <span class="presence-protocol">{"--"}</span>
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
                    </>
                },
            ),
        };
        html! { <div class={classes!("top-menu-popover", class)}>{body}</div> }
    }

    fn view_mode_rail(&self, ctx: &Context<Self>) -> Html {
        html! {
            <nav class="mode-rail" aria-label="Editor modes">
                <div class="mode-rail-label">{"MODE"}</div>
                { html! { for mode in Mode::ALL {
                    <button
                        key={mode.label()}
                        class={if self.mode == mode { "mode-button active" } else { "mode-button" }}
                        aria-pressed={(self.mode == mode).to_string()}
                        title={mode.description()}
                        onclick={ctx.link().callback(move |_| Msg::SetMode(mode))}
                    >
                        <kbd>{mode.key()}</kbd>
                        <span>{mode.label()}</span>
                    </button>
                } } }
                <div class="rail-spacer"></div>
                <button class="rail-tool" title="Grid snapping enabled">{"#"}</button>
                <button class="rail-tool" title="Guides visible">{"+"}</button>
            </nav>
        }
    }

    fn view_inspector(&self, ctx: &Context<Self>) -> Html {
        let selected_kind = self.selected.and_then(|point| self.effective_cell(point));
        html! {
            <aside class="inspector panel">
                <div class="panel-heading">
                    <div><small>{"INSPECTOR"}</small><strong>{self.mode.label()}</strong></div>
                    <span class="panel-code">{"I-01"}</span>
                </div>
                <section class={classes!("inspector-section", "mode-summary", self.mode.is_parity_gated().then_some("parity-gated"))}>
                    <span class="section-rule"></span>
                    <p>{self.mode.description()}</p>
                </section>
                <section class="inspector-section">
                    <h2>{"Selection"}</h2>
                    {
                        if let Some(point) = self.selected {
                            html! {
                                <div class="property-table">
                                    <div><span>{"Target"}</span><strong>{"Logical cell"}</strong></div>
                                    <div><span>{"Coordinate"}</span><code>{format!("x{:02} / y{:02}", point.x, point.y)}</code></div>
                                    <div><span>{"Kind"}</span><strong>{cell_kind_label(selected_kind.unwrap_or(CellKind::Floor))}</strong></div>
                                    <div><span>{"Index"}</span><code>{format!("{:03}", point.x + point.y * 8)}</code></div>
                                </div>
                            }
                        } else {
                            html! { <div class="empty-selection"><span>{"NO TARGET"}</span><p>{"Select a cell on the canvas."}</p></div> }
                        }
                    }
                </section>
                <section class="inspector-section">
                    <h2>{"Cell material"}</h2>
                    <div class="segmented-control">
                        <button
                            class={if selected_kind == Some(CellKind::Floor) { "active" } else { "" }}
                            disabled={self.selected.is_none() || self.mode != Mode::Map || !self.can_edit_timeline}
                            onclick={ctx.link().callback(|_| Msg::SetSelectedKind(CellKind::Floor))}
                        ><span class="swatch floor"></span>{"Floor"}</button>
                        <button
                            class={if selected_kind == Some(CellKind::Wall) { "active" } else { "" }}
                            disabled={self.selected.is_none() || self.mode != Mode::Map || !self.can_edit_timeline}
                            onclick={ctx.link().callback(|_| Msg::SetSelectedKind(CellKind::Wall))}
                        ><span class="swatch wall"></span>{"Wall"}</button>
                    </div>
                </section>
                <section class="inspector-section scene-tree">
                    <h2>{"Level structure"}</h2>
                    <button class="tree-row expanded"><span>{"v"}</span><strong>{self.level_display_name.clone()}</strong><code>{"ROOT"}</code></button>
                    <button class="tree-row child active"><span>{"#"}</span><strong>{"Logical map"}</strong><code>{"8x8"}</code></button>
                    <button class="tree-row child muted"><span>{"*"}</span><strong>{"Brush parity"}</strong><code>{"GATED"}</code></button>
                    <button class="tree-row child muted"><span>{"@"}</span><strong>{"Sandbox parity"}</strong><code>{"GATED"}</code></button>
                </section>
                <section class="inspector-section project-theme">
                    <h2>{"Theme source"}</h2>
                    <div class="theme-source-line">
                        <span class="source-dot"></span>
                        <div>
                            <strong>{self.theme.active().label()}</strong>
                            <small>{if self.theme.user_override.is_some() { "User override" } else { "Project default" }}</small>
                        </div>
                        <button onclick={ctx.link().callback(|_| Msg::UseProjectTheme)}>{"Reset"}</button>
                    </div>
                </section>
            </aside>
        }
    }

    fn view_canvas(&self, ctx: &Context<Self>) -> Html {
        let blame = self.selected.and_then(|point| self.blame_for_cell(point));
        let connection_label = match self.rpc_state {
            RpcState::Online => "SERVER LIVE",
            RpcState::Connecting => "CONNECTING",
            RpcState::Resyncing => "RESYNCING",
            RpcState::Offline => "LOCAL DRAFT",
        };
        html! {
            <main class="workbench">
                <div class="canvas-toolbar">
                    <div class="tool-group">
                        <button class="tool-button active">{format!("{} tool", self.mode.label())}</button>
                        <span class="tool-divider"></span>
                        <button class="tool-button">{"Grid 8 x 8"}</button>
                        <button class="tool-button">{"Snap 1.0"}</button>
                    </div>
                    <div class="canvas-readout">
                        <span><i class={classes!("status-dot", if matches!(self.rpc_state, RpcState::Online) { "online" } else { "local" })}></i>{connection_label}</span>
                        <code>{format!("{} EV", self.activity_events().len())}</code>
                        <code>{format!("{} PENDING", self.pending_commands.len())}</code>
                        <span>{"100%"}</span>
                    </div>
                </div>
                <div class="canvas-stage">
                    <div class="axis-label axis-y">{"Y / ROW"}</div>
                    <canvas
                        ref={self.canvas_ref.clone()}
                        width={CANVAS_SIZE.to_string()}
                        height={CANVAS_SIZE.to_string()}
                        aria-label="8 by 8 editable logical map"
                        onpointerdown={ctx.link().callback(Msg::CanvasDown)}
                        onpointermove={ctx.link().callback(Msg::CanvasMove)}
                        onpointerup={ctx.link().callback(Msg::CanvasUp)}
                        onpointercancel={ctx.link().callback(Msg::CanvasUp)}
                    ></canvas>
                    <div class="axis-label axis-x">{"X / COLUMN"}</div>
                    {
                        if self.mode.is_parity_gated() {
                            html! { <div class="parity-banner"><strong>{format!("{} PARITY GATED", self.mode.label().to_ascii_uppercase())}</strong><span>{"Visible for source-editor mode parity; commands are disabled in this MVP."}</span></div> }
                        } else {
                            Html::default()
                        }
                    }
                    { self.view_blame_popover(ctx, blame) }
                </div>
            </main>
        }
    }

    fn view_blame_popover(&self, ctx: &Context<Self>, blame: Option<BlameEntry>) -> Html {
        let Some(point) = self.selected else {
            return Html::default();
        };
        html! {
            <div class="blame-popover" role="dialog" aria-label="Cell blame">
                <div class="popover-head">
                    <span>{"CELL BLAME"}</span>
                    <code>{format!("{:02}:{:02}", point.x, point.y)}</code>
                    <button aria-label="Close blame" onclick={ctx.link().callback(|_| Msg::ClearSelection)}>{"x"}</button>
                </div>
                {
                    if let Some(entry) = blame {
                        html! {
                            <div class="popover-body">
                                <span class="actor-mark">{actor_mark(entry.actor.as_str())}</span>
                                <div><strong>{entry.actor.to_string()}</strong><small>{format_time(entry.occurred_at_ms)}</small></div>
                                <div class="sequence"><small>{"SEQUENCE"}</small><strong>{format!("#{:04}", entry.sequence)}</strong></div>
                            </div>
                        }
                    } else {
                        html! {
                            <div class="popover-empty"><strong>{"No observed provenance"}</strong><small>{if matches!(self.rpc_state, RpcState::Online | RpcState::Resyncing) { "Base cell with no recorded change" } else { "Base or restored local draft cell" }}</small></div>
                        }
                    }
                }
            </div>
        }
    }

    fn view_activity(&self, ctx: &Context<Self>) -> Html {
        let selected_blame = self.selected.and_then(|point| self.blame_for_cell(point));
        html! {
            <aside class="activity panel">
                <div class="panel-tabs">
                    <button
                        class={if self.right_tab == RightTab::Activity { "active" } else { "" }}
                        onclick={ctx.link().callback(|_| Msg::SetRightTab(RightTab::Activity))}
                    >{"Activity"}</button>
                    <button
                        class={if self.right_tab == RightTab::Blame { "active" } else { "" }}
                        onclick={ctx.link().callback(|_| Msg::SetRightTab(RightTab::Blame))}
                    >{"Blame"}</button>
                </div>
                {
                    match self.right_tab {
                        RightTab::Activity => self.view_activity_feed(),
                        RightTab::Blame => self.view_blame_panel(selected_blame),
                    }
                }
                <section class="collaborators">
                    <div class="section-title"><span>{"PRESENCE"}</span><code>{"UNAVAILABLE"}</code></div>
                    <div class="presence-unavailable"><strong>{"No roster in protocol"}</strong><small>{"Level events include actor IDs; online collaborator counts are not exposed yet."}</small></div>
                </section>
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
                            html! { <div class="empty-feed"><span>{"NO LOCAL EVENTS"}</span><p>{"Paint a map cell to start the timeline."}</p></div> }
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
                    if let (Some(point), Some(entry)) = (self.selected, blame) {
                        let history = self.history_for_cell(point);
                        html! {
                            <>
                                <div class="blame-hero">
                                <span class="actor-mark large">{actor_mark(entry.actor.as_str())}</span>
                                    <div><small>{"CURRENT ACTOR"}</small><strong>{entry.actor.to_string()}</strong><span>{format_time(entry.occurred_at_ms)}</span></div>
                                </div>
                                <div class="property-table right-properties">
                                    <div><span>{"Cell"}</span><code>{format!("{:02}:{:02}", point.x, point.y)}</code></div>
                                    <div><span>{"Sequence"}</span><strong>{format!("#{:04}", entry.sequence)}</strong></div>
                                    <div><span>{"Command"}</span><code>{entry.command_id.to_string()}</code></div>
                                    <div><span>{"Reverts"}</span><strong>{entry.reverts_sequence.map(|seq| format!("#{seq:04}")).unwrap_or_else(|| "none".to_owned())}</strong></div>
                                </div>
                                <div class="section-title"><span>{"CELL HISTORY"}</span><code>{history.len()}</code></div>
                                <div class="event-list">{ html! { for event in history.into_iter().rev() {
                                    {view_history_event(event)}
                                } } }</div>
                            </>
                        }
                    } else {
                        html! { <div class="empty-feed"><span>{"NO PROVENANCE"}</span><p>{"Select a changed cell to inspect actor, time, and sequence."}</p></div> }
                    }
                }
            </div>
        }
    }

    fn view_status(&self) -> Html {
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
            RpcState::Offline => "OFFLINE / LOCAL",
        };
        let endpoint = self
            .rpc
            .as_ref()
            .map(RpcClient::endpoint)
            .unwrap_or("ws(s)://host/rpc");
        html! {
            <footer class="statusbar">
                <span class="status-primary"><i class={classes!("status-dot", if matches!(self.rpc_state, RpcState::Online) { "online" } else { "local" })}></i>{rpc_label}</span>
                <span title={endpoint.to_owned()}>{format!("{} / {}", self.project_display_name, self.level_display_name)}</span>
                <span>{format!("MODE {}", self.mode.key())}</span>
                <span>{self.selected.map(|point| format!("CELL {:02}:{:02}", point.x, point.y)).unwrap_or_else(|| "CELL --:--".to_owned())}</span>
                <span>{self.server_sequence.map(|sequence| if matches!(self.rpc_state, RpcState::Online | RpcState::Resyncing) { format!("SEQ #{sequence:04}") } else { format!("LAST #{sequence:04}") }).unwrap_or_else(|| "SEQ LOCAL".to_owned())}</span>
                <span class="status-hash">{format!("HASH {}", &hash[..12.min(hash.len())])}</span>
                <span title={self.model.actor().to_string()}>{format!("ACTOR {}", actor_mark(self.model.actor().as_str()))}</span>
                <span class="status-spacer"></span>
                <span>{"Q/W/B/P modes"}</span>
                <span>{"Ctrl+K commands"}</span>
                <span>{"Ctrl+Z my undo"}</span>
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
        html! {
            <div class="toast-stack" aria-live="polite">
                { html! { for toast in &self.toasts {
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

    fn enter_catalog(&mut self, ctx: &Context<Self>, user: UserSummary) {
        self.invalidate_rpc();
        self.selected_workspace_id = user.personal_workspace_id.clone();
        self.user = Some(user);
        self.phase = Phase::Catalog;
        self.workspaces.clear();
        self.projects.clear();
        self.catalog_project = None;
        self.levels.clear();
        self.form_error = None;
        self.request_pending = true;

        let workspace_link = ctx.link().clone();
        wasm_bindgen_futures::spawn_local(async move {
            workspace_link.send_message(Msg::WorkspacesLoaded(RestClient.list_workspaces().await));
        });
        let project_link = ctx.link().clone();
        wasm_bindgen_futures::spawn_local(async move {
            project_link.send_message(Msg::ProjectsLoaded(RestClient.list_projects().await));
        });
    }

    fn open_level(&mut self, ctx: &Context<Self>, project: ProjectSummary, level: LevelSummary) {
        let Some(user) = self.user.as_ref() else {
            self.clear_session();
            return;
        };
        let key = draft_storage_key(&user.id, &project.id, &level.id);
        self.model = load_draft(&key)
            .map(|snapshot| EditorModel::from_snapshot(snapshot, &user.id, &self.session_id))
            .unwrap_or_else(|| EditorModel::blank(&user.id, &self.session_id));
        self.workspace_name = self
            .workspaces
            .iter()
            .find(|workspace| workspace.id == project.workspace_id)
            .map(|workspace| workspace.name.clone())
            .unwrap_or_else(|| project.workspace_id.clone());
        self.project_display_name = project.name.clone();
        self.level_display_name = level.name.clone();
        self.target = ProjectLevelTarget::new(project.id.clone(), level.id.clone());
        self.can_edit_timeline = project.can_edit_timeline();
        self.catalog_project = Some(project);
        self.phase = Phase::Editor;
        self.selected = None;
        self.server_events.clear();
        self.remote_blame.clear();
        self.seen_commands.clear();
        self.server_sequence = None;
        self.server_hash = None;
        self.canvas_dirty = true;
        self.start_connection(ctx);
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
        self.invalidate_rpc();
        self.phase = Phase::Authentication;
        self.user = None;
        self.workspaces.clear();
        self.projects.clear();
        self.catalog_project = None;
        self.levels.clear();
        self.target = ProjectLevelTarget::new(String::new(), String::new());
        self.workspace_name.clear();
        self.project_display_name.clear();
        self.level_display_name.clear();
        self.can_edit_timeline = false;
        self.auth_password.clear();
    }

    fn invalidate_rpc(&mut self) {
        self.connection_attempt += 1;
        self.rpc = None;
        self.rpc_state = RpcState::Offline;
        self.pending_commands.clear();
        self.pending_cells.clear();
        self.server_sequence = None;
        self.server_hash = None;
        self.server_events.clear();
        self.remote_blame.clear();
        self.seen_commands.clear();
    }

    fn set_mode(&mut self, mode: Mode) -> bool {
        if self.mode == mode {
            return false;
        }
        self.mode = mode;
        self.drag_kind = None;
        if mode == Mode::Select {
            self.right_tab = RightTab::Blame;
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

    fn canvas_down(&mut self, ctx: &Context<Self>, event: PointerEvent) -> bool {
        event.prevent_default();
        let Some(point) = self.point_from_pointer(&event) else {
            return false;
        };
        if let Some(canvas) = self.canvas_ref.cast::<HtmlCanvasElement>() {
            let _ = canvas.set_pointer_capture(event.pointer_id());
        }
        self.selected = Some(point);
        self.last_drag = Some(point);
        self.canvas_dirty = true;

        if self.mode != Mode::Map {
            self.drag_kind = None;
            return true;
        }

        let Some(current) = self.effective_cell(point) else {
            return true;
        };
        let kind = match current {
            CellKind::Floor => CellKind::Wall,
            CellKind::Wall => CellKind::Floor,
        };
        self.drag_kind = Some(kind);
        self.apply_cell(ctx, point, kind);
        true
    }

    fn canvas_move(&mut self, ctx: &Context<Self>, event: PointerEvent) -> bool {
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
        self.selected = Some(point);
        self.apply_cell(ctx, point, kind);
        true
    }

    fn point_from_pointer(&self, event: &PointerEvent) -> Option<GridPoint> {
        let canvas = self.canvas_ref.cast::<HtmlCanvasElement>()?;
        let bounds = canvas.get_bounding_client_rect();
        if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
            return None;
        }
        let x = (f64::from(event.client_x()) - bounds.left()) * f64::from(canvas.width())
            / bounds.width();
        let y = (f64::from(event.client_y()) - bounds.top()) * f64::from(canvas.height())
            / bounds.height();
        let board_x = x - BOARD_ORIGIN;
        let board_y = y - BOARD_ORIGIN;
        if board_x < 0.0
            || board_y < 0.0
            || board_x >= CELL_SIZE * 8.0
            || board_y >= CELL_SIZE * 8.0
        {
            return None;
        }
        Some(GridPoint::new(
            (board_x / CELL_SIZE).floor() as u16,
            (board_y / CELL_SIZE).floor() as u16,
        ))
    }

    fn apply_cell(&mut self, ctx: &Context<Self>, point: GridPoint, kind: CellKind) -> bool {
        if !self.can_edit_timeline {
            self.push_toast(
                "Editing requires the exact edit_timeline capability".to_owned(),
                "warning",
            );
            return true;
        }
        if self.pending_cells.contains_key(&point) {
            return false;
        }
        if matches!(self.rpc_state, RpcState::Connecting | RpcState::Resyncing) {
            self.push_toast(
                "Wait for collaboration sync before editing".to_owned(),
                "info",
            );
            return true;
        }

        if matches!(self.rpc_state, RpcState::Online) {
            let Some(rpc) = self.rpc.clone() else {
                return false;
            };
            let command = self.model.prepare_set_cell(point, kind, now_ms());
            let command_id = command.metadata.id.to_string();
            self.pending_commands.insert(command_id.clone());
            self.pending_cells.insert(
                point,
                PendingCell {
                    kind,
                    command_id: command_id.clone(),
                },
            );
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
        } else {
            let result = self.model.set_cell(point, kind, now_ms());
            self.finish_local_edit(result)
        }
    }

    fn finish_local_edit(
        &mut self,
        result: Result<ModelChange, oreak_core::TimelineError>,
    ) -> bool {
        match result {
            Ok(ModelChange::Applied { .. }) => {
                self.canvas_dirty = true;
                self.persist_draft();
                true
            }
            Ok(ModelChange::NoChange) => false,
            Err(error) => {
                self.push_toast(format!("Edit rejected: {error}"), "warning");
                true
            }
        }
    }

    fn undo(&mut self, ctx: &Context<Self>) -> bool {
        if !self.can_edit_timeline {
            self.push_toast(
                "Undo requires the exact edit_timeline capability".to_owned(),
                "warning",
            );
            return true;
        }
        if matches!(self.rpc_state, RpcState::Connecting | RpcState::Resyncing) {
            self.push_toast("Wait for collaboration sync before undo".to_owned(), "info");
            return true;
        }
        if matches!(self.rpc_state, RpcState::Online) {
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
            return true;
        }

        match self.model.undo(now_ms()) {
            Ok(event) => {
                self.canvas_dirty = true;
                self.persist_draft();
                self.open_menu = None;
                self.palette_open = false;
                self.push_toast(
                    format!("Appended undo as sequence #{:04}", event.sequence),
                    "success",
                );
            }
            Err(error) => self.push_toast(error.to_string(), "warning"),
        }
        true
    }

    fn key_down(&mut self, ctx: &Context<Self>, event: KeyboardEvent) -> bool {
        let scope = if self.palette_open {
            ShortcutScope::Palette
        } else if event_target_is_text_entry(&event) {
            ShortcutScope::TextEntry
        } else {
            ShortcutScope::Workspace
        };
        let Some(shortcut) = resolve_shortcut(
            &event.key(),
            event.ctrl_key() || event.meta_key(),
            event.shift_key(),
            scope,
        ) else {
            return false;
        };
        event.prevent_default();

        match shortcut {
            Shortcut::SelectMode(mode) => self.set_mode(mode),
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
        self.connection_attempt += 1;
        self.rpc = None;
        self.rpc_state = RpcState::Connecting;
        self.pending_commands.clear();
        self.pending_cells.clear();
        self.canvas_dirty = true;
        spawn_connection(ctx, self.connection_attempt, self.target.clone());
    }

    fn handle_rpc_update(&mut self, ctx: &Context<Self>, update: RpcUpdate) -> bool {
        match update {
            RpcUpdate::Snapshot(snapshot) => self.accept_server_snapshot(snapshot),
            RpcUpdate::History {
                target,
                through_sequence,
                events,
            } => self.accept_server_history(target, through_sequence, events),
            RpcUpdate::Subscription(LevelSubscriptionItem::Snapshot { snapshot }) => {
                self.accept_server_snapshot(*snapshot)
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
            RpcUpdate::Closed(error) => {
                self.connection_attempt += 1;
                self.rpc = None;
                self.rpc_state = RpcState::Offline;
                self.pending_commands.clear();
                self.pending_cells.clear();
                let snapshot = self.model.timeline().snapshot().clone();
                let _ = self.model.replace_snapshot(snapshot);
                self.canvas_dirty = true;
                self.push_toast(
                    format!("Collaboration disconnected; local editing remains active: {error}"),
                    "warning",
                );
                true
            }
        }
    }

    fn accept_server_snapshot(&mut self, response: LevelSnapshotResponse) -> bool {
        if response.target != self.target {
            self.connection_attempt += 1;
            self.rpc = None;
            self.rpc_state = RpcState::Offline;
            self.pending_commands.clear();
            self.pending_cells.clear();
            self.canvas_dirty = true;
            self.push_toast(
                format!(
                    "Ignored snapshot for unexpected target {} / {}",
                    response.target.project_id, response.target.level_id
                ),
                "warning",
            );
            return true;
        }
        let computed_hash = response.snapshot.content_hash().to_string();
        if computed_hash != response.level_hash {
            self.connection_attempt += 1;
            self.rpc = None;
            self.rpc_state = RpcState::Offline;
            self.pending_commands.clear();
            self.pending_cells.clear();
            self.canvas_dirty = true;
            self.push_toast(
                "Server snapshot hash did not match oreak-core; staying local".to_owned(),
                "warning",
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

        if let Err(error) = self.model.replace_snapshot(response.snapshot) {
            self.connection_attempt += 1;
            self.rpc = None;
            self.rpc_state = RpcState::Offline;
            self.pending_commands.clear();
            self.pending_cells.clear();
            self.canvas_dirty = true;
            self.push_toast(
                format!("Server snapshot was rejected locally: {error}"),
                "warning",
            );
            return true;
        }
        self.server_sequence = Some(response.server_sequence);
        self.server_hash = Some(response.level_hash);
        self.pending_commands.clear();
        self.pending_cells.clear();
        self.canvas_dirty = true;
        self.persist_draft();
        true
    }

    fn accept_server_history(
        &mut self,
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
                "history did not match the subscribed target and snapshot sequence".to_owned(),
            );
        }
        let projection = match hydrate_history(self.model.timeline().snapshot(), &events) {
            Ok(projection) => projection,
            Err(error) => return self.reject_server_history(error),
        };

        self.server_events = events;
        self.seen_commands = projection.seen_commands;
        self.remote_blame = projection.cell_blame;
        self.canvas_dirty = true;
        true
    }

    fn reject_server_history(&mut self, error: String) -> bool {
        self.connection_attempt += 1;
        self.rpc = None;
        self.rpc_state = RpcState::Offline;
        self.pending_commands.clear();
        self.pending_cells.clear();
        self.canvas_dirty = true;
        self.push_toast(
            format!("Server history hydration failed: {error}"),
            "warning",
        );
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
        self.remove_pending(&command_id);
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
            self.rpc_state = RpcState::Offline;
            self.pending_commands.clear();
            self.pending_cells.clear();
            self.canvas_dirty = true;
            self.push_toast(format!("Resync unavailable: {reason}"), "warning");
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
        self.canvas_dirty = true;
    }

    fn effective_cell(&self, point: GridPoint) -> Option<CellKind> {
        self.pending_cells
            .get(&point)
            .map(|pending| pending.kind)
            .or_else(|| self.model.cell(point).ok())
    }

    fn activity_events(&self) -> &[HistoryEvent] {
        if matches!(self.rpc_state, RpcState::Online | RpcState::Resyncing) {
            &self.server_events
        } else {
            self.model.timeline().events()
        }
    }

    fn blame_for_cell(&self, point: GridPoint) -> Option<BlameEntry> {
        if matches!(self.rpc_state, RpcState::Online | RpcState::Resyncing) {
            self.remote_blame.get(&point).cloned()
        } else {
            let local = self.model.timeline().blame_cell(point);
            if local
                .as_ref()
                .is_some_and(|entry| entry.actor.as_str() == self.model.actor().as_str())
            {
                local
            } else {
                self.remote_blame.get(&point).cloned().or(local)
            }
        }
    }

    fn history_for_cell(&self, point: GridPoint) -> Vec<&HistoryEvent> {
        if matches!(self.rpc_state, RpcState::Online | RpcState::Resyncing) {
            let target = LevelTarget::Cell(point);
            self.server_events
                .iter()
                .filter(|event| event.changes.iter().any(|change| change.target == target))
                .collect()
        } else {
            self.model.timeline().history_for_cell(point)
        }
    }

    fn push_toast(&mut self, message: String, tone: &'static str) {
        let id = self.next_toast_id;
        self.next_toast_id += 1;
        self.toasts.push(Toast { id, message, tone });
        if self.toasts.len() > 3 {
            self.toasts.remove(0);
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

        context.set_fill_style_str(palette.background);
        context.fill_rect(0.0, 0.0, f64::from(CANVAS_SIZE), f64::from(CANVAS_SIZE));

        context.set_stroke_style_str(palette.guide);
        context.set_line_width(1.0);
        for guide in (14..CANVAS_SIZE).step_by(16) {
            let guide = f64::from(guide) + 0.5;
            context.begin_path();
            context.move_to(guide, 0.0);
            context.line_to(guide, f64::from(CANVAS_SIZE));
            context.stroke();
            context.begin_path();
            context.move_to(0.0, guide);
            context.line_to(f64::from(CANVAS_SIZE), guide);
            context.stroke();
        }

        context.set_font("11px ui-monospace, SFMono-Regular, Consolas, monospace");
        context.set_text_align("center");
        context.set_text_baseline("middle");

        for y in 0..8_u16 {
            for x in 0..8_u16 {
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

        context.set_fill_style_str(palette.label);
        for index in 0..8_u16 {
            let center = BOARD_ORIGIN + f64::from(index) * CELL_SIZE + CELL_SIZE / 2.0;
            context.fill_text(&format!("{index:02}"), center, 23.0)?;
            context.fill_text(&format!("{index:02}"), 22.0, center)?;
        }

        if let Some(point) = self.selected {
            let left = BOARD_ORIGIN + f64::from(point.x) * CELL_SIZE;
            let top = BOARD_ORIGIN + f64::from(point.y) * CELL_SIZE;
            context.set_stroke_style_str(palette.selection);
            context.set_line_width(3.0);
            context.stroke_rect(left + 2.0, top + 2.0, CELL_SIZE - 4.0, CELL_SIZE - 4.0);
            context.set_fill_style_str(palette.selection);
            context.fill_rect(left + 2.0, top + 2.0, 14.0, 3.0);
            context.fill_rect(left + 2.0, top + 2.0, 3.0, 14.0);
        }

        context.set_text_align("right");
        context.set_fill_style_str(palette.label);
        context.fill_text(
            &format!(
                "{} / {} EVENTS",
                self.mode.label().to_ascii_uppercase(),
                self.activity_events().len()
            ),
            754.0,
            754.0,
        )?;
        Ok(())
    }
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
            },
        }
    }
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

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
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

fn event_target_is_text_entry(event: &KeyboardEvent) -> bool {
    let Some(element) = event
        .target()
        .and_then(|target| target.dyn_into::<Element>().ok())
    else {
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
        LevelCommand::PlaceEntity { entity } => (
            "Place entity",
            format!(
                "{} @ {:02}:{:02}",
                entity.id(),
                entity.origin().x,
                entity.origin().y
            ),
        ),
        LevelCommand::MoveEntity { entity_id, origin } => (
            "Move entity",
            format!("{entity_id} -> {:02}:{:02}", origin.x, origin.y),
        ),
        LevelCommand::RotateEntityClockwise { entity_id } => {
            ("Rotate entity", entity_id.to_string())
        }
        LevelCommand::FlipEntityHorizontal { entity_id } => ("Flip entity", entity_id.to_string()),
        LevelCommand::DeleteEntity { entity_id } => ("Delete entity", entity_id.to_string()),
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

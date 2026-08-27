//! Non-production, in-memory application services for the MVP REST API.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{Arc, OnceLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use argon2::{
    Algorithm, Argon2, Params, Version,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{COOKIE, RETRY_AFTER, SET_COOKIE},
    },
    response::{IntoResponse, Response},
    routing::{get, post},
};
use oreak_plugin_api::MANIFEST_FORMAT_VERSION;
use oreak_project::{
    ApprovalPolicy, AuditAction, AuditContext, AuditEvent, AuditEventId, Capability, LevelId,
    Project, ProjectError, ProjectId, ProjectRole, ReleaseChannelId, ReleaseGate, ThemeId,
    TimestampMs, UserId, Workspace, WorkspaceError, WorkspaceId, WorkspaceKind, WorkspaceRole,
};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};

const API_BODY_LIMIT: usize = 16 * 1024;
const AUTH_RATE_LIMIT: u32 = 5;
const AUTH_RATE_WINDOW: Duration = Duration::from_secs(60);
const MAX_RATE_LIMIT_KEYS: usize = 1_024;
const MAX_SESSIONS: usize = 4_096;
const SESSION_COOKIE: &str = "oreak_session";
const SESSION_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const MIN_PASSWORD_BYTES: usize = 12;
const MAX_PASSWORD_BYTES: usize = 128;
const MAX_NAME_BYTES: usize = 120;
const MAX_AUDIT_SUMMARIES: usize = 100;

#[derive(Clone)]
pub(crate) struct MvpState {
    store: Arc<RwLock<MemoryStore>>,
    auth_rate_limiter: Arc<Mutex<RateLimiter>>,
    secure_cookies: bool,
}

impl MvpState {
    pub(crate) fn new(secure_cookies: bool) -> Self {
        Self {
            store: Arc::new(RwLock::new(MemoryStore::default())),
            auth_rate_limiter: Arc::new(Mutex::new(RateLimiter::new(
                AUTH_RATE_LIMIT,
                AUTH_RATE_WINDOW,
                MAX_RATE_LIMIT_KEYS,
            ))),
            secure_cookies,
        }
    }

    async fn check_auth_rate_limit(&self, scope: &str, email: &str) -> Result<(), ApiError> {
        let email_key = if email.len() <= 254 {
            email.trim().to_ascii_lowercase()
        } else {
            "invalid".to_owned()
        };
        let key = format!("{scope}:{email_key}");
        if self
            .auth_rate_limiter
            .lock()
            .await
            .check(key, Instant::now())
        {
            Ok(())
        } else {
            Err(ApiError::rate_limited())
        }
    }

    pub(crate) async fn authorize_level(
        &self,
        token: Option<&str>,
        project_id: &str,
        level_id: &str,
        access: RpcLevelAccess,
    ) -> Result<UserId, RpcAccessError> {
        let token = token.ok_or(RpcAccessError::AuthenticationRequired)?;
        let user_id = self
            .authenticate_token(token)
            .await
            .map_err(|_| RpcAccessError::AuthenticationRequired)?;
        let project_id =
            ProjectId::new(project_id).map_err(|_| RpcAccessError::LevelUnavailable)?;
        let level_id = LevelId::new(level_id).map_err(|_| RpcAccessError::LevelUnavailable)?;
        let store = self.store.read().await;
        let record = store
            .projects
            .get(&project_id)
            .ok_or(RpcAccessError::LevelUnavailable)?;
        if !record.project.timelines().contains_key(&level_id) {
            return Err(RpcAccessError::LevelUnavailable);
        }
        let project_access = match access {
            RpcLevelAccess::View => ProjectAccess::View,
            RpcLevelAccess::Edit => ProjectAccess::EditLevel,
        };
        if !has_project_access(&store, record, &user_id, project_access) {
            return Err(RpcAccessError::PermissionDenied);
        }
        Ok(user_id)
    }

    async fn authenticate_token(&self, token: &str) -> Result<UserId, ApiError> {
        let now = SystemTime::now();
        let mut store = self.store.write().await;
        let Some(session) = store.sessions.get(token).cloned() else {
            return Err(ApiError::unauthorized());
        };
        if session.expires_at <= now {
            store.sessions.remove(token);
            return Err(ApiError::unauthorized());
        }
        if !store.users.contains_key(&session.user_id) {
            store.sessions.remove(token);
            return Err(ApiError::unauthorized());
        }
        Ok(session.user_id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RpcAccessError {
    AuthenticationRequired,
    PermissionDenied,
    LevelUnavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RpcLevelAccess {
    View,
    Edit,
}

/// All MVP persistence is process-local memory and is not suitable for production use.
#[derive(Default)]
struct MemoryStore {
    users: HashMap<UserId, UserRecord>,
    user_ids_by_email: HashMap<String, UserId>,
    sessions: HashMap<String, SessionRecord>,
    workspaces: BTreeMap<WorkspaceId, WorkspaceRecord>,
    projects: BTreeMap<ProjectId, ProjectRecord>,
}

#[derive(Clone)]
struct UserRecord {
    id: UserId,
    email: String,
    password_hash: String,
    personal_workspace_id: WorkspaceId,
}

#[derive(Clone)]
struct SessionRecord {
    user_id: UserId,
    created_at: SystemTime,
    expires_at: SystemTime,
}

struct WorkspaceRecord {
    name: String,
    workspace: Workspace,
}

struct ProjectRecord {
    name: String,
    project: Project,
    level_names: BTreeMap<LevelId, String>,
}

struct RateLimiter {
    limit: u32,
    window: Duration,
    max_keys: usize,
    entries: HashMap<String, RateEntry>,
}

struct RateEntry {
    window_started: Instant,
    last_seen: Instant,
    attempts: u32,
}

impl RateLimiter {
    fn new(limit: u32, window: Duration, max_keys: usize) -> Self {
        Self {
            limit,
            window,
            max_keys,
            entries: HashMap::new(),
        }
    }

    fn check(&mut self, key: String, now: Instant) -> bool {
        self.entries
            .retain(|_, entry| now.duration_since(entry.last_seen) < self.window);

        if !self.entries.contains_key(&key) && self.entries.len() >= self.max_keys {
            if let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_seen)
                .map(|(key, _)| key.clone())
            {
                self.entries.remove(&oldest);
            }
        }

        let entry = self.entries.entry(key).or_insert(RateEntry {
            window_started: now,
            last_seen: now,
            attempts: 0,
        });
        entry.last_seen = now;
        if now.duration_since(entry.window_started) >= self.window {
            entry.window_started = now;
            entry.attempts = 0;
        }
        if entry.attempts >= self.limit {
            return false;
        }
        entry.attempts += 1;
        true
    }
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
    retry_after_seconds: Option<u64>,
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            retry_after_seconds: None,
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "authentication required")
    }

    fn forbidden() -> Self {
        Self::new(StatusCode::FORBIDDEN, "permission denied")
    }

    fn not_found(resource: &str) -> Self {
        Self::new(StatusCode::NOT_FOUND, format!("{resource} not found"))
    }

    fn internal() -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal server error")
    }

    fn invalid_login() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "invalid email or password")
    }

    fn rate_limited() -> Self {
        Self {
            status: StatusCode::TOO_MANY_REQUESTS,
            message: "too many authentication attempts".to_owned(),
            retry_after_seconds: Some(AUTH_RATE_WINDOW.as_secs()),
        }
    }

    fn project(error: ProjectError) -> Self {
        let status = match &error {
            ProjectError::PermissionDenied { .. } | ProjectError::ProjectOwnerRequired => {
                StatusCode::FORBIDDEN
            }
            ProjectError::LevelNotFound(_) | ProjectError::ReleaseChannelNotFound(_) => {
                StatusCode::NOT_FOUND
            }
            ProjectError::DuplicateLevel(_) | ProjectError::DuplicateReleaseChannel(_) => {
                StatusCode::CONFLICT
            }
            _ => StatusCode::BAD_REQUEST,
        };
        Self::new(status, error.to_string())
    }

    fn workspace(error: WorkspaceError) -> Self {
        let status = match &error {
            WorkspaceError::PermissionDenied | WorkspaceError::OwnerRequired => {
                StatusCode::FORBIDDEN
            }
            WorkspaceError::DuplicateProject(_) => StatusCode::CONFLICT,
            _ => StatusCode::BAD_REQUEST,
        };
        Self::new(status, error.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut response = (
            self.status,
            Json(ErrorBody {
                error: self.message,
            }),
        )
            .into_response();
        if let Some(seconds) = self.retry_after_seconds
            && let Ok(value) = HeaderValue::from_str(&seconds.to_string())
        {
            response.headers_mut().insert(RETRY_AFTER, value);
        }
        response
    }
}

#[derive(Deserialize)]
struct CredentialsRequest {
    email: String,
    password: String,
}

#[derive(Serialize)]
struct UserSummary {
    id: UserId,
    email: String,
    personal_workspace_id: WorkspaceId,
}

impl From<&UserRecord> for UserSummary {
    fn from(user: &UserRecord) -> Self {
        Self {
            id: user.id.clone(),
            email: user.email.clone(),
            personal_workspace_id: user.personal_workspace_id.clone(),
        }
    }
}

#[derive(Deserialize)]
struct CreateWorkspaceRequest {
    name: String,
}

#[derive(Serialize)]
struct WorkspaceList {
    workspaces: Vec<WorkspaceSummary>,
}

#[derive(Serialize)]
struct WorkspaceSummary {
    id: WorkspaceId,
    name: String,
    kind: &'static str,
    role: &'static str,
    project_count: usize,
}

#[derive(Deserialize)]
struct CreateProjectRequest {
    workspace_id: WorkspaceId,
    name: String,
}

#[derive(Serialize)]
struct ProjectList {
    projects: Vec<ProjectSummary>,
}

#[derive(Serialize)]
struct ProjectSummary {
    id: ProjectId,
    workspace_id: WorkspaceId,
    name: String,
    workspace_role: Option<&'static str>,
    roles: Vec<String>,
    capabilities: Vec<String>,
    level_count: usize,
    plugin_manifest_format_version: u32,
}

#[derive(Serialize)]
struct MembershipList {
    members: Vec<MembershipSummary>,
}

#[derive(Serialize)]
struct MembershipSummary {
    user_id: UserId,
    email: Option<String>,
    roles: Vec<String>,
    capabilities: Vec<String>,
    joined_at_ms: i64,
}

#[derive(Deserialize)]
struct CreateLevelRequest {
    name: String,
}

#[derive(Serialize)]
struct LevelList {
    levels: Vec<LevelSummary>,
}

#[derive(Serialize)]
struct LevelSummary {
    id: LevelId,
    name: String,
    revision_count: usize,
}

#[derive(Serialize)]
struct AuditList {
    audit: Vec<AuditSummary>,
}

#[derive(Serialize)]
struct AuditSummary {
    id: AuditEventId,
    actor: UserId,
    occurred_at_ms: i64,
    action: AuditAction,
}

#[derive(Deserialize)]
struct CreateReleaseChannelRequest {
    name: String,
}

#[derive(Serialize)]
struct ReleaseChannelList {
    release_channels: Vec<ReleaseChannelSummary>,
}

#[derive(Serialize)]
struct ReleaseChannelSummary {
    id: ReleaseChannelId,
    name: String,
    created_by: UserId,
    created_at_ms: i64,
    current_artifact_id: Option<oreak_project::ArtifactId>,
}

#[derive(Clone, Copy)]
enum ProjectAccess {
    View,
    Administration,
    Audit,
    EditLevel,
    ManageReleaseChannels,
}

pub(crate) fn router(state: MvpState) -> Router {
    Router::new()
        .route("/api/auth/register", post(register))
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/auth/me", get(me))
        .route(
            "/api/workspaces",
            get(list_workspaces).post(create_workspace),
        )
        .route(
            "/api/workspaces/{workspace_id}/audit",
            get(list_workspace_audit),
        )
        .route("/api/projects", get(list_projects).post(create_project))
        .route(
            "/api/projects/{project_id}/members",
            get(list_project_members),
        )
        .route(
            "/api/projects/{project_id}/levels",
            get(list_levels).post(create_level),
        )
        .route("/api/projects/{project_id}/audit", get(list_project_audit))
        .route(
            "/api/projects/{project_id}/release-channels",
            get(list_release_channels).post(create_release_channel),
        )
        .layer(DefaultBodyLimit::max(API_BODY_LIMIT))
        .with_state(state)
}

async fn register(
    State(state): State<MvpState>,
    Json(input): Json<CredentialsRequest>,
) -> Result<Response, ApiError> {
    state
        .check_auth_rate_limit("register", &input.email)
        .await?;
    let email = normalize_email(&input.email)?;
    validate_password(&input.password)?;

    if state
        .store
        .read()
        .await
        .user_ids_by_email
        .contains_key(&email)
    {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "an account with that email already exists",
        ));
    }

    let password = input.password;
    let password_hash = tokio::task::spawn_blocking(move || hash_password(&password))
        .await
        .map_err(|_| ApiError::internal())?
        .map_err(|()| ApiError::internal())?;
    let user_id = UserId::new(random_identifier("usr")?).map_err(|_| ApiError::internal())?;
    let workspace_id =
        WorkspaceId::new(random_identifier("ws")?).map_err(|_| ApiError::internal())?;
    let context = audit_context(&user_id)?;
    let workspace = Workspace::new_personal(workspace_id.clone(), user_id.clone(), context)
        .map_err(ApiError::workspace)?;
    let user = UserRecord {
        id: user_id.clone(),
        email: email.clone(),
        password_hash,
        personal_workspace_id: workspace_id.clone(),
    };

    let (token, expires_at) = {
        let mut store = state.store.write().await;
        if store.user_ids_by_email.contains_key(&email) {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "an account with that email already exists",
            ));
        }
        if store.users.contains_key(&user_id) || store.workspaces.contains_key(&workspace_id) {
            return Err(ApiError::internal());
        }
        store.workspaces.insert(
            workspace_id,
            WorkspaceRecord {
                name: "Personal workspace".to_owned(),
                workspace,
            },
        );
        store.user_ids_by_email.insert(email, user_id.clone());
        store.users.insert(user_id.clone(), user.clone());
        create_session(&mut store, user_id)?
    };

    session_response(
        StatusCode::CREATED,
        UserSummary::from(&user),
        &token,
        expires_at,
        state.secure_cookies,
    )
}

async fn login(
    State(state): State<MvpState>,
    Json(input): Json<CredentialsRequest>,
) -> Result<Response, ApiError> {
    state.check_auth_rate_limit("login", &input.email).await?;
    let email = normalize_email(&input.email).ok();
    let password_valid_length = password_has_valid_length(&input.password);
    let credentials = if password_valid_length {
        let store = state.store.read().await;
        email.as_ref().and_then(|email| {
            let user_id = store.user_ids_by_email.get(email)?;
            let user = store.users.get(user_id)?;
            Some((user_id.clone(), user.password_hash.clone()))
        })
    } else {
        None
    };
    let account_exists = credentials.is_some();
    let (user_id, stored_hash) = credentials.unwrap_or_else(|| {
        (
            UserId::new("invalid").expect("fixed ID is valid"),
            dummy_hash(),
        )
    });
    let password = input.password;
    let verified = if password_valid_length {
        tokio::task::spawn_blocking(move || verify_password(&stored_hash, &password))
            .await
            .map_err(|_| ApiError::internal())?
    } else {
        false
    };
    if !account_exists || !verified {
        return Err(ApiError::invalid_login());
    }

    let (user, token, expires_at) = {
        let mut store = state.store.write().await;
        let user = store
            .users
            .get(&user_id)
            .cloned()
            .ok_or_else(ApiError::invalid_login)?;
        let (token, expires_at) = create_session(&mut store, user_id)?;
        (user, token, expires_at)
    };
    session_response(
        StatusCode::OK,
        UserSummary::from(&user),
        &token,
        expires_at,
        state.secure_cookies,
    )
}

async fn logout(State(state): State<MvpState>, headers: HeaderMap) -> Response {
    if let Some(token) = session_token(&headers) {
        state.store.write().await.sessions.remove(&token);
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        SET_COOKIE,
        HeaderValue::from_str(&clear_session_cookie(state.secure_cookies))
            .expect("the fixed clearing cookie is a valid header"),
    );
    response
}

async fn me(
    State(state): State<MvpState>,
    headers: HeaderMap,
) -> Result<Json<UserSummary>, ApiError> {
    let user_id = authenticate(&state, &headers).await?;
    let store = state.store.read().await;
    let user = store
        .users
        .get(&user_id)
        .ok_or_else(ApiError::unauthorized)?;
    Ok(Json(UserSummary::from(user)))
}

async fn list_workspaces(
    State(state): State<MvpState>,
    headers: HeaderMap,
) -> Result<Json<WorkspaceList>, ApiError> {
    let user_id = authenticate(&state, &headers).await?;
    let store = state.store.read().await;
    let workspaces = store
        .workspaces
        .values()
        .filter(|record| record.workspace.members().contains_key(&user_id))
        .map(|record| workspace_summary(record, &user_id))
        .collect();
    Ok(Json(WorkspaceList { workspaces }))
}

async fn create_workspace(
    State(state): State<MvpState>,
    headers: HeaderMap,
    Json(input): Json<CreateWorkspaceRequest>,
) -> Result<(StatusCode, Json<WorkspaceSummary>), ApiError> {
    let user_id = authenticate(&state, &headers).await?;
    let name = validate_name(input.name)?;
    let workspace_id =
        WorkspaceId::new(random_identifier("ws")?).map_err(|_| ApiError::internal())?;
    let workspace = Workspace::new_organization(
        workspace_id.clone(),
        user_id.clone(),
        audit_context(&user_id)?,
    )
    .map_err(ApiError::workspace)?;
    let record = WorkspaceRecord { name, workspace };
    let summary = workspace_summary(&record, &user_id);
    let mut store = state.store.write().await;
    if store.workspaces.contains_key(&workspace_id) {
        return Err(ApiError::internal());
    }
    store.workspaces.insert(workspace_id, record);
    Ok((StatusCode::CREATED, Json(summary)))
}

async fn list_workspace_audit(
    State(state): State<MvpState>,
    headers: HeaderMap,
    Path(workspace_id): Path<String>,
) -> Result<Json<AuditList>, ApiError> {
    let user_id = authenticate(&state, &headers).await?;
    let workspace_id = parse_workspace_id(workspace_id)?;
    let store = state.store.read().await;
    let workspace = store
        .workspaces
        .get(&workspace_id)
        .ok_or_else(|| ApiError::not_found("workspace"))?;
    if !workspace.workspace.can_administer(&user_id) {
        return Err(ApiError::forbidden());
    }
    Ok(Json(AuditList {
        audit: audit_summaries(workspace.workspace.audit_events()),
    }))
}

async fn list_projects(
    State(state): State<MvpState>,
    headers: HeaderMap,
) -> Result<Json<ProjectList>, ApiError> {
    let user_id = authenticate(&state, &headers).await?;
    let store = state.store.read().await;
    let projects = store
        .projects
        .values()
        .filter(|record| has_project_access(&store, record, &user_id, ProjectAccess::View))
        .filter_map(|record| project_summary(&store, record, &user_id))
        .collect();
    Ok(Json(ProjectList { projects }))
}

async fn create_project(
    State(state): State<MvpState>,
    headers: HeaderMap,
    Json(input): Json<CreateProjectRequest>,
) -> Result<(StatusCode, Json<ProjectSummary>), ApiError> {
    let user_id = authenticate(&state, &headers).await?;
    let name = validate_name(input.name)?;
    let project_id = ProjectId::new(random_identifier("prj")?).map_err(|_| ApiError::internal())?;
    let project = Project::new(
        project_id.clone(),
        input.workspace_id.clone(),
        user_id.clone(),
        ThemeId::new("default").expect("the fixed theme ID is valid"),
        ApprovalPolicy::new(true, 1, BTreeSet::new()).expect("the fixed approval policy is valid"),
        ReleaseGate::default(),
        audit_context(&user_id)?,
    )
    .map_err(ApiError::project)?;
    let record = ProjectRecord {
        name,
        project,
        level_names: BTreeMap::new(),
    };

    let mut store = state.store.write().await;
    if store.projects.contains_key(&project_id) {
        return Err(ApiError::internal());
    }
    {
        let workspace = store
            .workspaces
            .get(&input.workspace_id)
            .ok_or_else(|| ApiError::not_found("workspace"))?;
        if !workspace.workspace.can_administer(&user_id) {
            return Err(ApiError::forbidden());
        }
    }
    store
        .workspaces
        .get_mut(&input.workspace_id)
        .expect("workspace was checked")
        .workspace
        .register_project(project_id.clone(), audit_context(&user_id)?)
        .map_err(ApiError::workspace)?;
    let inserted_project_id = project_id.clone();
    store.projects.insert(project_id, record);
    let summary = project_summary(
        &store,
        store
            .projects
            .get(&inserted_project_id)
            .expect("the project was inserted"),
        &user_id,
    )
    .ok_or_else(ApiError::internal)?;
    Ok((StatusCode::CREATED, Json(summary)))
}

async fn list_project_members(
    State(state): State<MvpState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
) -> Result<Json<MembershipList>, ApiError> {
    let user_id = authenticate(&state, &headers).await?;
    let project_id = parse_project_id(project_id)?;
    let store = state.store.read().await;
    let record = authorized_project(&store, &project_id, &user_id, ProjectAccess::Administration)?;
    let members = record
        .project
        .members()
        .values()
        .map(|member| MembershipSummary {
            user_id: member.user_id().clone(),
            email: store
                .users
                .get(member.user_id())
                .map(|user| user.email.clone()),
            roles: member.roles().iter().map(role_name).collect(),
            capabilities: record
                .project
                .capabilities_for(member.user_id())
                .iter()
                .map(capability_name)
                .collect(),
            joined_at_ms: member.joined_at().as_i64(),
        })
        .collect();
    Ok(Json(MembershipList { members }))
}

async fn list_levels(
    State(state): State<MvpState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
) -> Result<Json<LevelList>, ApiError> {
    let user_id = authenticate(&state, &headers).await?;
    let project_id = parse_project_id(project_id)?;
    let store = state.store.read().await;
    let record = authorized_project(&store, &project_id, &user_id, ProjectAccess::View)?;
    let levels = record
        .project
        .timelines()
        .values()
        .map(|timeline| LevelSummary {
            id: timeline.level_id().clone(),
            name: record
                .level_names
                .get(timeline.level_id())
                .cloned()
                .unwrap_or_else(|| timeline.level_id().to_string()),
            revision_count: timeline.revisions().len(),
        })
        .collect();
    Ok(Json(LevelList { levels }))
}

async fn create_level(
    State(state): State<MvpState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(input): Json<CreateLevelRequest>,
) -> Result<(StatusCode, Json<LevelSummary>), ApiError> {
    let user_id = authenticate(&state, &headers).await?;
    let project_id = parse_project_id(project_id)?;
    let name = validate_name(input.name)?;
    let level_id = LevelId::new(random_identifier("lvl")?).map_err(|_| ApiError::internal())?;
    let context = audit_context(&user_id)?;
    let mut store = state.store.write().await;
    authorized_project(&store, &project_id, &user_id, ProjectAccess::EditLevel)?;
    let record = store
        .projects
        .get_mut(&project_id)
        .expect("project was authorized");
    record
        .project
        .create_level(level_id.clone(), context)
        .map_err(ApiError::project)?;
    record.level_names.insert(level_id.clone(), name.clone());
    Ok((
        StatusCode::CREATED,
        Json(LevelSummary {
            id: level_id,
            name,
            revision_count: 0,
        }),
    ))
}

async fn list_project_audit(
    State(state): State<MvpState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
) -> Result<Json<AuditList>, ApiError> {
    let user_id = authenticate(&state, &headers).await?;
    let project_id = parse_project_id(project_id)?;
    let store = state.store.read().await;
    let record = authorized_project(&store, &project_id, &user_id, ProjectAccess::Audit)?;
    Ok(Json(AuditList {
        audit: audit_summaries(record.project.audit_events()),
    }))
}

async fn list_release_channels(
    State(state): State<MvpState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
) -> Result<Json<ReleaseChannelList>, ApiError> {
    let user_id = authenticate(&state, &headers).await?;
    let project_id = parse_project_id(project_id)?;
    let store = state.store.read().await;
    let record = authorized_project(&store, &project_id, &user_id, ProjectAccess::View)?;
    let release_channels = record
        .project
        .release_channels()
        .values()
        .map(|channel| ReleaseChannelSummary {
            id: channel.id().clone(),
            name: channel.name().to_owned(),
            created_by: channel.created_by().clone(),
            created_at_ms: channel.created_at().as_i64(),
            current_artifact_id: record.project.current_artifact(channel.id()).cloned(),
        })
        .collect();
    Ok(Json(ReleaseChannelList { release_channels }))
}

async fn create_release_channel(
    State(state): State<MvpState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(input): Json<CreateReleaseChannelRequest>,
) -> Result<(StatusCode, Json<ReleaseChannelSummary>), ApiError> {
    let user_id = authenticate(&state, &headers).await?;
    let project_id = parse_project_id(project_id)?;
    let name = validate_name(input.name)?;
    let channel_id =
        ReleaseChannelId::new(random_identifier("channel")?).map_err(|_| ApiError::internal())?;
    let context = audit_context(&user_id)?;
    let mut store = state.store.write().await;
    authorized_project(
        &store,
        &project_id,
        &user_id,
        ProjectAccess::ManageReleaseChannels,
    )?;
    let record = store
        .projects
        .get_mut(&project_id)
        .expect("project was authorized");
    record
        .project
        .create_release_channel(channel_id.clone(), name, context)
        .map_err(ApiError::project)?;
    let channel = record
        .project
        .release_channels()
        .get(&channel_id)
        .expect("the domain inserted the channel");
    Ok((
        StatusCode::CREATED,
        Json(ReleaseChannelSummary {
            id: channel.id().clone(),
            name: channel.name().to_owned(),
            created_by: channel.created_by().clone(),
            created_at_ms: channel.created_at().as_i64(),
            current_artifact_id: None,
        }),
    ))
}

async fn authenticate(state: &MvpState, headers: &HeaderMap) -> Result<UserId, ApiError> {
    let token = session_token(headers).ok_or_else(ApiError::unauthorized)?;
    state.authenticate_token(&token).await
}

fn create_session(
    store: &mut MemoryStore,
    user_id: UserId,
) -> Result<(String, SystemTime), ApiError> {
    let now = SystemTime::now();
    store.sessions.retain(|_, session| session.expires_at > now);
    if store.sessions.len() >= MAX_SESSIONS
        && let Some(oldest) = store
            .sessions
            .iter()
            .min_by_key(|(_, session)| session.created_at)
            .map(|(token, _)| token.clone())
    {
        store.sessions.remove(&oldest);
    }
    let expires_at = now
        .checked_add(SESSION_TTL)
        .ok_or_else(ApiError::internal)?;
    for _ in 0..4 {
        let token = random_hex(32)?;
        if !store.sessions.contains_key(&token) {
            store.sessions.insert(
                token.clone(),
                SessionRecord {
                    user_id: user_id.clone(),
                    created_at: now,
                    expires_at,
                },
            );
            return Ok((token, expires_at));
        }
    }
    Err(ApiError::internal())
}

fn session_response(
    status: StatusCode,
    user: UserSummary,
    token: &str,
    expires_at: SystemTime,
    secure: bool,
) -> Result<Response, ApiError> {
    let mut response = (status, Json(user)).into_response();
    let cookie = session_cookie(token, expires_at, secure);
    response.headers_mut().insert(
        SET_COOKIE,
        HeaderValue::from_str(&cookie).map_err(|_| ApiError::internal())?,
    );
    Ok(response)
}

fn session_cookie(token: &str, expires_at: SystemTime, secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };
    format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}; Expires={}{secure}",
        SESSION_TTL.as_secs(),
        httpdate::fmt_http_date(expires_at),
    )
}

fn clear_session_cookie(secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };
    format!(
        "{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0; Expires={}{secure}",
        httpdate::fmt_http_date(UNIX_EPOCH),
    )
}

pub(crate) fn session_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .find_map(|pair| {
            let (name, value) = pair.trim().split_once('=')?;
            (name == SESSION_COOKIE && !value.is_empty()).then(|| value.to_owned())
        })
}

fn normalize_email(raw: &str) -> Result<String, ApiError> {
    let email = raw.trim();
    if email.len() > 254 || !email.is_ascii() || email.chars().any(char::is_whitespace) {
        return Err(ApiError::bad_request("invalid email address"));
    }
    let Some((local, domain)) = email.split_once('@') else {
        return Err(ApiError::bad_request("invalid email address"));
    };
    if local.is_empty()
        || local.len() > 64
        || local.contains('@')
        || local.starts_with('.')
        || local.ends_with('.')
        || local.contains("..")
        || !local
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".!#$%&'*+/=?^_`{|}~-".contains(&byte))
        || domain.is_empty()
        || domain.len() > 253
        || domain.split('.').any(|label| {
            label.is_empty()
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
    {
        return Err(ApiError::bad_request("invalid email address"));
    }
    Ok(email.to_ascii_lowercase())
}

fn password_has_valid_length(password: &str) -> bool {
    (MIN_PASSWORD_BYTES..=MAX_PASSWORD_BYTES).contains(&password.len())
}

fn validate_password(password: &str) -> Result<(), ApiError> {
    if password_has_valid_length(password) {
        Ok(())
    } else {
        Err(ApiError::bad_request(format!(
            "password must be between {MIN_PASSWORD_BYTES} and {MAX_PASSWORD_BYTES} bytes"
        )))
    }
}

fn password_hasher() -> Argon2<'static> {
    Argon2::new(Algorithm::Argon2id, Version::V0x13, Params::default())
}

fn hash_password(password: &str) -> Result<String, ()> {
    let mut salt_bytes = [0_u8; 16];
    getrandom::fill(&mut salt_bytes).map_err(|_| ())?;
    hash_password_with_salt(password, &salt_bytes)
}

fn hash_password_with_salt(password: &str, salt_bytes: &[u8]) -> Result<String, ()> {
    let salt = SaltString::encode_b64(salt_bytes).map_err(|_| ())?;
    password_hasher()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| ())
}

fn verify_password(encoded: &str, password: &str) -> bool {
    PasswordHash::new(encoded).is_ok_and(|hash| {
        password_hasher()
            .verify_password(password.as_bytes(), &hash)
            .is_ok()
    })
}

fn dummy_hash() -> String {
    static DUMMY_HASH: OnceLock<String> = OnceLock::new();
    DUMMY_HASH
        .get_or_init(|| {
            hash_password_with_salt("not-a-real-password", b"oreak-dummy-salt")
                .expect("the fixed dummy password and salt are valid")
        })
        .clone()
}

fn validate_name(name: String) -> Result<String, ApiError> {
    let name = name.trim();
    if name.is_empty() || name.len() > MAX_NAME_BYTES || name.chars().any(char::is_control) {
        return Err(ApiError::bad_request(format!(
            "name must be between 1 and {MAX_NAME_BYTES} bytes"
        )));
    }
    Ok(name.to_owned())
}

fn random_identifier(prefix: &str) -> Result<String, ApiError> {
    Ok(format!("{prefix}_{}", random_hex(16)?))
}

fn random_hex(byte_count: usize) -> Result<String, ApiError> {
    let mut bytes = vec![0_u8; byte_count];
    getrandom::fill(&mut bytes).map_err(|_| ApiError::internal())?;
    let mut encoded = String::with_capacity(byte_count * 2);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    Ok(encoded)
}

fn audit_context(actor: &UserId) -> Result<AuditContext, ApiError> {
    Ok(AuditContext::new(
        AuditEventId::new(random_identifier("audit")?).map_err(|_| ApiError::internal())?,
        actor.clone(),
        now_timestamp()?,
    ))
}

fn now_timestamp() -> Result<TimestampMs, ApiError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ApiError::internal())?
        .as_millis();
    let millis = i64::try_from(millis).map_err(|_| ApiError::internal())?;
    Ok(TimestampMs::new(millis))
}

fn parse_workspace_id(value: String) -> Result<WorkspaceId, ApiError> {
    WorkspaceId::new(value).map_err(|_| ApiError::not_found("workspace"))
}

fn parse_project_id(value: String) -> Result<ProjectId, ApiError> {
    ProjectId::new(value).map_err(|_| ApiError::not_found("project"))
}

fn workspace_summary(record: &WorkspaceRecord, user_id: &UserId) -> WorkspaceSummary {
    let role = record
        .workspace
        .members()
        .get(user_id)
        .map(|member| workspace_role_name(member.role()))
        .unwrap_or("none");
    WorkspaceSummary {
        id: record.workspace.id().clone(),
        name: record.name.clone(),
        kind: match record.workspace.kind() {
            WorkspaceKind::Personal => "personal",
            WorkspaceKind::Organization => "organization",
        },
        role,
        project_count: record.workspace.projects().len(),
    }
}

fn project_summary(
    store: &MemoryStore,
    record: &ProjectRecord,
    user_id: &UserId,
) -> Option<ProjectSummary> {
    let workspace = store.workspaces.get(record.project.workspace_id())?;
    let workspace_role = workspace
        .workspace
        .members()
        .get(user_id)
        .map(|member| workspace_role_name(member.role()));
    let roles = record
        .project
        .members()
        .get(user_id)
        .map(|member| member.roles().iter().map(role_name).collect())
        .unwrap_or_default();
    let capabilities = record
        .project
        .capabilities_for(user_id)
        .iter()
        .map(capability_name)
        .collect();
    Some(ProjectSummary {
        id: record.project.id().clone(),
        workspace_id: record.project.workspace_id().clone(),
        name: record.name.clone(),
        workspace_role,
        roles,
        capabilities,
        level_count: record.project.timelines().len(),
        plugin_manifest_format_version: MANIFEST_FORMAT_VERSION,
    })
}

fn authorized_project<'a>(
    store: &'a MemoryStore,
    project_id: &ProjectId,
    user_id: &UserId,
    access: ProjectAccess,
) -> Result<&'a ProjectRecord, ApiError> {
    let record = store
        .projects
        .get(project_id)
        .ok_or_else(|| ApiError::not_found("project"))?;
    if has_project_access(store, record, user_id, access) {
        Ok(record)
    } else {
        Err(ApiError::forbidden())
    }
}

fn has_project_access(
    store: &MemoryStore,
    record: &ProjectRecord,
    user_id: &UserId,
    access: ProjectAccess,
) -> bool {
    let workspace_admin = store
        .workspaces
        .get(record.project.workspace_id())
        .is_some_and(|workspace| workspace.workspace.can_administer(user_id));
    match access {
        ProjectAccess::View => {
            workspace_admin
                || record
                    .project
                    .has_capability(user_id, &Capability::ViewProject)
        }
        ProjectAccess::Administration => {
            workspace_admin
                || record
                    .project
                    .has_capability(user_id, &Capability::ManageMembers)
        }
        ProjectAccess::Audit => {
            workspace_admin
                || record
                    .project
                    .has_capability(user_id, &Capability::ViewAudit)
        }
        ProjectAccess::EditLevel => record
            .project
            .has_capability(user_id, &Capability::EditTimeline),
        ProjectAccess::ManageReleaseChannels => record
            .project
            .has_capability(user_id, &Capability::ManageReleaseChannels),
    }
}

fn audit_summaries(events: &[AuditEvent]) -> Vec<AuditSummary> {
    events
        .iter()
        .rev()
        .take(MAX_AUDIT_SUMMARIES)
        .map(|event| AuditSummary {
            id: event.id().clone(),
            actor: event.actor().clone(),
            occurred_at_ms: event.occurred_at().as_i64(),
            action: event.action().clone(),
        })
        .collect()
}

fn workspace_role_name(role: WorkspaceRole) -> &'static str {
    match role {
        WorkspaceRole::Owner => "owner",
        WorkspaceRole::Admin => "admin",
        WorkspaceRole::Member => "member",
    }
}

fn role_name(role: &ProjectRole) -> String {
    match role {
        ProjectRole::Template(template) => format!("{template:?}").to_ascii_lowercase(),
        ProjectRole::Custom(role_id) => format!("custom:{}", role_id.as_str()),
    }
}

fn capability_name(capability: &Capability) -> String {
    match capability {
        Capability::Custom(name) => format!("custom:{name}"),
        capability => {
            let mut name = String::new();
            for (index, character) in format!("{capability:?}").chars().enumerate() {
                if index > 0 && character.is_ascii_uppercase() {
                    name.push('_');
                }
                name.push(character.to_ascii_lowercase());
            }
            name
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords_are_argon2id_hashes() {
        let hash = hash_password("correct horse battery staple").unwrap();
        assert!(hash.starts_with("$argon2id$v=19$"));
        assert!(!hash.contains("correct horse battery staple"));
        assert!(verify_password(&hash, "correct horse battery staple"));
        assert!(!verify_password(&hash, "wrong password"));
    }

    #[test]
    fn cookies_have_required_security_attributes() {
        let cookie = session_cookie("opaque-token", UNIX_EPOCH + SESSION_TTL, true);
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Lax"));
        assert!(cookie.contains("Max-Age=604800"));
        assert!(cookie.contains("Expires="));
        assert!(cookie.contains("Secure"));
    }

    #[test]
    fn rate_limiter_is_bounded_and_enforces_attempt_limit() {
        let now = Instant::now();
        let mut limiter = RateLimiter::new(2, Duration::from_secs(60), 2);
        assert!(limiter.check("a".to_owned(), now));
        assert!(limiter.check("a".to_owned(), now));
        assert!(!limiter.check("a".to_owned(), now));
        assert!(limiter.check("b".to_owned(), now));
        assert!(limiter.check("c".to_owned(), now));
        assert_eq!(limiter.entries.len(), 2);
    }

    #[test]
    fn workspace_admin_can_audit_but_cannot_edit_project_levels() {
        let admin = UserId::new("organization-admin").unwrap();
        let editor = UserId::new("project-editor").unwrap();
        let workspace_id = WorkspaceId::new("organization").unwrap();
        let workspace = Workspace::new_organization(
            workspace_id.clone(),
            admin.clone(),
            AuditContext::new(
                AuditEventId::new("workspace-created").unwrap(),
                admin.clone(),
                TimestampMs::new(0),
            ),
        )
        .unwrap();
        let project = Project::new(
            ProjectId::new("project").unwrap(),
            workspace_id.clone(),
            editor.clone(),
            ThemeId::new("default").unwrap(),
            ApprovalPolicy::new(true, 1, BTreeSet::new()).unwrap(),
            ReleaseGate::default(),
            AuditContext::new(
                AuditEventId::new("project-created").unwrap(),
                editor,
                TimestampMs::new(0),
            ),
        )
        .unwrap();
        let mut store = MemoryStore::default();
        store.workspaces.insert(
            workspace_id,
            WorkspaceRecord {
                name: "Organization".to_owned(),
                workspace,
            },
        );
        let record = ProjectRecord {
            name: "Project".to_owned(),
            project,
            level_names: BTreeMap::new(),
        };
        assert!(has_project_access(
            &store,
            &record,
            &admin,
            ProjectAccess::Audit
        ));
        assert!(has_project_access(
            &store,
            &record,
            &admin,
            ProjectAccess::Administration
        ));
        assert!(!has_project_access(
            &store,
            &record,
            &admin,
            ProjectAccess::EditLevel
        ));
    }
}

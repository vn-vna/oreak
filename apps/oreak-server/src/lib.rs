//! Axum and jsonrpsee transport for Oreak's in-memory collaboration proof.

#![forbid(unsafe_code)]

mod mvp;

use std::{
    collections::HashMap,
    convert::Infallible,
    env,
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    body::Body,
    extract::Request,
    http::{
        HeaderMap, HeaderValue, Method, StatusCode,
        header::{CONTENT_TYPE, HOST, ORIGIN},
    },
    response::{Html, IntoResponse, Response},
    routing::get,
};
use jsonrpsee::{
    core::{RpcResult, SubscriptionResult, async_trait, to_json_raw_value},
    server::{
        Methods, PendingSubscriptionSink, ServerBuilder, ServerHandle, SubscriptionSink,
        stop_channel,
    },
    types::ErrorObjectOwned,
};
use oreak_core::{ActorId, ApplyOutcome, LevelSnapshot, LevelTimeline, TimelineError};
use oreak_protocol::{
    ApplyCommandRequest, ApplyCommandResponse, ApplyCommandResult, HealthResponse, HealthStatus,
    LevelEvent, LevelHistoryRequest, LevelHistoryResponse, LevelSnapshotResponse,
    LevelSubscriptionItem, MAX_LEVEL_HISTORY_PAGE_SIZE, OreakRpcServer, ProjectLevelTarget,
    RpcErrorCode, RpcErrorData, RpcErrorDetails, UndoLatestRequest, UndoLatestResponse,
};
use tokio::sync::{Mutex, RwLock, broadcast};
use tower::{Service, service_fn};
use tower_http::{
    cors::{AllowOrigin, CorsLayer},
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use tracing::{error, warn};

const INITIAL_LEVEL_AXIS: u16 = 8;
const LEVEL_EVENT_CAPACITY: usize = 256;
const DEFAULT_WEB_DIST: &str = "apps/oreak-web/dist";

pub struct Application {
    router: Router,
    rpc_handle: ServerHandle,
}

impl Application {
    #[must_use]
    pub fn rpc_handle(&self) -> ServerHandle {
        self.rpc_handle.clone()
    }

    pub fn into_router(self) -> Router {
        self.router
    }
}

#[derive(Clone)]
pub struct OreakRpcService {
    application: mvp::MvpState,
    levels: Arc<RwLock<HashMap<ProjectLevelTarget, Arc<LevelState>>>>,
}

#[derive(Clone)]
struct RpcSessionToken(String);

struct LevelState {
    timeline: Mutex<LevelTimeline>,
    events: broadcast::Sender<LevelEvent>,
}

impl LevelState {
    fn new() -> Self {
        let snapshot = LevelSnapshot::new(INITIAL_LEVEL_AXIS, INITIAL_LEVEL_AXIS)
            .expect("the fixed initial level dimensions are valid");
        let timeline =
            LevelTimeline::new(snapshot).expect("a newly-created level snapshot is valid");
        let (events, _) = broadcast::channel(LEVEL_EVENT_CAPACITY);
        Self {
            timeline: Mutex::new(timeline),
            events,
        }
    }
}

impl OreakRpcService {
    fn new(application: mvp::MvpState) -> Self {
        Self {
            application,
            levels: Arc::default(),
        }
    }

    async fn level(&self, target: &ProjectLevelTarget) -> Arc<LevelState> {
        if let Some(level) = self.levels.read().await.get(target).cloned() {
            return level;
        }

        self.levels
            .write()
            .await
            .entry(target.clone())
            .or_insert_with(|| Arc::new(LevelState::new()))
            .clone()
    }

    async fn authorize(
        &self,
        extensions: &jsonrpsee::Extensions,
        target: &ProjectLevelTarget,
        access: mvp::RpcLevelAccess,
    ) -> Result<oreak_project::UserId, ErrorObjectOwned> {
        let token = extensions
            .get::<RpcSessionToken>()
            .map(|token| token.0.as_str());
        self.application
            .authorize_level(
                token,
                target.project_id.as_str(),
                target.level_id.as_str(),
                access,
            )
            .await
            .map_err(access_rpc_error)
    }
}

#[async_trait]
impl OreakRpcServer for OreakRpcService {
    async fn health(&self) -> RpcResult<HealthResponse> {
        Ok(health_response())
    }

    async fn level_snapshot(
        &self,
        extensions: &jsonrpsee::Extensions,
        target: ProjectLevelTarget,
    ) -> RpcResult<LevelSnapshotResponse> {
        self.authorize(extensions, &target, mvp::RpcLevelAccess::View)
            .await?;
        let level = self.level(&target).await;
        let timeline = level.timeline.lock().await;
        Ok(snapshot_response(target, &timeline))
    }

    async fn level_history(
        &self,
        extensions: &jsonrpsee::Extensions,
        request: LevelHistoryRequest,
    ) -> RpcResult<LevelHistoryResponse> {
        self.authorize(extensions, &request.target, mvp::RpcLevelAccess::View)
            .await?;
        if request.limit == 0 {
            return Err(invalid_command_rpc_error(
                "history page limit must be greater than zero",
            ));
        }

        let level = self.level(&request.target).await;
        let timeline = level.timeline.lock().await;
        let limit = usize::from(request.limit.min(MAX_LEVEL_HISTORY_PAGE_SIZE));
        let mut matching = timeline.events().iter().rev().filter(|event| {
            request
                .before_sequence
                .is_none_or(|before| event.sequence < before)
        });
        let events: Vec<_> = matching.by_ref().take(limit).cloned().collect();
        let has_more = matching.next().is_some();
        let next_before_sequence = has_more
            .then(|| events.last().map(|event| event.sequence))
            .flatten();

        Ok(LevelHistoryResponse {
            target: request.target,
            events,
            next_before_sequence,
            has_more,
        })
    }

    async fn apply_command(
        &self,
        extensions: &jsonrpsee::Extensions,
        mut request: ApplyCommandRequest,
    ) -> RpcResult<ApplyCommandResponse> {
        let user_id = self
            .authorize(extensions, &request.target, mvp::RpcLevelAccess::Edit)
            .await?;
        request.command.metadata.actor = ActorId::new(user_id.as_str());
        request.command.metadata.occurred_at_ms = server_timestamp_ms();
        let level = self.level(&request.target).await;
        let mut timeline = level.timeline.lock().await;
        let outcome = timeline
            .apply(request.command)
            .map_err(timeline_rpc_error)?;

        match outcome {
            ApplyOutcome::Applied(event) => {
                let update = LevelEvent {
                    target: request.target.clone(),
                    server_sequence: event.sequence,
                    level_hash: event.after_hash.to_string(),
                    event: event.clone(),
                };
                let _ = level.events.send(update);
                Ok(ApplyCommandResponse {
                    target: request.target,
                    server_sequence: event.sequence,
                    level_hash: event.after_hash.to_string(),
                    result: ApplyCommandResult::Applied {
                        event: Box::new(event),
                    },
                })
            }
            ApplyOutcome::NoChange { snapshot_hash } => Ok(ApplyCommandResponse {
                target: request.target,
                server_sequence: current_sequence(&timeline),
                level_hash: snapshot_hash.to_string(),
                result: ApplyCommandResult::NoChange,
            }),
        }
    }

    async fn undo_latest(
        &self,
        extensions: &jsonrpsee::Extensions,
        mut request: UndoLatestRequest,
    ) -> RpcResult<UndoLatestResponse> {
        let user_id = self
            .authorize(extensions, &request.target, mvp::RpcLevelAccess::Edit)
            .await?;
        request.metadata.actor = ActorId::new(user_id.as_str());
        request.metadata.occurred_at_ms = server_timestamp_ms();
        let level = self.level(&request.target).await;
        let mut timeline = level.timeline.lock().await;
        let event = timeline
            .undo_latest(request.metadata)
            .map_err(timeline_rpc_error)?;
        let update = LevelEvent {
            target: request.target.clone(),
            server_sequence: event.sequence,
            level_hash: event.after_hash.to_string(),
            event: event.clone(),
        };
        let _ = level.events.send(update);

        Ok(UndoLatestResponse {
            target: request.target,
            server_sequence: event.sequence,
            level_hash: event.after_hash.to_string(),
            event,
        })
    }

    async fn subscribe_level(
        &self,
        pending: PendingSubscriptionSink,
        extensions: &jsonrpsee::Extensions,
        target: ProjectLevelTarget,
    ) -> SubscriptionResult {
        let token = extensions.get::<RpcSessionToken>().cloned();
        if let Err(error) = self
            .authorize(extensions, &target, mvp::RpcLevelAccess::View)
            .await
        {
            pending.reject(error).await;
            return Ok(());
        }
        let level = self.level(&target).await;
        let mut receiver = level.events.subscribe();
        let initial = {
            let timeline = level.timeline.lock().await;
            snapshot_response(target.clone(), &timeline)
        };
        let initial_sequence = initial.server_sequence;
        let sink = pending.accept().await?;

        if !send_subscription(
            &sink,
            &LevelSubscriptionItem::Snapshot {
                snapshot: Box::new(initial),
            },
        )
        .await
        {
            return Ok(());
        }

        loop {
            tokio::select! {
                () = sink.closed() => break,
                received = receiver.recv() => {
                    let item = match received {
                        Ok(event) if event.server_sequence > initial_sequence => {
                            LevelSubscriptionItem::Event {
                                event: Box::new(event),
                            }
                        }
                        Ok(_) => continue,
                        Err(broadcast::error::RecvError::Lagged(missed_events)) => {
                            LevelSubscriptionItem::ResyncRequired {
                                target: target.clone(),
                                missed_events,
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    };

                    if self
                        .application
                        .authorize_level(
                            token.as_ref().map(|token| token.0.as_str()),
                            target.project_id.as_str(),
                            target.level_id.as_str(),
                            mvp::RpcLevelAccess::View,
                        )
                        .await
                        .is_err()
                    {
                        break;
                    }

                    if !send_subscription(&sink, &item).await {
                        break;
                    }
                }
            }
        }

        Ok(())
    }
}

#[must_use]
pub fn build_application() -> Application {
    let web_dist = env::var_os("OREAK_WEB_DIST")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_WEB_DIST));
    let secure_cookies = env::var("OREAK_SECURE_COOKIES").is_ok_and(|value| {
        matches!(
            value.to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    });
    build_application_with_config(web_dist, secure_cookies)
}

fn build_application_with_config(web_dist: PathBuf, secure_cookies: bool) -> Application {
    let application = mvp::MvpState::new(secure_cookies);
    let methods: Methods = OreakRpcService::new(application.clone()).into_rpc().into();
    let rpc_builder = ServerBuilder::default().to_service_builder();
    let (stop_handle, rpc_handle) = stop_channel();
    let rpc_service = service_fn(move |mut request: Request| {
        let origin_allowed = is_same_origin_rpc_request(request.headers());
        if let Some(token) = mvp::session_token(request.headers()) {
            request.extensions_mut().insert(RpcSessionToken(token));
        }
        let mut service = rpc_builder
            .clone()
            .build(methods.clone(), stop_handle.clone());
        async move {
            if !origin_allowed {
                return Ok::<Response, Infallible>(
                    (StatusCode::FORBIDDEN, "RPC origin is not allowed").into_response(),
                );
            }
            let response = match service.call(request).await {
                Ok(response) => response.map(Body::new),
                Err(error) => {
                    error!(%error, "JSON-RPC transport failed");
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "JSON-RPC transport failed",
                    )
                        .into_response()
                }
            };
            Ok::<Response, Infallible>(response)
        }
    });

    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|origin, _| {
            is_local_development_origin(origin)
        }))
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([CONTENT_TYPE])
        .allow_credentials(true);

    let router = Router::new()
        .merge(mvp::router(application))
        .route("/health", get(rest_health))
        .route_service("/rpc", rpc_service)
        .layer(cors)
        .layer(TraceLayer::new_for_http());
    let index = web_dist.join("index.html");
    let router = if index.is_file() {
        router.fallback_service(ServeDir::new(web_dist).fallback(ServeFile::new(index)))
    } else {
        router.route("/", get(root)).fallback(not_found)
    };

    Application { router, rpc_handle }
}

fn health_response() -> HealthResponse {
    HealthResponse {
        status: HealthStatus::Ok,
        version: env!("CARGO_PKG_VERSION").to_owned(),
    }
}

async fn rest_health() -> Json<HealthResponse> {
    Json(health_response())
}

async fn root() -> Html<&'static str> {
    Html(
        r#"<!doctype html><html><body><h1>Oreak server</h1><p><strong>Non-production demo:</strong> all users, sessions, workspaces, and projects are stored only in process memory.</p><p>Health: <a href="/health">/health</a></p><p>JSON-RPC HTTP/WebSocket: <code>/rpc</code></p><p>Compiled web assets were not found. Set <code>OREAK_WEB_DIST</code> to a Yew distribution directory.</p></body></html>"#,
    )
}

async fn not_found() -> (StatusCode, &'static str) {
    (StatusCode::NOT_FOUND, "not found")
}

fn current_sequence(timeline: &LevelTimeline) -> u64 {
    timeline.events().last().map_or(0, |event| event.sequence)
}

fn snapshot_response(
    target: ProjectLevelTarget,
    timeline: &LevelTimeline,
) -> LevelSnapshotResponse {
    LevelSnapshotResponse {
        target,
        server_sequence: current_sequence(timeline),
        snapshot: timeline.snapshot().clone(),
        level_hash: timeline.snapshot().content_hash().to_string(),
    }
}

fn server_timestamp_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}

async fn send_subscription(sink: &SubscriptionSink, item: &LevelSubscriptionItem) -> bool {
    let message = match to_json_raw_value(item) {
        Ok(message) => message,
        Err(error) => {
            error!(%error, "failed to serialize a level subscription item");
            return false;
        }
    };

    sink.send(message).await.is_ok()
}

fn timeline_rpc_error(error: TimelineError) -> ErrorObjectOwned {
    let (code, details) = match error {
        TimelineError::DuplicateCommandId(command_id) => (
            RpcErrorCode::DuplicateCommand,
            Some(RpcErrorDetails::DuplicateCommand { command_id }),
        ),
        TimelineError::UndoConflict {
            actor,
            blocked_sequences,
        } => (
            RpcErrorCode::UndoConflict,
            Some(RpcErrorDetails::UndoConflict {
                actor,
                blocked_sequences,
            }),
        ),
        TimelineError::NothingToUndo(actor) => (
            RpcErrorCode::NothingToUndo,
            Some(RpcErrorDetails::NothingToUndo { actor }),
        ),
        TimelineError::InvalidGrid(error) => (
            RpcErrorCode::InvalidCommand,
            Some(RpcErrorDetails::InvalidCommand {
                reason: error.to_string(),
            }),
        ),
        TimelineError::InvalidLevel(error) => (
            RpcErrorCode::InvalidCommand,
            Some(RpcErrorDetails::InvalidCommand {
                reason: error.to_string(),
            }),
        ),
        internal @ (TimelineError::UndoBecameNoOp(_) | TimelineError::SequenceExhausted) => {
            error!(error = %internal, "core timeline failed unexpectedly");
            (RpcErrorCode::Internal, None)
        }
    };
    let data = RpcErrorData::new(code, details);
    ErrorObjectOwned::owned(code.json_rpc_code(), code.message(), Some(data))
}

fn invalid_command_rpc_error(reason: impl Into<String>) -> ErrorObjectOwned {
    let code = RpcErrorCode::InvalidCommand;
    let data = RpcErrorData::new(
        code,
        Some(RpcErrorDetails::InvalidCommand {
            reason: reason.into(),
        }),
    );
    ErrorObjectOwned::owned(code.json_rpc_code(), code.message(), Some(data))
}

fn access_rpc_error(error: mvp::RpcAccessError) -> ErrorObjectOwned {
    let (code, details) = match error {
        mvp::RpcAccessError::AuthenticationRequired => (
            RpcErrorCode::AuthenticationRequired,
            Some(RpcErrorDetails::AuthenticationRequired),
        ),
        mvp::RpcAccessError::PermissionDenied => (
            RpcErrorCode::PermissionDenied,
            Some(RpcErrorDetails::PermissionDenied),
        ),
        mvp::RpcAccessError::LevelUnavailable => (
            RpcErrorCode::LevelUnavailable,
            Some(RpcErrorDetails::LevelUnavailable),
        ),
    };
    let data = RpcErrorData::new(code, details);
    ErrorObjectOwned::owned(code.json_rpc_code(), code.message(), Some(data))
}

fn is_local_development_origin(origin: &HeaderValue) -> bool {
    let Ok(origin) = origin.to_str() else {
        return false;
    };
    let Some(authority) = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
    else {
        return false;
    };
    if authority.contains('/') || authority.contains('@') {
        return false;
    }
    if authority == "localhost" {
        return true;
    }
    if let Some(port) = authority.strip_prefix("localhost:") {
        return port.parse::<u16>().is_ok();
    }
    if let Ok(ip) = authority.parse::<IpAddr>() {
        return ip.is_loopback();
    }
    if let Ok(address) = authority.parse::<SocketAddr>() {
        return address.ip().is_loopback();
    }

    warn!(%origin, "rejected non-local CORS origin");
    false
}

fn is_same_origin_rpc_request(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(ORIGIN) else {
        return true;
    };
    let (Ok(origin), Some(host)) = (origin.to_str(), headers.get(HOST)) else {
        return false;
    };
    let Ok(host) = host.to_str() else {
        return false;
    };
    let Some(origin_authority) = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
    else {
        return false;
    };
    !origin_authority.contains('/') && origin_authority.eq_ignore_ascii_case(host)
}

#[cfg(test)]
mod tests {
    use std::{fs, time::SystemTime};

    use axum::{
        body::Body,
        http::{HeaderValue, Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::{
        build_application_with_config, is_local_development_origin, is_same_origin_rpc_request,
    };

    #[test]
    fn cors_accepts_only_loopback_origins() {
        assert!(is_local_development_origin(&HeaderValue::from_static(
            "http://localhost:5173"
        )));
        assert!(is_local_development_origin(&HeaderValue::from_static(
            "https://127.0.0.1:8443"
        )));
        assert!(!is_local_development_origin(&HeaderValue::from_static(
            "https://example.com"
        )));
        assert!(!is_local_development_origin(&HeaderValue::from_static(
            "http://localhost.example.com"
        )));
    }

    #[test]
    fn rpc_origin_must_match_the_request_host_when_present() {
        let mut headers = axum::http::HeaderMap::new();
        assert!(is_same_origin_rpc_request(&headers));
        headers.insert(
            axum::http::header::HOST,
            HeaderValue::from_static("oreak.test"),
        );
        headers.insert(
            axum::http::header::ORIGIN,
            HeaderValue::from_static("https://oreak.test"),
        );
        assert!(is_same_origin_rpc_request(&headers));
        headers.insert(
            axum::http::header::ORIGIN,
            HeaderValue::from_static("https://attacker.test"),
        );
        assert!(!is_same_origin_rpc_request(&headers));
    }

    #[tokio::test]
    async fn serves_spa_fallback_when_compiled_assets_exist() {
        let unique = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dist = std::env::temp_dir().join(format!("oreak-web-dist-{unique}"));
        fs::create_dir_all(&dist).unwrap();
        fs::write(dist.join("index.html"), "<main>Oreak web app</main>").unwrap();
        fs::write(dist.join("app.js"), "console.log('oreak');").unwrap();
        let router = build_application_with_config(dist.clone(), false).into_router();

        let fallback = router
            .clone()
            .oneshot(
                Request::get("/projects/example")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(fallback.status(), StatusCode::OK);
        let body = fallback.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"<main>Oreak web app</main>");

        let asset = router
            .oneshot(Request::get("/app.js").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(asset.status(), StatusCode::OK);
        let body = asset.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"console.log('oreak');");
        fs::remove_dir_all(dist).unwrap();
    }

    #[tokio::test]
    async fn retains_diagnostic_root_when_compiled_assets_are_absent() {
        let unique = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let missing = std::env::temp_dir().join(format!("oreak-web-dist-missing-{unique}"));
        let response = build_application_with_config(missing, false)
            .into_router()
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert!(String::from_utf8_lossy(&body).contains("Non-production demo"));
    }
}

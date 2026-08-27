use std::{net::SocketAddr, time::Duration};

use axum::{
    Router,
    body::Body,
    http::{
        HeaderMap, HeaderValue, Method, Request, Response, StatusCode,
        header::{CONTENT_TYPE, COOKIE, SET_COOKIE},
    },
};
use http_body_util::BodyExt;
use jsonrpsee::{
    core::client::Error as ClientError,
    server::ServerHandle,
    ws_client::{WsClient, WsClientBuilder},
};
use oreak_core::{CellKind, CommandEnvelope, CommandMetadata, GridPoint, LevelCommand};
use oreak_protocol::{
    ApplyCommandRequest, ApplyCommandResult, HealthResponse, HealthStatus, LevelHistoryRequest,
    LevelSubscriptionItem, OreakRpcClient, ProjectLevelTarget, RpcErrorCode, RpcErrorData,
    UndoLatestRequest,
};
use oreak_server::build_application;
use tokio::{net::TcpListener, task::JoinHandle, time::timeout};
use tower::ServiceExt;

const TEST_PASSWORD: &str = "correct horse battery staple";

struct TestServer {
    address: SocketAddr,
    router: Router,
    rpc_handle: ServerHandle,
    task: JoinHandle<()>,
}

impl TestServer {
    async fn start() -> Self {
        let application = build_application();
        let rpc_handle = application.rpc_handle();
        let router = application.into_router();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let served_router = router.clone();
        let task = tokio::spawn(async move {
            axum::serve(listener, served_router).await.unwrap();
        });
        Self {
            address,
            router,
            rpc_handle,
            task,
        }
    }

    async fn client(&self, cookie: Option<&str>) -> WsClient {
        let mut headers = HeaderMap::new();
        if let Some(cookie) = cookie {
            headers.insert(COOKIE, HeaderValue::from_str(cookie).unwrap());
        }
        WsClientBuilder::default()
            .set_headers(headers)
            .build(format!("ws://{}/rpc", self.address))
            .await
            .unwrap()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        let _ = self.rpc_handle.stop();
        self.task.abort();
    }
}

fn set_cell(command_id: &str, point: GridPoint, kind: CellKind) -> CommandEnvelope {
    CommandEnvelope::new(
        CommandMetadata::new(command_id, "alice", 1_000),
        LevelCommand::SetCell { point, kind },
    )
}

async fn api_request(
    router: &Router,
    method: Method,
    uri: &str,
    body: Option<serde_json::Value>,
    cookie: Option<&str>,
) -> Response<Body> {
    let mut request = Request::builder().method(method).uri(uri);
    if body.is_some() {
        request = request.header(CONTENT_TYPE, "application/json");
    }
    if let Some(cookie) = cookie {
        request = request.header(COOKIE, cookie);
    }
    router
        .clone()
        .oneshot(
            request
                .body(body.map_or_else(Body::empty, |value| Body::from(value.to_string())))
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn json_body(response: Response<Body>) -> serde_json::Value {
    let body = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&body).unwrap()
}

fn session_cookie(response: &Response<Body>) -> String {
    response
        .headers()
        .get(SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned()
}

async fn register(router: &Router, email: &str) -> (String, serde_json::Value) {
    let response = api_request(
        router,
        Method::POST,
        "/api/auth/register",
        Some(serde_json::json!({ "email": email, "password": TEST_PASSWORD })),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let cookie = session_cookie(&response);
    let body = json_body(response).await;
    (cookie, body)
}

async fn create_project_level(
    router: &Router,
    email: &str,
) -> (String, ProjectLevelTarget, serde_json::Value) {
    let (cookie, user) = register(router, email).await;
    let project_response = api_request(
        router,
        Method::POST,
        "/api/projects",
        Some(serde_json::json!({
            "workspace_id": user["personal_workspace_id"],
            "name": "RPC test project"
        })),
        Some(&cookie),
    )
    .await;
    assert_eq!(project_response.status(), StatusCode::CREATED);
    let project = json_body(project_response).await;
    let project_id = project["id"].as_str().unwrap();
    let level_response = api_request(
        router,
        Method::POST,
        &format!("/api/projects/{project_id}/levels"),
        Some(serde_json::json!({ "name": "RPC test level" })),
        Some(&cookie),
    )
    .await;
    assert_eq!(level_response.status(), StatusCode::CREATED);
    let level = json_body(level_response).await;
    let target = ProjectLevelTarget::new(project_id, level["id"].as_str().unwrap());
    (cookie, target, user)
}

#[tokio::test]
async fn rest_health_reports_ready() {
    let response = build_application()
        .into_router()
        .oneshot(Request::get("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let health: HealthResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(health.status, HealthStatus::Ok);
}

#[tokio::test]
async fn registration_normalizes_email_sets_cookie_and_logout_revokes_session() {
    let router = build_application().into_router();
    let response = api_request(
        &router,
        Method::POST,
        "/api/auth/register",
        Some(serde_json::json!({
            "email": "  Alice@Example.COM ",
            "password": TEST_PASSWORD
        })),
        None,
    )
    .await;

    assert_eq!(response.status(), StatusCode::CREATED);
    let set_cookie = response
        .headers()
        .get(SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(set_cookie.contains("HttpOnly"));
    assert!(set_cookie.contains("SameSite=Lax"));
    assert!(set_cookie.contains("Max-Age=604800"));
    assert!(set_cookie.contains("Expires="));
    let cookie = session_cookie(&response);
    let user = json_body(response).await;
    assert_eq!(user["email"], "alice@example.com");

    let me = api_request(&router, Method::GET, "/api/auth/me", None, Some(&cookie)).await;
    assert_eq!(me.status(), StatusCode::OK);
    assert_eq!(json_body(me).await["email"], "alice@example.com");

    let logout = api_request(
        &router,
        Method::POST,
        "/api/auth/logout",
        None,
        Some(&cookie),
    )
    .await;
    assert_eq!(logout.status(), StatusCode::NO_CONTENT);
    assert!(
        logout
            .headers()
            .get(SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );

    let revoked = api_request(&router, Method::GET, "/api/auth/me", None, Some(&cookie)).await;
    assert_eq!(revoked.status(), StatusCode::UNAUTHORIZED);

    let login = api_request(
        &router,
        Method::POST,
        "/api/auth/login",
        Some(serde_json::json!({
            "email": "ALICE@example.com",
            "password": TEST_PASSWORD
        })),
        None,
    )
    .await;
    assert_eq!(login.status(), StatusCode::OK);
    let new_cookie = session_cookie(&login);
    assert_ne!(new_cookie, cookie);
    let me = api_request(
        &router,
        Method::GET,
        "/api/auth/me",
        None,
        Some(&new_cookie),
    )
    .await;
    assert_eq!(me.status(), StatusCode::OK);
}

#[tokio::test]
async fn login_errors_are_generic_and_attempts_are_rate_limited() {
    let router = build_application().into_router();
    register(&router, "login@example.com").await;

    let wrong_password = api_request(
        &router,
        Method::POST,
        "/api/auth/login",
        Some(serde_json::json!({
            "email": "login@example.com",
            "password": "a sufficiently long wrong password"
        })),
        None,
    )
    .await;
    assert_eq!(wrong_password.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        json_body(wrong_password).await["error"],
        "invalid email or password"
    );

    let unknown_email = api_request(
        &router,
        Method::POST,
        "/api/auth/login",
        Some(serde_json::json!({
            "email": "unknown@example.com",
            "password": TEST_PASSWORD
        })),
        None,
    )
    .await;
    assert_eq!(unknown_email.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        json_body(unknown_email).await["error"],
        "invalid email or password"
    );

    for attempt in 0..6 {
        let response = api_request(
            &router,
            Method::POST,
            "/api/auth/login",
            Some(serde_json::json!({
                "email": "limited@example.com",
                "password": "short"
            })),
            None,
        )
        .await;
        if attempt < 5 {
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        } else {
            assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
            assert_eq!(response.headers()["retry-after"], "60");
        }
    }
}

#[tokio::test]
async fn project_routes_enforce_membership_capabilities() {
    let router = build_application().into_router();
    let (owner_cookie, owner) = register(&router, "owner@example.com").await;
    let (outsider_cookie, _) = register(&router, "outsider@example.com").await;
    let workspace_id = owner["personal_workspace_id"].as_str().unwrap();

    let project_response = api_request(
        &router,
        Method::POST,
        "/api/projects",
        Some(serde_json::json!({
            "workspace_id": workspace_id,
            "name": "First project"
        })),
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(project_response.status(), StatusCode::CREATED);
    let project = json_body(project_response).await;
    let project_id = project["id"].as_str().unwrap();
    assert_eq!(project["plugin_manifest_format_version"], 1);

    let level_uri = format!("/api/projects/{project_id}/levels");
    let level = api_request(
        &router,
        Method::POST,
        &level_uri,
        Some(serde_json::json!({ "name": "Opening" })),
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(level.status(), StatusCode::CREATED);

    let denied = api_request(
        &router,
        Method::POST,
        &level_uri,
        Some(serde_json::json!({ "name": "Unauthorized" })),
        Some(&outsider_cookie),
    )
    .await;
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);

    let members_uri = format!("/api/projects/{project_id}/members");
    let members = api_request(
        &router,
        Method::GET,
        &members_uri,
        None,
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(members.status(), StatusCode::OK);
    let members = json_body(members).await;
    assert_eq!(members["members"][0]["roles"][0], "owner");
    assert!(
        members["members"][0]["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|capability| capability == "edit_timeline")
    );

    let channels_uri = format!("/api/projects/{project_id}/release-channels");
    let channel = api_request(
        &router,
        Method::POST,
        &channels_uri,
        Some(serde_json::json!({ "name": "Nightly" })),
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(channel.status(), StatusCode::CREATED);

    let audit_uri = format!("/api/projects/{project_id}/audit");
    let audit = api_request(&router, Method::GET, &audit_uri, None, Some(&owner_cookie)).await;
    assert_eq!(audit.status(), StatusCode::OK);
    assert!(json_body(audit).await["audit"].as_array().unwrap().len() >= 3);

    let denied_audit = api_request(
        &router,
        Method::GET,
        &audit_uri,
        None,
        Some(&outsider_cookie),
    )
    .await;
    assert_eq!(denied_audit.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn json_rpc_health_accepts_http() {
    let application = build_application();
    let _rpc_handle = application.rpc_handle();
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "health",
        "params": []
    });
    let response = application
        .into_router()
        .oneshot(
            Request::post("/rpc")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(request.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let response: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(response["result"]["status"], "ok");
}

#[tokio::test]
async fn authenticated_project_level_starts_as_an_eight_by_eight_snapshot() {
    let server = TestServer::start().await;
    let (cookie, target, _) = create_project_level(&server.router, "snapshot@example.com").await;
    let client = server.client(Some(&cookie)).await;
    let snapshot = client.level_snapshot(target.clone()).await.unwrap();

    assert_eq!(snapshot.target, target);
    assert_eq!(snapshot.server_sequence, 0);
    assert_eq!(snapshot.snapshot.size().width(), 8);
    assert_eq!(snapshot.snapshot.size().height(), 8);
    assert_eq!(snapshot.level_hash.len(), 64);
}

#[tokio::test]
async fn apply_command_updates_the_authoritative_snapshot() {
    let server = TestServer::start().await;
    let (cookie, target, user) = create_project_level(&server.router, "apply@example.com").await;
    let client = server.client(Some(&cookie)).await;
    let point = GridPoint::new(2, 3);
    let response = client
        .apply_command(ApplyCommandRequest {
            target: target.clone(),
            command: set_cell("apply-1", point, CellKind::Wall),
        })
        .await
        .unwrap();

    assert_eq!(response.server_sequence, 1);
    assert!(matches!(
        response.result,
        ApplyCommandResult::Applied { ref event }
            if event.metadata.actor.as_str() == user["id"].as_str().unwrap()
                && event.metadata.occurred_at_ms != 1_000
    ));
    let snapshot = client.level_snapshot(target).await.unwrap();
    assert_eq!(snapshot.server_sequence, 1);
    assert_eq!(snapshot.snapshot.cell(point), Ok(CellKind::Wall));
    assert_eq!(snapshot.level_hash, response.level_hash);
}

#[tokio::test]
async fn undo_uses_the_authenticated_actor_instead_of_client_metadata() {
    let server = TestServer::start().await;
    let (cookie, target, user) = create_project_level(&server.router, "undo@example.com").await;
    let client = server.client(Some(&cookie)).await;
    let point = GridPoint::new(1, 1);
    client
        .apply_command(ApplyCommandRequest {
            target: target.clone(),
            command: set_cell("undo-source", point, CellKind::Wall),
        })
        .await
        .unwrap();

    let response = client
        .undo_latest(UndoLatestRequest {
            target: target.clone(),
            metadata: CommandMetadata::new("undo-spoof", "another-user", 1),
        })
        .await
        .unwrap();

    assert_eq!(
        response.event.metadata.actor.as_str(),
        user["id"].as_str().unwrap()
    );
    assert_eq!(response.event.reverts_sequence, Some(1));
    let snapshot = client.level_snapshot(target).await.unwrap();
    assert_eq!(snapshot.snapshot.cell(point), Ok(CellKind::Floor));
}

#[tokio::test]
async fn duplicate_accepted_command_returns_structured_error() {
    let server = TestServer::start().await;
    let (cookie, target, _) = create_project_level(&server.router, "duplicate@example.com").await;
    let client = server.client(Some(&cookie)).await;
    client
        .apply_command(ApplyCommandRequest {
            target: target.clone(),
            command: set_cell("duplicate-1", GridPoint::new(0, 0), CellKind::Wall),
        })
        .await
        .unwrap();

    let error = client
        .apply_command(ApplyCommandRequest {
            target,
            command: set_cell("duplicate-1", GridPoint::new(1, 0), CellKind::Wall),
        })
        .await
        .unwrap_err();
    let ClientError::Call(error) = error else {
        panic!("expected a JSON-RPC call error");
    };
    assert_eq!(error.code(), RpcErrorCode::DuplicateCommand.json_rpc_code());
    let data: RpcErrorData = serde_json::from_str(error.data().unwrap().get()).unwrap();
    assert_eq!(data.code, RpcErrorCode::DuplicateCommand);
    assert_eq!(data.message, RpcErrorCode::DuplicateCommand.message());
}

#[tokio::test]
async fn subscription_starts_with_snapshot_and_skips_no_ops() {
    let server = TestServer::start().await;
    let (cookie, target, _) =
        create_project_level(&server.router, "subscription@example.com").await;
    let client = server.client(Some(&cookie)).await;
    let mut subscription = client.subscribe_level(target.clone()).await.unwrap();

    let first = timeout(Duration::from_secs(1), subscription.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(
        first,
        LevelSubscriptionItem::Snapshot { snapshot } if snapshot.server_sequence == 0
    ));

    let no_op = client
        .apply_command(ApplyCommandRequest {
            target: target.clone(),
            command: set_cell("no-op", GridPoint::new(0, 0), CellKind::Floor),
        })
        .await
        .unwrap();
    assert_eq!(no_op.result, ApplyCommandResult::NoChange);
    assert!(
        timeout(Duration::from_millis(100), subscription.next())
            .await
            .is_err()
    );

    client
        .apply_command(ApplyCommandRequest {
            target,
            command: set_cell("applied", GridPoint::new(0, 0), CellKind::Wall),
        })
        .await
        .unwrap();
    let update = timeout(Duration::from_secs(1), subscription.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(
        update,
        LevelSubscriptionItem::Event { event } if event.server_sequence == 1
    ));
}

#[tokio::test]
async fn level_history_is_newest_first_paginated_scoped_and_canonical() {
    let server = TestServer::start().await;
    let (cookie, target, user) = create_project_level(&server.router, "history@example.com").await;
    let client = server.client(Some(&cookie)).await;
    for sequence in 1..=5_u16 {
        client
            .apply_command(ApplyCommandRequest {
                target: target.clone(),
                command: set_cell(
                    &format!("history-{sequence}"),
                    GridPoint::new(sequence - 1, 0),
                    CellKind::Wall,
                ),
            })
            .await
            .unwrap();
    }

    let first = client
        .level_history(LevelHistoryRequest {
            target: target.clone(),
            before_sequence: Some(6),
            limit: 2,
        })
        .await
        .unwrap();
    assert_eq!(first.target, target);
    assert_eq!(
        first
            .events
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        vec![5, 4]
    );
    assert!(first.has_more);
    assert_eq!(first.next_before_sequence, Some(4));
    assert!(first.events.iter().all(|event| {
        event.metadata.actor.as_str() == user["id"].as_str().unwrap()
            && event.metadata.occurred_at_ms != 1_000
    }));

    let second = client
        .level_history(LevelHistoryRequest {
            target: target.clone(),
            before_sequence: first.next_before_sequence,
            limit: 2,
        })
        .await
        .unwrap();
    assert_eq!(
        second
            .events
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        vec![3, 2]
    );
    assert!(second.has_more);
    assert_eq!(second.next_before_sequence, Some(2));

    let third = client
        .level_history(LevelHistoryRequest {
            target: target.clone(),
            before_sequence: second.next_before_sequence,
            limit: 2,
        })
        .await
        .unwrap();
    assert_eq!(
        third
            .events
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        vec![1]
    );
    assert!(!third.has_more);
    assert_eq!(third.next_before_sequence, None);
    let all_sequences: Vec<_> = first
        .events
        .iter()
        .chain(&second.events)
        .chain(&third.events)
        .map(|event| event.sequence)
        .collect();
    assert_eq!(all_sequences, vec![5, 4, 3, 2, 1]);

    let second_level = api_request(
        &server.router,
        Method::POST,
        &format!("/api/projects/{}/levels", target.project_id),
        Some(serde_json::json!({ "name": "Other level" })),
        Some(&cookie),
    )
    .await;
    assert_eq!(second_level.status(), StatusCode::CREATED);
    let second_level = json_body(second_level).await;
    let other_target = ProjectLevelTarget::new(
        target.project_id.clone(),
        second_level["id"].as_str().unwrap(),
    );
    let other_history = client
        .level_history(LevelHistoryRequest {
            target: other_target.clone(),
            before_sequence: None,
            limit: 10,
        })
        .await
        .unwrap();
    assert_eq!(other_history.target, other_target);
    assert!(other_history.events.is_empty());

    let error = client
        .level_history(LevelHistoryRequest {
            target,
            before_sequence: None,
            limit: 0,
        })
        .await
        .unwrap_err();
    let ClientError::Call(error) = error else {
        panic!("expected a JSON-RPC call error");
    };
    assert_eq!(error.code(), RpcErrorCode::InvalidCommand.json_rpc_code());
}

#[tokio::test]
async fn collaboration_requires_a_live_authorized_session() {
    let server = TestServer::start().await;
    let (owner_cookie, target, _) =
        create_project_level(&server.router, "rpc-owner@example.com").await;
    let (outsider_cookie, _) = register(&server.router, "rpc-outsider@example.com").await;

    for cookie in [None, Some(outsider_cookie.as_str())] {
        let client = server.client(cookie).await;
        let error = client.level_snapshot(target.clone()).await.unwrap_err();
        let ClientError::Call(error) = error else {
            panic!("expected a JSON-RPC call error");
        };
        let expected = if cookie.is_some() {
            RpcErrorCode::PermissionDenied
        } else {
            RpcErrorCode::AuthenticationRequired
        };
        assert_eq!(error.code(), expected.json_rpc_code());

        let error = client
            .level_history(LevelHistoryRequest {
                target: target.clone(),
                before_sequence: None,
                limit: 10,
            })
            .await
            .unwrap_err();
        let ClientError::Call(error) = error else {
            panic!("expected a JSON-RPC call error");
        };
        assert_eq!(error.code(), expected.json_rpc_code());
    }

    let owner = server.client(Some(&owner_cookie)).await;
    owner.level_snapshot(target.clone()).await.unwrap();
    let logout = api_request(
        &server.router,
        Method::POST,
        "/api/auth/logout",
        None,
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(logout.status(), StatusCode::NO_CONTENT);
    let error = owner.level_snapshot(target).await.unwrap_err();
    let ClientError::Call(error) = error else {
        panic!("expected a JSON-RPC call error");
    };
    assert_eq!(
        error.code(),
        RpcErrorCode::AuthenticationRequired.json_rpc_code()
    );
}

use std::{io::Cursor, net::SocketAddr, time::Duration};

use axum::{
    Router,
    body::Body,
    http::{
        HeaderMap, HeaderValue, Method, Request, Response, StatusCode,
        header::{CONTENT_TYPE, COOKIE, SET_COOKIE},
    },
};
use http_body_util::BodyExt;
use image::{
    DynamicImage, ExtendedColorType, ImageBuffer, ImageEncoder, ImageFormat, Rgba,
    codecs::png::PngEncoder,
};
use jsonrpsee::{
    core::client::Error as ClientError,
    server::ServerHandle,
    ws_client::{WsClient, WsClientBuilder},
};
use oreak_core::{
    Blind, BlindPixel, BlindStroke, BlindTile, Block, CellKind, CollectCapacity, CollectLayer,
    CommandEnvelope, CommandMetadata, DecoratorId, DistributionLayerSelection,
    DistributionPoolSelection, DistributionRequest, EntityId, EntityMove, GridAnchor, GridPoint,
    GridSize, LevelCommand, PlaceableEntity, PlaceableEntityKind, PoolBoundary,
    PoolDistributionGroup, Shape,
};
use oreak_protocol::{
    ApplyCommandRequest, ApplyCommandResult, HealthResponse, HealthStatus, LevelHistoryRequest,
    LevelPresenceItem, LevelSubscriptionItem, OreakRpcClient, ProjectLevelTarget, RpcErrorCode,
    RpcErrorData, UndoLatestRequest, UpdateLevelCursorRequest,
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

async fn binary_request(
    router: &Router,
    method: Method,
    uri: &str,
    content_type: &str,
    body: Vec<u8>,
    cookie: Option<&str>,
) -> Response<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header(CONTENT_TYPE, content_type);
    if let Some(cookie) = cookie {
        request = request.header(COOKIE, cookie);
    }
    router
        .clone()
        .oneshot(request.body(Body::from(body)).unwrap())
        .await
        .unwrap()
}

fn rgba_png(width: u32, height: u32) -> Vec<u8> {
    let mut png = Vec::new();
    let pixels = vec![0_u8; width as usize * height as usize * 4];
    PngEncoder::new(&mut png)
        .write_image(&pixels, width, height, ExtendedColorType::Rgba8)
        .unwrap();
    png
}

fn encoded_rgba_image(format: ImageFormat) -> Vec<u8> {
    encoded_rgba_image_at_size(format, 2, 1)
}

fn encoded_rgba_image_at_size(format: ImageFormat, width: u32, height: u32) -> Vec<u8> {
    let image = DynamicImage::ImageRgba8(ImageBuffer::from_pixel(
        width,
        height,
        Rgba([24, 96, 176, 255]),
    ));
    let mut encoded = Cursor::new(Vec::new());
    image.write_to(&mut encoded, format).unwrap();
    encoded.into_inner()
}

fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut chunk = Vec::with_capacity(12 + data.len());
    chunk.extend_from_slice(&(data.len() as u32).to_be_bytes());
    chunk.extend_from_slice(kind);
    chunk.extend_from_slice(data);
    let crc = png_crc32(&chunk[4..]);
    chunk.extend_from_slice(&crc.to_be_bytes());
    chunk
}

fn png_crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 0 {
                crc >> 1
            } else {
                (crc >> 1) ^ 0xedb8_8320
            };
        }
    }
    !crc
}

fn animated_png() -> Vec<u8> {
    let png = rgba_png(1, 1);
    let ihdr_end = 8 + 4 + 4 + 13 + 4;
    let mut animated = png[..ihdr_end].to_vec();
    animated.extend(png_chunk(b"acTL", &[0, 0, 0, 1, 0, 0, 0, 0]));
    animated.extend(png_chunk(
        b"fcTL",
        &[
            0, 0, 0, 0, // sequence number
            0, 0, 0, 1, // width
            0, 0, 0, 1, // height
            0, 0, 0, 0, // x offset
            0, 0, 0, 0, // y offset
            0, 1, // delay numerator
            0, 10, // delay denominator
            0,  // dispose op
            0,  // blend op
        ],
    ));
    animated.extend(&png[ihdr_end..]);
    animated
}

fn riff_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut chunk = Vec::with_capacity(8 + data.len() + data.len() % 2);
    chunk.extend_from_slice(kind);
    chunk.extend_from_slice(&(data.len() as u32).to_le_bytes());
    chunk.extend_from_slice(data);
    if data.len() % 2 != 0 {
        chunk.push(0);
    }
    chunk
}

fn animated_webp() -> Vec<u8> {
    let still = encoded_rgba_image(ImageFormat::WebP);
    let frame_image_chunk = &still[12..];
    let mut anmf = vec![
        0, 0, 0, // x offset
        0, 0, 0, // y offset
        1, 0, 0, // width minus one
        0, 0, 0, // height minus one
        1, 0, 0, // duration
        0, // flags
    ];
    anmf.extend_from_slice(frame_image_chunk);

    let mut payload = Vec::new();
    payload.extend(riff_chunk(
        b"VP8X",
        &[
            0b0000_0010,
            0,
            0,
            0, // animation flags and reserved bytes
            1,
            0,
            0, // canvas width minus one
            0,
            0,
            0, // canvas height minus one
        ],
    ));
    payload.extend(riff_chunk(b"ANIM", &[0, 0, 0, 0, 0, 0]));
    payload.extend(riff_chunk(b"ANMF", &anmf));

    let mut animated = Vec::with_capacity(12 + payload.len());
    animated.extend_from_slice(b"RIFF");
    animated.extend_from_slice(&((4 + payload.len()) as u32).to_le_bytes());
    animated.extend_from_slice(b"WEBP");
    animated.extend(payload);
    animated
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
async fn level_configuration_creation_update_validation_and_authorization() {
    let router = build_application().into_router();
    let (owner_cookie, owner) = register(&router, "level-config-owner@example.com").await;
    let (viewer_cookie, _) = register(&router, "level-config-viewer@example.com").await;
    let (outsider_cookie, _) = register(&router, "level-config-outsider@example.com").await;

    let project_response = api_request(
        &router,
        Method::POST,
        "/api/projects",
        Some(serde_json::json!({
            "workspace_id": owner["personal_workspace_id"],
            "name": "Configured levels"
        })),
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(project_response.status(), StatusCode::CREATED);
    let project = json_body(project_response).await;
    let project_id = project["id"].as_str().unwrap();
    let levels_uri = format!("/api/projects/{project_id}/levels");

    let created = api_request(
        &router,
        Method::POST,
        &levels_uri,
        Some(serde_json::json!({
            "name": "  Opening  ",
            "duration_seconds": 12.5
        })),
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let created = json_body(created).await;
    assert_eq!(created["name"], "Opening");
    assert_eq!(created["duration_seconds"], 12.5);
    let level_id = created["id"].as_str().unwrap();
    let configuration_uri = format!("/api/projects/{project_id}/levels/{level_id}/configuration");

    let default_duration = api_request(
        &router,
        Method::POST,
        &levels_uri,
        Some(serde_json::json!({ "name": "Untimed" })),
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(default_duration.status(), StatusCode::CREATED);
    assert_eq!(json_body(default_duration).await["duration_seconds"], 0.0);

    let listed = api_request(&router, Method::GET, &levels_uri, None, Some(&owner_cookie)).await;
    assert_eq!(listed.status(), StatusCode::OK);
    let listed = json_body(listed).await;
    let opening = listed["levels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|level| level["id"] == level_id)
        .unwrap();
    assert_eq!(opening["name"], "Opening");
    assert_eq!(opening["duration_seconds"], 12.5);

    let invitation = api_request(
        &router,
        Method::POST,
        &format!("/api/projects/{project_id}/invitations"),
        Some(serde_json::json!({
            "email": "level-config-viewer@example.com",
            "roles": ["viewer"]
        })),
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(invitation.status(), StatusCode::CREATED);
    let invitation = json_body(invitation).await;
    let invitation_id = invitation["id"].as_str().unwrap();
    let accepted = api_request(
        &router,
        Method::POST,
        &format!("/api/projects/{project_id}/invitations/{invitation_id}/accept"),
        None,
        Some(&viewer_cookie),
    )
    .await;
    assert_eq!(accepted.status(), StatusCode::OK);

    let viewed = api_request(
        &router,
        Method::GET,
        &configuration_uri,
        None,
        Some(&viewer_cookie),
    )
    .await;
    assert_eq!(viewed.status(), StatusCode::OK);
    assert_eq!(json_body(viewed).await["duration_seconds"], 12.5);

    let viewer_update = api_request(
        &router,
        Method::PUT,
        &configuration_uri,
        Some(serde_json::json!({
            "name": "Denied",
            "duration_seconds": 1.0
        })),
        Some(&viewer_cookie),
    )
    .await;
    assert_eq!(viewer_update.status(), StatusCode::FORBIDDEN);
    let outsider_view = api_request(
        &router,
        Method::GET,
        &configuration_uri,
        None,
        Some(&outsider_cookie),
    )
    .await;
    assert_eq!(outsider_view.status(), StatusCode::FORBIDDEN);

    let invalid = api_request(
        &router,
        Method::PUT,
        &configuration_uri,
        Some(serde_json::json!({
            "name": "Invalid",
            "duration_seconds": -1.0
        })),
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

    for (method, uri, body) in [
        (
            Method::POST,
            levels_uri.as_str(),
            serde_json::json!({ "name": "Extra", "unknown": true }),
        ),
        (
            Method::PUT,
            configuration_uri.as_str(),
            serde_json::json!({
                "name": "Extra",
                "duration_seconds": 1.0,
                "unknown": true
            }),
        ),
    ] {
        let response = api_request(&router, method, uri, Some(body), Some(&owner_cookie)).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    let updated = api_request(
        &router,
        Method::PUT,
        &configuration_uri,
        Some(serde_json::json!({
            "name": "  Finale  ",
            "duration_seconds": 30.0
        })),
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(updated.status(), StatusCode::OK);
    let updated = json_body(updated).await;
    assert_eq!(updated["name"], "Finale");
    assert_eq!(updated["duration_seconds"], 30.0);

    let audit = api_request(
        &router,
        Method::GET,
        &format!("/api/projects/{project_id}/audit"),
        None,
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(audit.status(), StatusCode::OK);
    assert!(
        json_body(audit).await["audit"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["action"]["LevelConfigurationChanged"]["level_id"] == level_id)
    );
}

#[tokio::test]
async fn project_and_level_deletion_are_audited_and_idempotently_missing() {
    let router = build_application().into_router();
    let (cookie, target, user) =
        create_project_level(&router, "delete-resources@example.com").await;
    let project_id = target.project_id.to_string();
    let level_id = target.level_id.to_string();
    let level_uri = format!("/api/projects/{project_id}/levels/{level_id}");

    let deleted_level = api_request(&router, Method::DELETE, &level_uri, None, Some(&cookie)).await;
    assert_eq!(deleted_level.status(), StatusCode::NO_CONTENT);
    let missing_level = api_request(&router, Method::DELETE, &level_uri, None, Some(&cookie)).await;
    assert_eq!(missing_level.status(), StatusCode::NOT_FOUND);
    let project_audit = api_request(
        &router,
        Method::GET,
        &format!("/api/projects/{project_id}/audit"),
        None,
        Some(&cookie),
    )
    .await;
    assert!(
        json_body(project_audit).await["audit"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["action"]["LevelDeleted"]["level_id"] == level_id)
    );

    let project_uri = format!("/api/projects/{project_id}");
    let deleted_project =
        api_request(&router, Method::DELETE, &project_uri, None, Some(&cookie)).await;
    assert_eq!(deleted_project.status(), StatusCode::NO_CONTENT);
    let missing_project =
        api_request(&router, Method::DELETE, &project_uri, None, Some(&cookie)).await;
    assert_eq!(missing_project.status(), StatusCode::NOT_FOUND);
    let workspace_id = user["personal_workspace_id"].as_str().unwrap();
    let workspace_audit = api_request(
        &router,
        Method::GET,
        &format!("/api/workspaces/{workspace_id}/audit"),
        None,
        Some(&cookie),
    )
    .await;
    assert!(
        json_body(workspace_audit).await["audit"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["action"]["ProjectUnregistered"]["project_id"] == project_id)
    );
}

#[tokio::test]
async fn project_catalog_invitations_configuration_and_roles_are_authorized() {
    let router = build_application().into_router();
    let (owner_cookie, owner) = register(&router, "catalog-owner@example.com").await;
    let (invitee_cookie, invitee) = register(&router, "catalog-invitee@example.com").await;
    let (outsider_cookie, _) = register(&router, "catalog-outsider@example.com").await;

    let project_response = api_request(
        &router,
        Method::POST,
        "/api/projects",
        Some(serde_json::json!({
            "workspace_id": owner["personal_workspace_id"],
            "name": "Shared catalog project"
        })),
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(project_response.status(), StatusCode::CREATED);
    let project = json_body(project_response).await;
    let project_id = project["id"].as_str().unwrap();

    let level_response = api_request(
        &router,
        Method::POST,
        &format!("/api/projects/{project_id}/levels"),
        Some(serde_json::json!({ "name": "Shared level" })),
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(level_response.status(), StatusCode::CREATED);

    let invitation_response = api_request(
        &router,
        Method::POST,
        &format!("/api/projects/{project_id}/invitations"),
        Some(serde_json::json!({
            "email": "  CATALOG-INVITEE@example.com ",
            "roles": ["editor"]
        })),
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(invitation_response.status(), StatusCode::CREATED);
    let invitation = json_body(invitation_response).await;
    assert_eq!(invitation["invitee_email"], "catalog-invitee@example.com");
    let invitation_id = invitation["id"].as_str().unwrap();
    let accept_uri = format!("/api/projects/{project_id}/invitations/{invitation_id}/accept");

    let invitee_catalog = api_request(
        &router,
        Method::GET,
        "/api/catalog",
        None,
        Some(&invitee_cookie),
    )
    .await;
    assert_eq!(invitee_catalog.status(), StatusCode::OK);
    let invitee_catalog = json_body(invitee_catalog).await;
    assert!(invitee_catalog["projects"].as_array().unwrap().is_empty());
    assert_eq!(invitee_catalog["invitations"][0]["id"], invitation_id);

    let denied_accept = api_request(
        &router,
        Method::POST,
        &accept_uri,
        None,
        Some(&outsider_cookie),
    )
    .await;
    assert_eq!(denied_accept.status(), StatusCode::FORBIDDEN);

    let accepted = api_request(
        &router,
        Method::POST,
        &accept_uri,
        None,
        Some(&invitee_cookie),
    )
    .await;
    assert_eq!(accepted.status(), StatusCode::OK);
    let accepted = json_body(accepted).await;
    assert_eq!(accepted["roles"][0], "editor");
    assert!(
        accepted["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|capability| capability == "edit_timeline")
    );

    let invitee_catalog = api_request(
        &router,
        Method::GET,
        "/api/catalog",
        None,
        Some(&invitee_cookie),
    )
    .await;
    assert_eq!(invitee_catalog.status(), StatusCode::OK);
    let invitee_catalog = json_body(invitee_catalog).await;
    assert!(
        invitee_catalog["invitations"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let shared_project = invitee_catalog["projects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["project"]["id"] == project_id)
        .unwrap();
    assert_eq!(shared_project["levels"][0]["name"], "Shared level");

    let configuration_uri = format!("/api/projects/{project_id}/configuration");
    let invitee_configuration = api_request(
        &router,
        Method::GET,
        &configuration_uri,
        None,
        Some(&invitee_cookie),
    )
    .await;
    assert_eq!(invitee_configuration.status(), StatusCode::OK);
    let invitee_configuration = json_body(invitee_configuration).await;
    assert_eq!(invitee_configuration["members_visible"], false);
    assert!(
        invitee_configuration["members"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let denied_configuration = api_request(
        &router,
        Method::GET,
        &configuration_uri,
        None,
        Some(&outsider_cookie),
    )
    .await;
    assert_eq!(denied_configuration.status(), StatusCode::FORBIDDEN);

    let member_id = invitee["id"].as_str().unwrap();
    let updated_member = api_request(
        &router,
        Method::PUT,
        &format!("/api/projects/{project_id}/members/{member_id}"),
        Some(serde_json::json!({ "roles": ["viewer"] })),
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(updated_member.status(), StatusCode::OK);
    assert_eq!(json_body(updated_member).await["roles"][0], "viewer");

    let denied_theme = api_request(
        &router,
        Method::PUT,
        &configuration_uri,
        Some(serde_json::json!({ "default_theme": "light" })),
        Some(&invitee_cookie),
    )
    .await;
    assert_eq!(denied_theme.status(), StatusCode::FORBIDDEN);

    let updated_theme = api_request(
        &router,
        Method::PUT,
        &configuration_uri,
        Some(serde_json::json!({ "default_theme": "light" })),
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(updated_theme.status(), StatusCode::OK);
    assert_eq!(json_body(updated_theme).await["default_theme"], "light");
}

#[tokio::test]
async fn project_shape_catalog_supports_validated_editor_crud() {
    let router = build_application().into_router();
    let (owner_cookie, target, _) = create_project_level(&router, "shape-owner@example.com").await;
    let (outsider_cookie, _) = register(&router, "shape-outsider@example.com").await;
    let catalog_uri = format!("/api/projects/{}/shapes", target.project_id);

    let defaults = api_request(
        &router,
        Method::GET,
        &catalog_uri,
        None,
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(defaults.status(), StatusCode::OK);
    let defaults = json_body(defaults).await;
    let defaults = defaults["shapes"].as_array().unwrap();
    assert_eq!(defaults.len(), 9);
    for height in 1..=3 {
        for width in 1..=3 {
            assert!(defaults.iter().any(|shape| {
                shape["shape"]["width"] == width
                    && shape["shape"]["height"] == height
                    && shape["shape"]["occupied_mask"] == (1_u64 << (width * height)) - 1
            }));
        }
    }

    let created = api_request(
        &router,
        Method::POST,
        &catalog_uri,
        Some(serde_json::json!({
            "name": "Corner",
            "shape": { "width": 2, "height": 2, "occupied_mask": 7 }
        })),
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let created = json_body(created).await;
    let shape_id = created["id"].as_str().unwrap();
    assert_eq!(created["name"], "Corner");
    assert_eq!(created["shape"]["occupied_mask"], 7);
    assert_eq!(created["created_by"], created["updated_by"]);

    let listed = api_request(
        &router,
        Method::GET,
        &catalog_uri,
        None,
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(listed.status(), StatusCode::OK);
    let listed = json_body(listed).await;
    assert!(
        listed["shapes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|shape| shape["id"] == shape_id)
    );

    let denied = api_request(
        &router,
        Method::GET,
        &catalog_uri,
        None,
        Some(&outsider_cookie),
    )
    .await;
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);

    let entry_uri = format!("{catalog_uri}/{shape_id}");
    let updated = api_request(
        &router,
        Method::PUT,
        &entry_uri,
        Some(serde_json::json!({
            "name": "Vertical",
            "shape": { "width": 1, "height": 2, "occupied_mask": 3 }
        })),
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(updated.status(), StatusCode::OK);
    let updated = json_body(updated).await;
    assert_eq!(updated["name"], "Vertical");
    assert_eq!(updated["shape"]["height"], 2);

    let invalid = api_request(
        &router,
        Method::POST,
        &catalog_uri,
        Some(serde_json::json!({
            "name": "Disconnected",
            "shape": { "width": 2, "height": 2, "occupied_mask": 9 }
        })),
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(invalid.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let oversized = api_request(
        &router,
        Method::POST,
        &catalog_uri,
        Some(serde_json::json!({
            "name": "Too wide",
            "shape": { "width": 64, "height": 1, "occupied_mask": u64::MAX }
        })),
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(oversized.status(), StatusCode::BAD_REQUEST);

    let deleted = api_request(
        &router,
        Method::DELETE,
        &entry_uri,
        None,
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    let listed = api_request(
        &router,
        Method::GET,
        &catalog_uri,
        None,
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(
        json_body(listed).await["shapes"].as_array().unwrap().len(),
        9
    );
}

#[tokio::test]
async fn project_image_catalog_imports_authorized_verified_png_templates() {
    const ONE_PIXEL_PNG: &[u8] = &[
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 4,
        0, 0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 100, 248, 15, 0, 1, 5,
        1, 1, 39, 24, 227, 102, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];

    let router = build_application().into_router();
    let (owner_cookie, target, _) = create_project_level(&router, "image-owner@example.com").await;
    let (outsider_cookie, _) = register(&router, "image-outsider@example.com").await;
    let catalog_uri = format!("/api/projects/{}/images", target.project_id);

    let rejected = binary_request(
        &router,
        Method::POST,
        &catalog_uri,
        "image/png",
        b"not a png".to_vec(),
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);

    let unsupported_gif = binary_request(
        &router,
        Method::POST,
        &catalog_uri,
        "image/gif",
        vec![
            71, 73, 70, 56, 57, 97, 1, 0, 1, 0, 128, 0, 0, 0, 0, 0, 255, 255, 255, 33, 249, 4, 1,
            0, 0, 0, 0, 44, 0, 0, 0, 0, 1, 0, 1, 0, 0, 2, 2, 68, 1, 0, 59,
        ],
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(unsupported_gif.status(), StatusCode::BAD_REQUEST);

    let above_json_limit = binary_request(
        &router,
        Method::POST,
        &catalog_uri,
        "image/png",
        vec![0; 20 * 1024],
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(above_json_limit.status(), StatusCode::BAD_REQUEST);

    let oversized_upload = binary_request(
        &router,
        Method::POST,
        &catalog_uri,
        "image/png",
        vec![0; 512 * 1024 + 1],
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(oversized_upload.status(), StatusCode::PAYLOAD_TOO_LARGE);

    let oversized_dimensions = binary_request(
        &router,
        Method::POST,
        &catalog_uri,
        "image/png",
        rgba_png(1_025, 1),
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(oversized_dimensions.status(), StatusCode::BAD_REQUEST);

    let created = binary_request(
        &router,
        Method::POST,
        &format!("{catalog_uri}?name=Portal%20texture"),
        "image/png",
        ONE_PIXEL_PNG.to_vec(),
        Some(&owner_cookie),
    )
    .await;
    let created_status = created.status();
    let created_body = created.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        created_status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&created_body)
    );
    let created: serde_json::Value = serde_json::from_slice(&created_body).unwrap();
    let image_id = created["id"].as_str().unwrap();
    assert_eq!(created["name"], "Portal texture");
    assert_eq!(created["media_type"], "image/png");
    assert_eq!(created["width"], 1);
    assert_eq!(created["height"], 1);
    assert!(
        created["content_hash"]
            .as_str()
            .unwrap()
            .starts_with("blake3:")
    );

    let untrusted_content_type = binary_request(
        &router,
        Method::POST,
        &catalog_uri,
        "application/octet-stream",
        ONE_PIXEL_PNG.to_vec(),
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(untrusted_content_type.status(), StatusCode::CREATED);

    let denied_upload = binary_request(
        &router,
        Method::POST,
        &catalog_uri,
        "image/png",
        ONE_PIXEL_PNG.to_vec(),
        Some(&outsider_cookie),
    )
    .await;
    assert_eq!(denied_upload.status(), StatusCode::FORBIDDEN);

    let listed = api_request(
        &router,
        Method::GET,
        &catalog_uri,
        None,
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(listed.status(), StatusCode::OK);
    assert_eq!(json_body(listed).await["images"][0]["id"], image_id);

    let content_uri = format!("{catalog_uri}/{image_id}/content");
    let downloaded = api_request(
        &router,
        Method::GET,
        &content_uri,
        None,
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(downloaded.status(), StatusCode::OK);
    assert_eq!(downloaded.headers()[CONTENT_TYPE], "image/png");
    assert_eq!(downloaded.headers()["cache-control"], "private, no-store");
    assert_eq!(downloaded.headers()["x-content-type-options"], "nosniff");
    assert!(downloaded.headers().contains_key("etag"));
    let downloaded_bytes = downloaded.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(downloaded_bytes.as_ref(), ONE_PIXEL_PNG);

    let denied_list = api_request(
        &router,
        Method::GET,
        &catalog_uri,
        None,
        Some(&outsider_cookie),
    )
    .await;
    assert_eq!(denied_list.status(), StatusCode::FORBIDDEN);
    let denied_content = api_request(
        &router,
        Method::GET,
        &content_uri,
        None,
        Some(&outsider_cookie),
    )
    .await;
    assert_eq!(denied_content.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn project_image_catalog_rejects_animated_png_and_webp() {
    let router = build_application().into_router();
    let (owner_cookie, target, _) =
        create_project_level(&router, "image-animation@example.com").await;
    let catalog_uri = format!("/api/projects/{}/images", target.project_id);

    for (media_type, image) in [
        ("image/png", animated_png()),
        ("image/webp", animated_webp()),
    ] {
        let response = binary_request(
            &router,
            Method::POST,
            &catalog_uri,
            media_type,
            image,
            Some(&owner_cookie),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn project_image_catalog_detects_jpeg_and_webp_content() {
    let router = build_application().into_router();
    let (owner_cookie, target, _) =
        create_project_level(&router, "image-formats@example.com").await;
    let catalog_uri = format!("/api/projects/{}/images", target.project_id);

    for (format, media_type) in [
        (ImageFormat::Jpeg, "image/jpeg"),
        (ImageFormat::WebP, "image/webp"),
    ] {
        let original = encoded_rgba_image(format);
        let created = binary_request(
            &router,
            Method::POST,
            &catalog_uri,
            "application/octet-stream",
            original.clone(),
            Some(&owner_cookie),
        )
        .await;
        assert_eq!(created.status(), StatusCode::CREATED);
        let created = json_body(created).await;
        assert_eq!(created["media_type"], media_type);
        assert_eq!(created["width"], 2);
        assert_eq!(created["height"], 1);
        let image_id = created["id"].as_str().unwrap();
        let content_uri = format!("{catalog_uri}/{image_id}/content");

        let downloaded = api_request(
            &router,
            Method::GET,
            &content_uri,
            None,
            Some(&owner_cookie),
        )
        .await;
        assert_eq!(downloaded.status(), StatusCode::OK);
        assert_eq!(downloaded.headers()[CONTENT_TYPE], media_type);
        assert_eq!(downloaded.headers()["cache-control"], "private, no-store");
        assert_eq!(downloaded.headers()["x-content-type-options"], "nosniff");
        let downloaded = downloaded.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(downloaded.as_ref(), original.as_slice());
    }

    for format in [ImageFormat::Jpeg, ImageFormat::WebP] {
        let oversized = binary_request(
            &router,
            Method::POST,
            &catalog_uri,
            "application/octet-stream",
            encoded_rgba_image_at_size(format, 1_025, 1),
            Some(&owner_cookie),
        )
        .await;
        assert_eq!(oversized.status(), StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn project_image_catalog_enforces_template_quota() {
    let router = build_application().into_router();
    let (owner_cookie, target, _) = create_project_level(&router, "image-quota@example.com").await;
    let catalog_uri = format!("/api/projects/{}/images", target.project_id);
    let image = rgba_png(1, 1);

    for _ in 0..64 {
        let response = binary_request(
            &router,
            Method::POST,
            &catalog_uri,
            "image/png",
            image.clone(),
            Some(&owner_cookie),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED);
    }

    let exceeded = binary_request(
        &router,
        Method::POST,
        &catalog_uri,
        "image/png",
        image,
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(exceeded.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn prepared_image_applies_atomically_through_authorized_rpc() {
    use oreak_core::{
        DitherMode, ImagePlacement, ImageSampling, ImageTransparency, IndexedImage, PaletteSettings,
    };
    use oreak_protocol::ApplyImageRequest;

    let server = TestServer::start().await;
    let (cookie, target, user) =
        create_project_level(&server.router, "image-apply@example.com").await;
    let client = server.client(Some(&cookie)).await;
    let settings = PaletteSettings {
        enabled_colors: (1..=10).collect(),
        dithering: DitherMode::None,
        alpha_threshold: 1,
        background: None,
        mappings: vec![],
    };
    // One continuous two-pixel source: left red, right yellow.
    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(
            &[255, 139, 104, 255, 255, 209, 90, 255],
            2,
            1,
            ExtendedColorType::Rgba8,
        )
        .unwrap();
    let uploaded = binary_request(
        &server.router,
        Method::POST,
        &format!("/api/projects/{}/images?name=two-colors", target.project_id),
        "image/png",
        png,
        Some(&cookie),
    )
    .await;
    assert_eq!(uploaded.status(), StatusCode::CREATED);
    let source = json_body(uploaded).await;
    let prepared_response = api_request(
        &server.router,
        Method::POST,
        &format!(
            "/api/projects/{}/images/{}/prepare",
            target.project_id,
            source["id"].as_str().unwrap()
        ),
        Some(serde_json::json!({"name":"two-colors prepared", "settings":settings})),
        Some(&cookie),
    )
    .await;
    assert_eq!(prepared_response.status(), StatusCode::CREATED);
    let prepared = json_body(prepared_response).await;
    let prepared_id = prepared["entry"]["id"].as_str().unwrap().to_owned();
    let indexed: IndexedImage = serde_json::from_value(prepared["image"].clone()).unwrap();
    assert_eq!(indexed.pixels, vec![1, 2]);
    let shape = Shape::new(1, 1, 1).unwrap();
    let pools: Vec<_> = (0..2)
        .map(|x| {
            PlaceableEntity::blind(
                format!("image-pool-{x}"),
                GridPoint::new(x, 0),
                shape,
                Blind::new(2, vec![BlindTile::empty(2).unwrap()]).unwrap(),
            )
            .unwrap()
        })
        .collect();
    client
        .apply_command(ApplyCommandRequest {
            target: target.clone(),
            command: CommandEnvelope::new(
                CommandMetadata::new("image-place-pools", "spoof", 1),
                LevelCommand::PlaceEntities {
                    entities: pools.clone(),
                },
            ),
        })
        .await
        .unwrap();
    let placement = ImagePlacement {
        x: 0.0,
        y: 0.0,
        width: 2.0,
        height: 1.0,
        sampling: ImageSampling::Nearest,
        pixelation: 1,
        resolution: None,
        transparency: ImageTransparency::Preserve,
    };
    let request = ApplyImageRequest {
        target: target.clone(),
        metadata: CommandMetadata::new("image-apply", "spoof", 1),
        prepared_image_id: prepared_id.clone(),
        placement: placement.clone(),
        targets: pools.clone(),
    };
    let anonymous = server.client(None).await;
    assert!(anonymous.apply_image(request.clone()).await.is_err());
    // No bypass through the ordinary command API, even with a valid pixel buffer.
    assert!(
        client
            .apply_command(ApplyCommandRequest {
                target: target.clone(),
                command: CommandEnvelope::new(
                    CommandMetadata::new("image-direct", "spoof", 1),
                    LevelCommand::ApplyImageToPools {
                        image: indexed,
                        settings: settings.clone(),
                        placement: placement.clone(),
                        targets: pools.clone()
                    },
                ),
            })
            .await
            .is_err()
    );
    let mut missing = request.clone();
    missing.prepared_image_id = "missing-prepared-image".into();
    assert!(client.apply_image(missing).await.is_err());
    let applied = client.apply_image(request.clone()).await.unwrap();
    assert_eq!(applied.server_sequence, 2);
    let ApplyCommandResult::Applied { event } = applied.result else {
        panic!("expected image event")
    };
    assert_eq!(event.metadata.actor.as_str(), user["id"].as_str().unwrap());
    assert_eq!(event.changes.len(), 2);
    let snapshot = client.level_snapshot(target.clone()).await.unwrap();
    for (index, pool) in pools.iter().enumerate() {
        let entity = snapshot.snapshot.entity(pool.id()).unwrap();
        let blind = entity.as_blind().unwrap();
        for y in 0..2 {
            for x in 0..2 {
                assert_eq!(
                    blind.color_at(shape, BlindPixel::new(x, y)),
                    Some(index as u8 + 1)
                );
            }
        }
    }
    // A stale selection snapshot must reject the entire command, not partially overwrite.
    let mut stale = request;
    stale.metadata = CommandMetadata::new("image-stale", "spoof", 1);
    assert!(client.apply_image(stale).await.is_err());
    assert_eq!(
        client
            .level_snapshot(target.clone())
            .await
            .unwrap()
            .server_sequence,
        2
    );
    client
        .undo_latest(UndoLatestRequest {
            target: target.clone(),
            metadata: CommandMetadata::new("image-undo", "spoof", 1),
        })
        .await
        .unwrap();
    let restored = client.level_snapshot(target).await.unwrap();
    assert_eq!(restored.server_sequence, 3);
    for pool in pools {
        assert_eq!(restored.snapshot.entity(pool.id()), Some(&pool));
    }
}

fn distribution_entities(boundary: PoolBoundary) -> Vec<PlaceableEntity> {
    let shape = Shape::new(1, 1, 1).unwrap();
    vec![
        PlaceableEntity::blind(
            "distribution-pool",
            GridPoint::new(0, 0),
            shape,
            Blind::new(8, vec![BlindTile::from_colors(8, vec![1; 64]).unwrap()])
                .unwrap()
                .with_boundary(boundary)
                .unwrap(),
        )
        .unwrap(),
        PlaceableEntity::block(
            "distribution-a",
            GridPoint::new(1, 0),
            shape,
            Block::new(vec![
                CollectLayer::new(1, Some(3), CollectCapacity::Finite(4), true),
                CollectLayer::new(1, Some(2), CollectCapacity::Finite(91), false),
            ]),
        )
        .unwrap(),
        PlaceableEntity::block(
            "distribution-b",
            GridPoint::new(2, 0),
            shape,
            Block::new(vec![CollectLayer::new(
                1,
                None,
                CollectCapacity::Unlimited,
                false,
            )]),
        )
        .unwrap(),
    ]
}

fn distribution_layers(entity: &PlaceableEntity) -> &[CollectLayer] {
    let PlaceableEntityKind::Block(block) = entity.kind() else {
        panic!("expected a Block");
    };
    block.collect_layers()
}

fn distribution_command(expected_entities: Vec<PlaceableEntity>) -> LevelCommand {
    LevelCommand::ApplyDistribution {
        request: DistributionRequest {
            pools: vec![DistributionPoolSelection {
                entity_id: "distribution-pool".into(),
                group_ids: vec![0],
            }],
            // Deliberately not entity/layer order; locked weight must be ignored.
            layers: vec![
                DistributionLayerSelection {
                    entity_id: "distribution-b".into(),
                    layer_index: 0,
                    weight: 3,
                },
                DistributionLayerSelection {
                    entity_id: "distribution-a".into(),
                    layer_index: 1,
                    weight: 1,
                },
                DistributionLayerSelection {
                    entity_id: "distribution-a".into(),
                    layer_index: 0,
                    weight: 999,
                },
            ],
        },
        expected_entities,
    }
}

fn distribution_rpc_request(
    target: &ProjectLevelTarget,
    id: &str,
    command: LevelCommand,
) -> ApplyCommandRequest {
    ApplyCommandRequest {
        target: target.clone(),
        command: CommandEnvelope::new(CommandMetadata::new(id, "spoofed", 1), command),
    }
}

fn assert_rpc_error_code(error: ClientError, code: RpcErrorCode) -> RpcErrorData {
    let ClientError::Call(error) = error else {
        panic!("expected a JSON-RPC call error");
    };
    assert_eq!(error.code(), code.json_rpc_code());
    let data: RpcErrorData = serde_json::from_str(error.data().unwrap().get()).unwrap();
    assert_eq!(data.code, code);
    data
}

#[tokio::test]
async fn distribution_rpc_counts_weights_atomically_and_undo_restores_capacities() {
    for (boundary, expected_capacities) in [
        // At ppc=8 the shared default padding/radius is 1: 6*6 playable pixels.
        (PoolBoundary::default(), [4, 8, 24]),
        (
            PoolBoundary {
                padding_pixels: Some(0),
                corner_radius_pixels: Some(0),
            },
            [4, 15, 45],
        ),
    ] {
        let server = TestServer::start().await;
        let (cookie, target, user) =
            create_project_level(&server.router, "distribution-apply@example.com").await;
        let client = server.client(Some(&cookie)).await;
        let entities = distribution_entities(boundary);
        client
            .apply_command(distribution_rpc_request(
                &target,
                "distribution-place",
                LevelCommand::PlaceEntities {
                    entities: entities.clone(),
                },
            ))
            .await
            .unwrap();
        let before = client.level_snapshot(target.clone()).await.unwrap();
        let mut subscription = client.subscribe_level(target.clone()).await.unwrap();
        assert!(matches!(
            subscription.next().await.unwrap().unwrap(),
            LevelSubscriptionItem::Snapshot { .. }
        ));
        let request = distribution_rpc_request(
            &target,
            "distribution-apply",
            distribution_command(entities.clone()),
        );
        let applied = client.apply_command(request.clone()).await.unwrap();
        assert_eq!(applied.server_sequence, 2);
        let ApplyCommandResult::Applied { event } = applied.result else {
            panic!("expected one distribution event");
        };
        assert_eq!(event.changes.len(), 2);
        assert_eq!(event.metadata.actor.as_str(), user["id"].as_str().unwrap());
        assert_ne!(event.metadata.occurred_at_ms, 1);
        assert!(matches!(
            event.inverse,
            LevelCommand::RestoreEntities { .. }
        ));
        assert!(matches!(
            subscription.next().await.unwrap().unwrap(),
            LevelSubscriptionItem::Event { event } if event.server_sequence == 2
        ));
        let after = client.level_snapshot(target.clone()).await.unwrap();
        assert_eq!(after.snapshot.entity(entities[0].id()), Some(&entities[0]));
        let capacities: Vec<_> = entities[1..]
            .iter()
            .flat_map(|original| distribution_layers(after.snapshot.entity(original.id()).unwrap()))
            .map(|layer| layer.capacity())
            .collect();
        assert_eq!(
            capacities,
            expected_capacities.map(CollectCapacity::Finite).to_vec()
        );
        // Radius, color and lock state are retained; only unlocked capacities change.
        for original in &entities[1..] {
            let actual = after.snapshot.entity(original.id()).unwrap();
            for (before, after) in distribution_layers(original)
                .iter()
                .zip(distribution_layers(actual))
            {
                assert_eq!(before.color_index(), after.color_index());
                assert_eq!(before.radius(), after.radius());
                assert_eq!(before.is_locked(), after.is_locked());
            }
        }
        let error = client.apply_command(request).await.unwrap_err();
        let data = assert_rpc_error_code(error, RpcErrorCode::DuplicateCommand);
        assert_eq!(data.message, RpcErrorCode::DuplicateCommand.message());
        assert_eq!(client.level_snapshot(target.clone()).await.unwrap(), after);
        let undo = client
            .undo_latest(UndoLatestRequest {
                target: target.clone(),
                metadata: CommandMetadata::new("distribution-undo", "spoofed", 1),
            })
            .await
            .unwrap();
        assert_eq!(undo.server_sequence, 3);
        assert_eq!(undo.event.reverts_sequence, Some(2));
        assert!(matches!(
            undo.event.command,
            LevelCommand::RestoreEntities { .. }
        ));
        let restored = client.level_snapshot(target.clone()).await.unwrap();
        assert_eq!(restored.snapshot, before.snapshot);
        assert_eq!(restored.level_hash, before.level_hash);
        assert!(matches!(
            subscription.next().await.unwrap().unwrap(),
            LevelSubscriptionItem::Event { event } if event.server_sequence == 3
        ));
        assert!(
            timeout(Duration::from_millis(100), subscription.next())
                .await
                .is_err()
        );
        let history = client
            .level_history(LevelHistoryRequest {
                target,
                before_sequence: None,
                limit: 10,
            })
            .await
            .unwrap();
        assert_eq!(
            history
                .events
                .iter()
                .map(|event| event.sequence)
                .collect::<Vec<_>>(),
            vec![3, 2, 1]
        );
    }
}

#[tokio::test]
async fn distribution_group_save_stale_guards_and_undo_preserve_exact_entities() {
    let server = TestServer::start().await;
    let (cookie, target, _) =
        create_project_level(&server.router, "distribution-groups@example.com").await;
    let client = server.client(Some(&cookie)).await;
    let entities = distribution_entities(PoolBoundary::default());
    client
        .apply_command(distribution_rpc_request(
            &target,
            "distribution-place",
            LevelCommand::PlaceEntities {
                entities: entities.clone(),
            },
        ))
        .await
        .unwrap();
    let before = client.level_snapshot(target.clone()).await.unwrap();
    let groups = vec![PoolDistributionGroup {
        id: 7,
        name: "Border selection".into(),
        pixels: vec![BlindPixel::new(0, 0), BlindPixel::new(7, 7)],
    }];
    let group_command = LevelCommand::SetPoolDistributionGroups {
        entity_id: entities[0].id().clone(),
        expected: entities[0].clone(),
        groups: groups.clone(),
    };
    let saved = client
        .apply_command(distribution_rpc_request(
            &target,
            "distribution-save",
            group_command.clone(),
        ))
        .await
        .unwrap();
    assert_eq!(saved.server_sequence, 2);
    let current = client.level_snapshot(target.clone()).await.unwrap();
    assert_eq!(
        current.snapshot.entity(entities[0].id()),
        Some(&entities[0].with_pool_distribution_groups(groups).unwrap())
    );
    let mut subscription = client.subscribe_level(target.clone()).await.unwrap();
    subscription.next().await.unwrap().unwrap();
    // Exact source metadata is stale even though only excluded border pixels changed.
    let stale_source = distribution_command(entities.clone());
    let mut stale_targets = current.snapshot.entities().to_vec();
    let index = stale_targets
        .iter()
        .position(|entity| entity.id() == entities[1].id())
        .unwrap();
    stale_targets[index] = stale_targets[index]
        .with_block_collect_layers(vec![
            CollectLayer::new(1, Some(9), CollectCapacity::Finite(4), true),
            CollectLayer::new(1, Some(2), CollectCapacity::Finite(91), false),
        ])
        .unwrap();
    // Exact target radius differs, while color/capacity and allocation inputs agree.
    for (id, command, entity_id) in [
        ("distribution-stale-source", stale_source, entities[0].id()),
        (
            "distribution-stale-target",
            distribution_command(stale_targets),
            entities[1].id(),
        ),
        ("distribution-stale-groups", group_command, entities[0].id()),
    ] {
        let error = client
            .apply_command(distribution_rpc_request(&target, id, command))
            .await
            .unwrap_err();
        let data = assert_rpc_error_code(error, RpcErrorCode::InvalidCommand);
        let details = serde_json::to_string(&data).unwrap();
        assert!(details.contains(entity_id.as_str()));
        assert!(details.contains("changed after the preview"));
        assert_eq!(
            client.level_snapshot(target.clone()).await.unwrap(),
            current
        );
    }
    let history = client
        .level_history(LevelHistoryRequest {
            target: target.clone(),
            before_sequence: None,
            limit: 10,
        })
        .await
        .unwrap();
    assert_eq!(history.events.len(), 2);
    assert!(
        timeout(Duration::from_millis(100), subscription.next())
            .await
            .is_err()
    );
    let undo = client
        .undo_latest(UndoLatestRequest {
            target: target.clone(),
            metadata: CommandMetadata::new("distribution-group-undo", "spoofed", 1),
        })
        .await
        .unwrap();
    assert_eq!(undo.server_sequence, 3);
    assert_eq!(undo.event.reverts_sequence, Some(2));
    assert!(matches!(
        undo.event.command,
        LevelCommand::RestoreEntities { .. }
    ));
    let restored = client.level_snapshot(target).await.unwrap();
    assert_eq!(restored.snapshot, before.snapshot);
    assert_eq!(restored.level_hash, before.level_hash);
}

#[tokio::test]
async fn distribution_payload_budget_rejects_before_commit_or_broadcast() {
    let server = TestServer::start().await;
    let (cookie, target, _) =
        create_project_level(&server.router, "distribution-budget@example.com").await;
    let client = server.client(Some(&cookie)).await;
    let mut entities = distribution_entities(PoolBoundary::default());
    // Ordinary commands can leave a large snapshot. Each placement response fits
    // the transport limit, but distribution/group edits must check the full snapshot.
    for entity in &mut entities[1..] {
        let mut layers = distribution_layers(entity).to_vec();
        layers.resize(
            40_000,
            CollectLayer::new(1, None, CollectCapacity::Finite(91), false),
        );
        *entity = entity.with_block_collect_layers(layers).unwrap();
    }
    let extra = PlaceableEntity::block(
        "distribution-unselected",
        GridPoint::new(3, 0),
        Shape::new(1, 1, 1).unwrap(),
        Block::new(vec![
            CollectLayer::new(
                1,
                None,
                CollectCapacity::Finite(91),
                false
            );
            40_000
        ]),
    )
    .unwrap();
    let saved_groups = vec![PoolDistributionGroup {
        id: 2,
        name: "Existing border mask".into(),
        pixels: vec![BlindPixel::new(0, 0)],
    }];
    for (index, entity) in entities.iter().chain(std::iter::once(&extra)).enumerate() {
        client
            .apply_command(distribution_rpc_request(
                &target,
                &format!("distribution-large-place-{index}"),
                LevelCommand::PlaceEntity {
                    entity: entity.clone(),
                },
            ))
            .await
            .unwrap();
        if index == 0 {
            client
                .apply_command(distribution_rpc_request(
                    &target,
                    "distribution-early-group-save",
                    LevelCommand::SetPoolDistributionGroups {
                        entity_id: entity.id().clone(),
                        expected: entity.clone(),
                        groups: saved_groups.clone(),
                    },
                ))
                .await
                .unwrap();
        }
    }
    entities[0] = entities[0]
        .with_pool_distribution_groups(saved_groups)
        .unwrap();
    client
        .apply_command(distribution_rpc_request(
            &target,
            "distribution-later-small-command",
            LevelCommand::SetCell {
                point: GridPoint::new(7, 7),
                kind: CellKind::Wall,
            },
        ))
        .await
        .unwrap();
    let before = client.level_snapshot(target.clone()).await.unwrap();
    assert!(serde_json::to_vec(&before).unwrap().len() > 8 * 1024 * 1024);
    let mut subscription = client.subscribe_level(target.clone()).await.unwrap();
    subscription.next().await.unwrap().unwrap();
    for (id, command) in [
        (
            "distribution-large-apply",
            distribution_command(entities.clone()),
        ),
        (
            "distribution-large-groups",
            LevelCommand::SetPoolDistributionGroups {
                entity_id: entities[0].id().clone(),
                expected: entities[0].clone(),
                groups: vec![PoolDistributionGroup {
                    id: 1,
                    name: "Mask".into(),
                    pixels: vec![BlindPixel::new(1, 1)],
                }],
            },
        ),
    ] {
        let error = client
            .apply_command(distribution_rpc_request(&target, id, command))
            .await
            .unwrap_err();
        let data = assert_rpc_error_code(error, RpcErrorCode::InvalidCommand);
        let details = serde_json::to_string(&data).unwrap();
        assert!(details.contains("invalid distribution"));
        assert!(details.contains("8 MiB"));
        assert_eq!(client.level_snapshot(target.clone()).await.unwrap(), before);
    }
    // An active group edit also enables the conservative bounded-history undo
    // guard. Even undoing the later tiny cell edit cannot publish a huge snapshot.
    let error = client
        .undo_latest(UndoLatestRequest {
            target: target.clone(),
            metadata: CommandMetadata::new("distribution-large-undo", "spoofed", 1),
        })
        .await
        .unwrap_err();
    let data = assert_rpc_error_code(error, RpcErrorCode::InvalidCommand);
    assert!(serde_json::to_string(&data).unwrap().contains("8 MiB"));
    assert_eq!(client.level_snapshot(target.clone()).await.unwrap(), before);
    let history = client
        .level_history(LevelHistoryRequest {
            target,
            before_sequence: None,
            limit: 10,
        })
        .await
        .unwrap();
    assert_eq!(history.events[0].sequence, 6);
    assert!(
        timeout(Duration::from_millis(100), subscription.next())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn distribution_commands_require_existing_edit_permission() {
    let server = TestServer::start().await;
    let (cookie, target, _) =
        create_project_level(&server.router, "distribution-owner@example.com").await;
    let (viewer_cookie, _) = register(&server.router, "distribution-viewer@example.com").await;
    let invitation = api_request(
        &server.router,
        Method::POST,
        &format!("/api/projects/{}/invitations", target.project_id),
        Some(serde_json::json!({"email": "distribution-viewer@example.com", "roles": ["viewer"]})),
        Some(&cookie),
    )
    .await;
    assert_eq!(invitation.status(), StatusCode::CREATED);
    let invitation = json_body(invitation).await;
    let accepted = api_request(
        &server.router,
        Method::POST,
        &format!(
            "/api/projects/{}/invitations/{}/accept",
            target.project_id,
            invitation["id"].as_str().unwrap()
        ),
        None,
        Some(&viewer_cookie),
    )
    .await;
    assert_eq!(accepted.status(), StatusCode::OK);
    let owner = server.client(Some(&cookie)).await;
    let entities = distribution_entities(PoolBoundary::default());
    owner
        .apply_command(distribution_rpc_request(
            &target,
            "distribution-place",
            LevelCommand::PlaceEntities {
                entities: entities.clone(),
            },
        ))
        .await
        .unwrap();
    let before = owner.level_snapshot(target.clone()).await.unwrap();
    for (cookie, error_code) in [
        (Some(viewer_cookie.as_str()), RpcErrorCode::PermissionDenied),
        (None, RpcErrorCode::AuthenticationRequired),
    ] {
        let client = server.client(cookie).await;
        if cookie.is_some() {
            assert_eq!(client.level_snapshot(target.clone()).await.unwrap(), before);
        }
        for command in [
            distribution_command(entities.clone()),
            LevelCommand::SetPoolDistributionGroups {
                entity_id: entities[0].id().clone(),
                expected: entities[0].clone(),
                groups: vec![PoolDistributionGroup {
                    id: 1,
                    name: "Selected".into(),
                    pixels: vec![BlindPixel::new(1, 1)],
                }],
            },
        ] {
            let error = client
                .apply_command(distribution_rpc_request(
                    &target,
                    "distribution-denied",
                    command,
                ))
                .await
                .unwrap_err();
            assert_rpc_error_code(error, error_code);
        }
    }
    assert_eq!(owner.level_snapshot(target).await.unwrap(), before);
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
async fn grouped_move_and_resize_share_the_authoritative_timeline() {
    let server = TestServer::start().await;
    let (cookie, target, _) = create_project_level(&server.router, "layout-rpc@example.com").await;
    let client = server.client(Some(&cookie)).await;
    for (command_id, entity_id, point) in [
        ("place-a", "a", GridPoint::new(1, 1)),
        ("place-b", "b", GridPoint::new(2, 1)),
    ] {
        let entity = PlaceableEntity::block(
            entity_id,
            point,
            Shape::new(1, 1, 1).unwrap(),
            Block::default(),
        )
        .unwrap();
        client
            .apply_command(ApplyCommandRequest {
                target: target.clone(),
                command: CommandEnvelope::new(
                    CommandMetadata::new(command_id, "spoofed", 1),
                    LevelCommand::PlaceEntity { entity },
                ),
            })
            .await
            .unwrap();
    }

    let moved = client
        .apply_command(ApplyCommandRequest {
            target: target.clone(),
            command: CommandEnvelope::new(
                CommandMetadata::new("move-group", "spoofed", 1),
                LevelCommand::MoveEntities {
                    moves: vec![
                        EntityMove::new("a", GridPoint::new(2, 1)),
                        EntityMove::new("b", GridPoint::new(3, 1)),
                    ],
                },
            ),
        })
        .await
        .unwrap();
    assert_eq!(moved.server_sequence, 3);
    assert!(matches!(
        moved.result,
        ApplyCommandResult::Applied { ref event } if event.changes.len() == 2
    ));

    let resized = client
        .apply_command(ApplyCommandRequest {
            target: target.clone(),
            command: CommandEnvelope::new(
                CommandMetadata::new("resize", "spoofed", 1),
                LevelCommand::ResizeGrid {
                    size: GridSize::new(10, 6).unwrap(),
                    anchor: GridAnchor::BottomLeft,
                },
            ),
        })
        .await
        .unwrap();
    assert_eq!(resized.server_sequence, 4);

    let snapshot = client.level_snapshot(target).await.unwrap().snapshot;
    assert_eq!(snapshot.size(), GridSize::new(10, 6).unwrap());
    assert_eq!(
        snapshot.entity(&EntityId::from("a")).unwrap().origin(),
        GridPoint::new(2, 1)
    );
    assert_eq!(
        snapshot.entity(&EntityId::from("b")).unwrap().origin(),
        GridPoint::new(3, 1)
    );
}

#[tokio::test]
async fn blind_brush_commands_use_the_authoritative_core_path() {
    let server = TestServer::start().await;
    let (cookie, target, user) =
        create_project_level(&server.router, "brush-rpc@example.com").await;
    let client = server.client(Some(&cookie)).await;
    let entity_id = EntityId::from("blind-rpc");
    let entity = PlaceableEntity::blind(
        entity_id.clone(),
        GridPoint::new(2, 2),
        Shape::new(1, 1, 1).unwrap(),
        Blind::new(2, vec![BlindTile::empty(2).unwrap()]).unwrap(),
    )
    .unwrap();
    client
        .apply_command(ApplyCommandRequest {
            target: target.clone(),
            command: CommandEnvelope::new(
                CommandMetadata::new("place-blind", "spoofed", 1),
                LevelCommand::PlaceEntity { entity },
            ),
        })
        .await
        .unwrap();

    let painted = client
        .apply_command(ApplyCommandRequest {
            target: target.clone(),
            command: CommandEnvelope::new(
                CommandMetadata::new("paint-blind", "spoofed", 1),
                LevelCommand::PaintBlindStroke {
                    entity_id: entity_id.clone(),
                    color_index: 4,
                    stroke: BlindStroke::new(vec![BlindPixel::new(0, 0), BlindPixel::new(1, 0)])
                        .unwrap(),
                },
            ),
        })
        .await
        .unwrap();
    assert!(matches!(
        painted.result,
        ApplyCommandResult::Applied { ref event }
            if event.metadata.actor.as_str() == user["id"].as_str().unwrap()
    ));

    client
        .apply_command(ApplyCommandRequest {
            target: target.clone(),
            command: CommandEnvelope::new(
                CommandMetadata::new("erase-blind", "spoofed", 1),
                LevelCommand::EraseBlindStroke {
                    entity_id: entity_id.clone(),
                    stroke: BlindStroke::new(vec![BlindPixel::new(1, 0)]).unwrap(),
                },
            ),
        })
        .await
        .unwrap();
    client
        .apply_command(ApplyCommandRequest {
            target: target.clone(),
            command: CommandEnvelope::new(
                CommandMetadata::new("fill-blind", "spoofed", 1),
                LevelCommand::FloodFillBlind {
                    entity_id: entity_id.clone(),
                    start: BlindPixel::new(1, 0),
                    color_index: 7,
                },
            ),
        })
        .await
        .unwrap();
    let recolored = client
        .apply_command(ApplyCommandRequest {
            target: target.clone(),
            command: CommandEnvelope::new(
                CommandMetadata::new("fill-painted", "spoofed", 1),
                LevelCommand::FloodFillBlind {
                    entity_id: entity_id.clone(),
                    start: BlindPixel::new(0, 0),
                    color_index: 9,
                },
            ),
        })
        .await
        .unwrap();
    assert!(matches!(
        recolored.result,
        ApplyCommandResult::Applied { .. }
    ));
    assert_eq!(recolored.server_sequence, 5);

    let snapshot = client.level_snapshot(target).await.unwrap();
    let entity = snapshot.snapshot.entity(&entity_id).unwrap();
    let blind = entity.as_blind().unwrap();
    assert_eq!(
        blind.color_at(entity.shape(), BlindPixel::new(0, 0)),
        Some(9)
    );
    assert_eq!(
        blind.color_at(entity.shape(), BlindPixel::new(1, 0)),
        Some(7)
    );
    assert_eq!(
        blind.color_at(entity.shape(), BlindPixel::new(0, 1)),
        Some(7)
    );
    assert_eq!(
        blind.color_at(entity.shape(), BlindPixel::new(1, 1)),
        Some(7)
    );
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
async fn glass_commands_apply_and_broadcast_through_the_authoritative_path() {
    let server = TestServer::start().await;
    let (cookie, target, user) =
        create_project_level(&server.router, "glass-rpc@example.com").await;
    let client = server.client(Some(&cookie)).await;
    let entity_id = EntityId::from("glass-pool");
    let entity = PlaceableEntity::blind(
        entity_id.clone(),
        GridPoint::new(2, 2),
        Shape::new(1, 1, 1).unwrap(),
        Blind::new(2, vec![BlindTile::empty(2).unwrap()]).unwrap(),
    )
    .unwrap();
    client
        .apply_command(ApplyCommandRequest {
            target: target.clone(),
            command: CommandEnvelope::new(
                CommandMetadata::new("place-glass-pool", "spoofed", 1),
                LevelCommand::PlaceEntity { entity },
            ),
        })
        .await
        .unwrap();

    let mut subscription = client.subscribe_level(target.clone()).await.unwrap();
    let initial = timeout(Duration::from_secs(1), subscription.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(
        initial,
        LevelSubscriptionItem::Snapshot { snapshot } if snapshot.server_sequence == 1
    ));

    let response = client
        .apply_command(ApplyCommandRequest {
            target: target.clone(),
            command: CommandEnvelope::new(
                CommandMetadata::new("set-glass", "spoofed", 1),
                LevelCommand::SetGlass {
                    decorator_id: DecoratorId::from("glass-pool-decorator"),
                    entity_id: entity_id.clone(),
                    blocking_count: 3,
                },
            ),
        })
        .await
        .unwrap();
    assert!(matches!(
        response.result,
        ApplyCommandResult::Applied { ref event }
            if event.metadata.actor.as_str() == user["id"].as_str().unwrap()
                && matches!(&event.command, LevelCommand::SetGlass { blocking_count: 3, .. })
    ));

    let update = timeout(Duration::from_secs(1), subscription.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(
        update,
        LevelSubscriptionItem::Event { event }
            if event.server_sequence == 2
                && matches!(&event.event.command, LevelCommand::SetGlass { blocking_count: 3, .. })
    ));
    let snapshot = client.level_snapshot(target).await.unwrap().snapshot;
    assert_eq!(snapshot.glass_blocking_count(&entity_id), 3);
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
async fn presence_reports_connection_join_cursor_and_leave() {
    let server = TestServer::start().await;
    let (cookie, target, _) = create_project_level(&server.router, "presence@example.com").await;
    let first_client = server.client(Some(&cookie)).await;
    let second_client = server.client(Some(&cookie)).await;
    let mut first = first_client
        .subscribe_level_presence(target.clone())
        .await
        .unwrap();
    let first_snapshot = timeout(Duration::from_secs(1), first.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let LevelPresenceItem::Snapshot {
        self_id: first_id,
        participants,
        ..
    } = first_snapshot
    else {
        panic!("presence subscription must start with a snapshot");
    };
    assert_eq!(participants.len(), 1);

    let mut second = second_client
        .subscribe_level_presence(target.clone())
        .await
        .unwrap();
    let second_snapshot = timeout(Duration::from_secs(1), second.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let LevelPresenceItem::Snapshot {
        self_id: second_id,
        participants,
        ..
    } = second_snapshot
    else {
        panic!("presence subscription must start with a snapshot");
    };
    assert_ne!(first_id, second_id);
    assert_eq!(participants.len(), 2);

    let joined = timeout(Duration::from_secs(1), first.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(
        joined,
        LevelPresenceItem::Joined { participant, .. } if participant.id == second_id
    ));

    second_client
        .update_level_cursor(UpdateLevelCursorRequest {
            target: target.clone(),
            cursor: Some(GridPoint::new(3, 4)),
        })
        .await
        .unwrap();
    let cursor = timeout(Duration::from_secs(1), first.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(
        cursor,
        LevelPresenceItem::Cursor {
            presence_id,
            cursor: Some(GridPoint { x: 3, y: 4 }),
            ..
        } if presence_id == second_id
    ));

    second.unsubscribe().await.unwrap();
    let left = timeout(Duration::from_secs(1), first.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(
        left,
        LevelPresenceItem::Left { presence_id, .. } if presence_id == second_id
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

use axum::{
    Router,
    body::Body,
    http::{
        Method, Request, Response, StatusCode,
        header::{CACHE_CONTROL, CONTENT_TYPE, COOKIE, ETAG, SET_COOKIE},
    },
};
use http_body_util::BodyExt;
use image::{ExtendedColorType, ImageEncoder, codecs::png::PngEncoder};
use oreak_core::{DitherMode, PALETTE_RGB, PaletteSettings, convert_rgba};
use oreak_server::build_application;
use serde_json::{Value, json};
use tower::ServiceExt;

async fn request(
    router: &Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
    cookie: Option<&str>,
) -> Response<Body> {
    let mut request = Request::builder().method(method).uri(uri);
    if let Some(cookie) = cookie {
        request = request.header(COOKIE, cookie);
    }
    if body.is_some() {
        request = request.header(CONTENT_TYPE, "application/json");
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

async fn json_body(response: Response<Body>) -> Value {
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

async fn register(router: &Router, email: &str) -> (String, Value) {
    let response = request(
        router,
        Method::POST,
        "/api/auth/register",
        Some(json!({"email":email,"password":"correct horse battery staple"})),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let cookie = response.headers()[SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    (cookie, json_body(response).await)
}

async fn project(router: &Router, cookie: &str, user: &Value) -> String {
    let response = request(
        router,
        Method::POST,
        "/api/projects",
        Some(json!({"workspace_id":user["personal_workspace_id"],"name":"Prepared images"})),
        Some(cookie),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    json_body(response).await["id"].as_str().unwrap().to_owned()
}

async fn upload(
    router: &Router,
    cookie: &str,
    project: &str,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> (String, Vec<u8>, Value) {
    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(rgba, width, height, ExtendedColorType::Rgba8)
        .unwrap();
    let response = router
        .clone()
        .oneshot(
            Request::post(format!("/api/projects/{project}/images?name=Original"))
                .header(COOKIE, cookie)
                .header(CONTENT_TYPE, "image/png")
                .body(Body::from(png.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let entry = json_body(response).await;
    (entry["id"].as_str().unwrap().to_owned(), png, entry)
}

fn settings() -> PaletteSettings {
    PaletteSettings {
        enabled_colors: vec![1, 4, 10],
        dithering: DitherMode::FloydSteinberg,
        alpha_threshold: 128,
        background: None,
        mappings: Vec::new(),
    }
}

#[tokio::test]
async fn prepared_images_are_scoped_immutable_deterministic_and_palette_only() {
    let router = build_application().into_router();
    let (owner, user) = register(&router, "prepared-owner@example.com").await;
    let project_id = project(&router, &owner, &user).await;
    let other_project = project(&router, &owner, &user).await;
    let (viewer, _) = register(&router, "prepared-viewer@example.com").await;
    let (outsider, _) = register(&router, "prepared-outsider@example.com").await;
    let invited = request(
        &router,
        Method::POST,
        &format!("/api/projects/{project_id}/invitations"),
        Some(json!({"email":"prepared-viewer@example.com","roles":["viewer"]})),
        Some(&owner),
    )
    .await;
    assert_eq!(invited.status(), StatusCode::CREATED);
    let invitation = json_body(invited).await;
    let accepted = request(
        &router,
        Method::POST,
        &format!(
            "/api/projects/{project_id}/invitations/{}/accept",
            invitation["id"].as_str().unwrap()
        ),
        None,
        Some(&viewer),
    )
    .await;
    assert_eq!(accepted.status(), StatusCode::OK);

    let rgba = vec![
        255, 139, 104, 255, 84, 202, 236, 255, 0, 0, 0, 0, 90, 80, 70, 120, 244, 247, 250, 255,
        180, 180, 180, 255,
    ];
    let (source, original, source_entry) = upload(&router, &owner, &project_id, 3, 2, &rgba).await;
    let source_uri = format!("/api/projects/{project_id}/images/{source}");
    let list_uri = format!("/api/projects/{project_id}/prepared-images");
    let prepare_uri = format!("{source_uri}/prepare");
    let body = json!({"name":"  Prepared  ","settings":settings()});

    for cookie in [None, Some(outsider.as_str()), Some(viewer.as_str())] {
        let response = request(
            &router,
            Method::POST,
            &prepare_uri,
            Some(body.clone()),
            cookie,
        )
        .await;
        assert_eq!(
            response.status(),
            if cookie.is_none() {
                StatusCode::UNAUTHORIZED
            } else {
                StatusCode::FORBIDDEN
            }
        );
    }
    let prepared = request(
        &router,
        Method::POST,
        &prepare_uri,
        Some(body.clone()),
        Some(&owner),
    )
    .await;
    assert_eq!(prepared.status(), StatusCode::CREATED);
    assert_eq!(prepared.headers()[CACHE_CONTROL], "private, no-store");
    let prepared = json_body(prepared).await;
    assert_eq!(prepared["entry"]["source_image_id"], source);
    assert_eq!(prepared["entry"]["created_by"], user["id"]);
    assert_eq!(prepared["entry"]["name"], "Prepared");
    assert_eq!(prepared["entry"]["width"], 3);
    assert_eq!(prepared["entry"]["height"], 2);
    assert_eq!(
        prepared["entry"]["palette_version"],
        prepared["image"]["palette_version"]
    );
    assert!(prepared["entry"]["created_at_ms"].is_i64());
    assert_eq!(
        prepared["entry"]["settings"],
        serde_json::to_value(settings()).unwrap()
    );
    assert_eq!(
        prepared["image"],
        serde_json::to_value(convert_rgba(3, 2, &rgba, &settings()).unwrap()).unwrap()
    );
    assert!(
        prepared["image"]["pixels"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| [0, 1, 4, 10].contains(&v.as_u64().unwrap()))
    );
    let repeated = request(
        &router,
        Method::POST,
        &prepare_uri,
        Some(body),
        Some(&owner),
    )
    .await;
    assert_eq!(repeated.status(), StatusCode::CREATED);
    let repeated = json_body(repeated).await;
    assert_ne!(repeated["entry"]["id"], prepared["entry"]["id"]);
    assert_eq!(repeated["image"], prepared["image"]);
    let prepared_id = prepared["entry"]["id"].as_str().unwrap();
    let prepared_uri = format!("{list_uri}/{prepared_id}");

    for uri in [&list_uri, &prepared_uri, &format!("{source_uri}/pixels")] {
        assert_eq!(
            request(&router, Method::GET, uri, None, None)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            request(&router, Method::GET, uri, None, Some(&outsider))
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
        let response = request(&router, Method::GET, uri, None, Some(&viewer)).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[CACHE_CONTROL], "private, no-store");
    }
    assert_eq!(
        json_body(request(&router, Method::GET, &prepared_uri, None, Some(&viewer)).await).await,
        prepared
    );
    let pixels = json_body(
        request(
            &router,
            Method::GET,
            &format!("{source_uri}/pixels"),
            None,
            Some(&viewer),
        )
        .await,
    )
    .await;
    assert_eq!(pixels, json!({"width":3,"height":2,"rgba":rgba}));
    let listed =
        json_body(request(&router, Method::GET, &list_uri, None, Some(&viewer)).await).await;
    assert_eq!(listed["images"].as_array().unwrap().len(), 2);
    assert_eq!(listed["images"][0], prepared["entry"]);
    assert!(listed["images"][0].get("pixels").is_none());

    for path in [
        format!("/api/projects/{other_project}/prepared-images/{prepared_id}"),
        format!("/api/projects/{other_project}/images/{source}/pixels"),
    ] {
        assert_eq!(
            request(&router, Method::GET, &path, None, Some(&owner))
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(
        request(
            &router,
            Method::POST,
            &format!("/api/projects/{other_project}/images/{source}/prepare"),
            Some(json!({"name":"Wrong project", "settings":settings()})),
            Some(&owner)
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    for method in [Method::PUT, Method::DELETE] {
        assert_eq!(
            request(&router, method, &prepared_uri, None, Some(&owner))
                .await
                .status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
    }
    let unchanged = json_body(
        request(
            &router,
            Method::GET,
            &format!("/api/projects/{project_id}/images"),
            None,
            Some(&owner),
        )
        .await,
    )
    .await;
    assert_eq!(unchanged["images"], json!([source_entry]));
    let bytes = request(
        &router,
        Method::GET,
        &format!("{source_uri}/content"),
        None,
        Some(&owner),
    )
    .await
    .into_body()
    .collect()
    .await
    .unwrap()
    .to_bytes();
    assert_eq!(bytes.as_ref(), original);
}

#[tokio::test]
async fn prepared_image_thumbnail_is_authenticated_scoped_bounded_and_palette_mapped() {
    let router = build_application().into_router();
    let (owner, user) = register(&router, "thumbnail-owner@example.com").await;
    let project_id = project(&router, &owner, &user).await;
    let other_project = project(&router, &owner, &user).await;
    let (viewer, _) = register(&router, "thumbnail-viewer@example.com").await;
    let (outsider, _) = register(&router, "thumbnail-outsider@example.com").await;
    let invited = request(
        &router,
        Method::POST,
        &format!("/api/projects/{project_id}/invitations"),
        Some(json!({"email":"thumbnail-viewer@example.com","roles":["viewer"]})),
        Some(&owner),
    )
    .await;
    assert_eq!(invited.status(), StatusCode::CREATED);
    let invitation = json_body(invited).await;
    assert_eq!(
        request(
            &router,
            Method::POST,
            &format!(
                "/api/projects/{project_id}/invitations/{}/accept",
                invitation["id"].as_str().unwrap()
            ),
            None,
            Some(&viewer),
        )
        .await
        .status(),
        StatusCode::OK
    );

    let (width, height) = (300, 300);
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let pixel = match (x < width / 2, y < height / 2) {
                (true, true) => [0, 0, 0, 0],
                (false, true) => {
                    let [r, g, b] = PALETTE_RGB[0];
                    [r, g, b, 255]
                }
                (true, false) => {
                    let [r, g, b] = PALETTE_RGB[3];
                    [r, g, b, 255]
                }
                (false, false) => {
                    let [r, g, b] = PALETTE_RGB[9];
                    [r, g, b, 255]
                }
            };
            rgba.extend_from_slice(&pixel);
        }
    }
    let (source, _, _) = upload(&router, &owner, &project_id, width, height, &rgba).await;
    let mut mapped_settings = settings();
    mapped_settings.dithering = DitherMode::None;
    mapped_settings.mappings.push(oreak_core::PaletteMapping {
        source_rgb: PALETTE_RGB[0],
        target_color: 4,
    });
    let prepared = request(
        &router,
        Method::POST,
        &format!("/api/projects/{project_id}/images/{source}/prepare"),
        Some(json!({"name":"Thumbnail source","settings":mapped_settings})),
        Some(&owner),
    )
    .await;
    assert_eq!(prepared.status(), StatusCode::CREATED);
    let prepared = json_body(prepared).await;
    let prepared_id = prepared["entry"]["id"].as_str().unwrap();
    let prepared_uri = format!("/api/projects/{project_id}/prepared-images/{prepared_id}");
    let thumbnail_uri = format!("{prepared_uri}/thumbnail");

    assert_eq!(
        request(&router, Method::GET, &thumbnail_uri, None, None)
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(&router, Method::GET, &thumbnail_uri, None, Some(&outsider))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &router,
            Method::GET,
            &format!("/api/projects/{other_project}/prepared-images/{prepared_id}/thumbnail"),
            None,
            Some(&owner),
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );

    let before =
        json_body(request(&router, Method::GET, &prepared_uri, None, Some(&owner)).await).await;
    let response = request(&router, Method::GET, &thumbnail_uri, None, Some(&viewer)).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[CONTENT_TYPE], "image/png");
    assert_eq!(response.headers()[CACHE_CONTROL], "private, no-store");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    let etag = response.headers()[ETAG].clone();
    let png = response.into_body().collect().await.unwrap().to_bytes();
    let thumbnail = image::load_from_memory(&png).unwrap().into_rgba8();
    assert_eq!(thumbnail.dimensions(), (256, 256));
    assert_eq!(thumbnail.get_pixel(0, 0).0, [0, 0, 0, 0]);
    // The thumbnail must reflect the authoritative custom mapping, not the original.
    assert_eq!(
        prepared["entry"]["settings"]["mappings"][0]["target_color"],
        4
    );
    let [r4, g4, b4] = PALETTE_RGB[3];
    assert_eq!(thumbnail.get_pixel(255, 0).0, [r4, g4, b4, 255]);
    assert_eq!(thumbnail.get_pixel(0, 255).0, [r4, g4, b4, 255]);
    let [r10, g10, b10] = PALETTE_RGB[9];
    assert_eq!(thumbnail.get_pixel(255, 255).0, [r10, g10, b10, 255]);

    let repeated = request(&router, Method::GET, &thumbnail_uri, None, Some(&owner)).await;
    assert_eq!(repeated.headers()[ETAG], etag);
    assert_eq!(
        repeated.into_body().collect().await.unwrap().to_bytes(),
        png
    );
    let after =
        json_body(request(&router, Method::GET, &prepared_uri, None, Some(&owner)).await).await;
    assert_eq!(after, before);
}

#[tokio::test]
async fn prepared_images_reject_invalid_inputs_and_enforce_catalog_quota() {
    let router = build_application().into_router();
    let (owner, user) = register(&router, "prepared-quota@example.com").await;
    let project_id = project(&router, &owner, &user).await;
    let (source, _, _) = upload(&router, &owner, &project_id, 1, 1, &[255, 139, 104, 255]).await;
    let uri = format!("/api/projects/{project_id}/images/{source}/prepare");
    for name in [
        "".to_owned(),
        " \t ".to_owned(),
        "a\nname".to_owned(),
        "x".repeat(121),
    ] {
        assert_eq!(
            request(
                &router,
                Method::POST,
                &uri,
                Some(json!({"name":name,"settings":settings()})),
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
    }
    for colors in [vec![], vec![0], vec![11], vec![1, 1]] {
        let mut invalid = settings();
        invalid.enabled_colors = colors;
        assert_eq!(
            request(
                &router,
                Method::POST,
                &uri,
                Some(json!({"name":"Invalid","settings":invalid})),
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
    }
    let mut invalid = settings();
    invalid.background = Some(2);
    assert_eq!(
        request(
            &router,
            Method::POST,
            &uri,
            Some(json!({"name":"Invalid","settings":invalid})),
            Some(&owner)
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    for invalid in [
        json!({"name":"Invalid","settings":{"enabled_colors":[1],"dithering":"unknown","alpha_threshold":128,"background":null}}),
        json!({"name":"Invalid","settings":settings(),"width":999}),
    ] {
        assert_eq!(
            request(&router, Method::POST, &uri, Some(invalid), Some(&owner))
                .await
                .status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    for i in 0..64 {
        assert_eq!(
            request(
                &router,
                Method::POST,
                &uri,
                Some(json!({"name":format!("Prepared {i}"),"settings":settings()})),
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::CREATED
        );
    }
    let exceeded = request(
        &router,
        Method::POST,
        &uri,
        Some(json!({"name":"Too many","settings":settings()})),
        Some(&owner),
    )
    .await;
    assert_eq!(exceeded.status(), StatusCode::BAD_REQUEST);
    assert!(
        json_body(exceeded).await["error"]
            .as_str()
            .unwrap()
            .contains("template limit")
    );
    let listed = json_body(
        request(
            &router,
            Method::GET,
            &format!("/api/projects/{project_id}/prepared-images"),
            None,
            Some(&owner),
        )
        .await,
    )
    .await;
    assert_eq!(listed["images"].as_array().unwrap().len(), 64);
}

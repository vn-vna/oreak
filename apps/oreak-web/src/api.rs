#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use serde::{Deserialize, Serialize};

pub const EDIT_TIMELINE_CAPABILITY: &str = "edit_timeline";
pub const MANAGE_MEMBERS_CAPABILITY: &str = "manage_members";
pub const MANAGE_THEME_CAPABILITY: &str = "manage_theme";

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct UserSummary {
    pub id: String,
    pub email: String,
    pub personal_workspace_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct WorkspaceSummary {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub role: String,
    pub project_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ProjectSummary {
    pub id: String,
    pub workspace_id: String,
    pub name: String,
    pub workspace_role: Option<String>,
    pub roles: Vec<String>,
    pub capabilities: Vec<String>,
    pub level_count: usize,
    pub plugin_manifest_format_version: u32,
    pub default_theme: String,
}

impl ProjectSummary {
    pub fn can_edit_timeline(&self) -> bool {
        has_edit_timeline(&self.capabilities)
    }

    pub fn can_manage_members(&self) -> bool {
        has_capability(&self.capabilities, MANAGE_MEMBERS_CAPABILITY)
    }

    pub fn can_manage_theme(&self) -> bool {
        has_capability(&self.capabilities, MANAGE_THEME_CAPABILITY)
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct CatalogSnapshot {
    pub workspaces: Vec<WorkspaceSummary>,
    pub projects: Vec<CatalogProject>,
    pub invitations: Vec<ProjectInvitationSummary>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct CatalogProject {
    pub project: ProjectSummary,
    pub levels: Vec<LevelSummary>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct MembershipSummary {
    pub user_id: String,
    pub email: Option<String>,
    pub roles: Vec<String>,
    pub capabilities: Vec<String>,
    pub joined_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ProjectInvitationSummary {
    pub id: String,
    pub project_id: String,
    pub project_name: String,
    pub invitee_email: String,
    pub roles: Vec<String>,
    pub invited_by: String,
    pub invited_at_ms: i64,
    pub expires_at_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ProjectConfiguration {
    pub project: ProjectSummary,
    pub members: Vec<MembershipSummary>,
    pub invitations: Vec<ProjectInvitationSummary>,
    pub members_visible: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct LevelSummary {
    pub id: String,
    pub name: String,
    pub duration_seconds: f64,
    pub revision_count: usize,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct LevelConfiguration {
    pub name: String,
    pub duration_seconds: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShapeDefinition {
    pub width: u8,
    pub height: u8,
    pub occupied_mask: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ShapeCatalogEntry {
    pub id: String,
    pub name: String,
    pub shape: ShapeDefinition,
    pub created_by: String,
    pub created_at_ms: i64,
    pub updated_by: String,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ShapeCatalogList {
    pub shapes: Vec<ShapeCatalogEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CredentialsRequest<'a> {
    pub email: &'a str,
    pub password: &'a str,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CreateProjectRequest<'a> {
    pub workspace_id: &'a str,
    pub name: &'a str,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CreateLevelRequest<'a> {
    pub name: &'a str,
    pub duration_seconds: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UpdateLevelConfigurationRequest<'a> {
    pub name: &'a str,
    pub duration_seconds: f64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SaveShapeCatalogEntryRequest<'a> {
    pub name: &'a str,
    pub shape: ShapeDefinition,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct InviteProjectMemberRequest<'a> {
    pub email: &'a str,
    pub roles: &'a [String],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct UpdateProjectMemberRequest<'a> {
    pub roles: &'a [String],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct UpdateProjectConfigurationRequest<'a> {
    pub default_theme: &'a str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiError {
    pub status: Option<u16>,
    pub message: String,
    pub retry_after_seconds: Option<u64>,
}

impl ApiError {
    pub fn network(message: impl Into<String>) -> Self {
        Self {
            status: None,
            message: message.into(),
            retry_after_seconds: None,
        }
    }

    pub fn is_unauthorized(&self) -> bool {
        self.status == Some(401)
    }
}

#[derive(Deserialize)]
struct ErrorBody {
    error: String,
}

pub fn parse_api_error(status: u16, body: &str, retry_after: Option<&str>) -> ApiError {
    let message = serde_json::from_str::<ErrorBody>(body)
        .map(|body| body.error)
        .unwrap_or_else(|_| format!("request failed with HTTP {status}"));
    ApiError {
        status: Some(status),
        message,
        retry_after_seconds: retry_after.and_then(|value| value.parse().ok()),
    }
}

pub fn has_edit_timeline(capabilities: &[String]) -> bool {
    has_capability(capabilities, EDIT_TIMELINE_CAPABILITY)
}

pub fn has_capability(capabilities: &[String], expected: &str) -> bool {
    capabilities.iter().any(|capability| capability == expected)
}

pub fn draft_storage_key(user_id: &str, project_id: &str, level_id: &str) -> String {
    format!("oreak.draft.{user_id}.{project_id}.{level_id}.v1")
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy, Debug, Default)]
pub struct RestClient;

#[cfg(target_arch = "wasm32")]
impl RestClient {
    pub async fn me(self) -> Result<UserSummary, ApiError> {
        self.get("/api/auth/me").await
    }

    pub async fn login(self, email: &str, password: &str) -> Result<UserSummary, ApiError> {
        self.post("/api/auth/login", &CredentialsRequest { email, password })
            .await
    }

    pub async fn register(self, email: &str, password: &str) -> Result<UserSummary, ApiError> {
        self.post(
            "/api/auth/register",
            &CredentialsRequest { email, password },
        )
        .await
    }

    pub async fn logout(self) -> Result<(), ApiError> {
        self.request_empty("POST", "/api/auth/logout", None).await
    }

    pub async fn catalog(self) -> Result<CatalogSnapshot, ApiError> {
        self.get("/api/catalog").await
    }

    pub async fn create_project(
        self,
        workspace_id: &str,
        name: &str,
    ) -> Result<ProjectSummary, ApiError> {
        self.post(
            "/api/projects",
            &CreateProjectRequest { workspace_id, name },
        )
        .await
    }

    pub async fn create_level(
        self,
        project_id: &str,
        name: &str,
        duration_seconds: f64,
    ) -> Result<LevelSummary, ApiError> {
        self.post(
            &format!("/api/projects/{project_id}/levels"),
            &CreateLevelRequest {
                name,
                duration_seconds,
            },
        )
        .await
    }

    pub async fn level_configuration(
        self,
        project_id: &str,
        level_id: &str,
    ) -> Result<LevelConfiguration, ApiError> {
        self.get(&format!(
            "/api/projects/{project_id}/levels/{level_id}/configuration"
        ))
        .await
    }

    pub async fn update_level_configuration(
        self,
        project_id: &str,
        level_id: &str,
        name: &str,
        duration_seconds: f64,
    ) -> Result<LevelConfiguration, ApiError> {
        self.put(
            &format!("/api/projects/{project_id}/levels/{level_id}/configuration"),
            &UpdateLevelConfigurationRequest {
                name,
                duration_seconds,
            },
        )
        .await
    }

    pub async fn project_configuration(
        self,
        project_id: &str,
    ) -> Result<ProjectConfiguration, ApiError> {
        self.get(&format!("/api/projects/{project_id}/configuration"))
            .await
    }

    pub async fn invite_project_member(
        self,
        project_id: &str,
        email: &str,
        roles: &[String],
    ) -> Result<ProjectInvitationSummary, ApiError> {
        self.post(
            &format!("/api/projects/{project_id}/invitations"),
            &InviteProjectMemberRequest { email, roles },
        )
        .await
    }

    pub async fn accept_project_invitation(
        self,
        project_id: &str,
        invitation_id: &str,
    ) -> Result<ProjectSummary, ApiError> {
        self.post_empty_json(&format!(
            "/api/projects/{project_id}/invitations/{invitation_id}/accept"
        ))
        .await
    }

    pub async fn update_project_member(
        self,
        project_id: &str,
        member_id: &str,
        roles: &[String],
    ) -> Result<MembershipSummary, ApiError> {
        self.put(
            &format!("/api/projects/{project_id}/members/{member_id}"),
            &UpdateProjectMemberRequest { roles },
        )
        .await
    }

    pub async fn update_project_configuration(
        self,
        project_id: &str,
        default_theme: &str,
    ) -> Result<ProjectSummary, ApiError> {
        self.put(
            &format!("/api/projects/{project_id}/configuration"),
            &UpdateProjectConfigurationRequest { default_theme },
        )
        .await
    }

    pub async fn list_shape_catalog(
        self,
        project_id: &str,
    ) -> Result<Vec<ShapeCatalogEntry>, ApiError> {
        self.get::<ShapeCatalogList>(&format!("/api/projects/{project_id}/shapes"))
            .await
            .map(|response| response.shapes)
    }

    pub async fn create_shape_catalog_entry(
        self,
        project_id: &str,
        name: &str,
        shape: ShapeDefinition,
    ) -> Result<ShapeCatalogEntry, ApiError> {
        self.post(
            &format!("/api/projects/{project_id}/shapes"),
            &SaveShapeCatalogEntryRequest { name, shape },
        )
        .await
    }

    pub async fn update_shape_catalog_entry(
        self,
        project_id: &str,
        shape_id: &str,
        name: &str,
        shape: ShapeDefinition,
    ) -> Result<ShapeCatalogEntry, ApiError> {
        self.put(
            &format!("/api/projects/{project_id}/shapes/{shape_id}"),
            &SaveShapeCatalogEntryRequest { name, shape },
        )
        .await
    }

    pub async fn delete_shape_catalog_entry(
        self,
        project_id: &str,
        shape_id: &str,
    ) -> Result<(), ApiError> {
        self.request_empty(
            "DELETE",
            &format!("/api/projects/{project_id}/shapes/{shape_id}"),
            None,
        )
        .await
    }

    async fn get<T: serde::de::DeserializeOwned>(self, path: &str) -> Result<T, ApiError> {
        self.request_json("GET", path, None).await
    }

    async fn post<T: serde::de::DeserializeOwned, B: Serialize>(
        self,
        path: &str,
        body: &B,
    ) -> Result<T, ApiError> {
        let body = serde_json::to_string(body)
            .map_err(|error| ApiError::network(format!("unable to encode request: {error}")))?;
        self.request_json("POST", path, Some(body)).await
    }

    async fn put<T: serde::de::DeserializeOwned, B: Serialize>(
        self,
        path: &str,
        body: &B,
    ) -> Result<T, ApiError> {
        let body = serde_json::to_string(body)
            .map_err(|error| ApiError::network(format!("unable to encode request: {error}")))?;
        self.request_json("PUT", path, Some(body)).await
    }

    async fn post_empty_json<T: serde::de::DeserializeOwned>(
        self,
        path: &str,
    ) -> Result<T, ApiError> {
        self.request_json("POST", path, Some("{}".to_owned())).await
    }

    async fn request_json<T: serde::de::DeserializeOwned>(
        self,
        method: &str,
        path: &str,
        body: Option<String>,
    ) -> Result<T, ApiError> {
        let (status, text, retry_after) = send(method, path, body).await?;
        if !(200..300).contains(&status) {
            return Err(parse_api_error(status, &text, retry_after.as_deref()));
        }
        serde_json::from_str(&text)
            .map_err(|error| ApiError::network(format!("invalid API response: {error}")))
    }

    async fn request_empty(
        self,
        method: &str,
        path: &str,
        body: Option<String>,
    ) -> Result<(), ApiError> {
        let (status, text, retry_after) = send(method, path, body).await?;
        if (200..300).contains(&status) {
            Ok(())
        } else {
            Err(parse_api_error(status, &text, retry_after.as_deref()))
        }
    }
}

#[cfg(target_arch = "wasm32")]
async fn send(
    method: &str,
    path: &str,
    body: Option<String>,
) -> Result<(u16, String, Option<String>), ApiError> {
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{Request, RequestCredentials, RequestInit, Response};

    let init = RequestInit::new();
    init.set_method(method);
    init.set_credentials(RequestCredentials::Include);
    if let Some(body) = body.as_ref() {
        init.set_body(&JsValue::from_str(body));
    }
    let request = Request::new_with_str_and_init(path, &init)
        .map_err(|error| ApiError::network(js_error(error)))?;
    if body.is_some() {
        request
            .headers()
            .set("Content-Type", "application/json")
            .map_err(|error| ApiError::network(js_error(error)))?;
    }
    request
        .headers()
        .set("Accept", "application/json")
        .map_err(|error| ApiError::network(js_error(error)))?;

    let window =
        web_sys::window().ok_or_else(|| ApiError::network("browser window unavailable"))?;
    let response = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|error| ApiError::network(js_error(error)))?
        .dyn_into::<Response>()
        .map_err(|error| ApiError::network(js_error(error)))?;
    let status = response.status();
    let retry_after = response.headers().get("Retry-After").ok().flatten();
    let text = JsFuture::from(
        response
            .text()
            .map_err(|error| ApiError::network(js_error(error)))?,
    )
    .await
    .map_err(|error| ApiError::network(js_error(error)))?
    .as_string()
    .unwrap_or_default();
    Ok((status, text, retry_after))
}

#[cfg(target_arch = "wasm32")]
fn js_error(error: wasm_bindgen::JsValue) -> String {
    error
        .as_string()
        .unwrap_or_else(|| "browser request failed".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(workspace_id: &str, capabilities: &[&str]) -> ProjectSummary {
        ProjectSummary {
            id: format!("project-{workspace_id}"),
            workspace_id: workspace_id.to_owned(),
            name: "Test project".to_owned(),
            workspace_role: Some("owner".to_owned()),
            roles: vec!["owner".to_owned()],
            capabilities: capabilities
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            level_count: 0,
            plugin_manifest_format_version: 1,
            default_theme: "dark".to_owned(),
        }
    }

    #[test]
    fn rest_dtos_deserialize_server_shapes() {
        let user: UserSummary = serde_json::from_str(
            r#"{"id":"usr_1","email":"a@example.com","personal_workspace_id":"ws_1"}"#,
        )
        .unwrap();
        let catalog: CatalogSnapshot = serde_json::from_str(
            r#"{"workspaces":[{"id":"ws_1","name":"Personal","kind":"personal","role":"owner","project_count":1}],"projects":[{"project":{"id":"prj_1","workspace_id":"ws_1","name":"Alpha","workspace_role":"owner","roles":["owner"],"capabilities":["edit_timeline"],"level_count":1,"plugin_manifest_format_version":1,"default_theme":"dark"},"levels":[{"id":"lvl_1","name":"Opening","duration_seconds":90.0,"revision_count":0}]}],"invitations":[]}"#,
        )
        .unwrap();
        let shapes: ShapeCatalogList = serde_json::from_str(
            r#"{"shapes":[{"id":"shp_1","name":"Corner","shape":{"width":2,"height":2,"occupied_mask":7},"created_by":"usr_1","created_at_ms":1000,"updated_by":"usr_1","updated_at_ms":1000}]}"#,
        )
        .unwrap();

        assert_eq!(user.personal_workspace_id, "ws_1");
        assert!(catalog.projects[0].project.can_edit_timeline());
        assert_eq!(catalog.projects[0].levels[0].name, "Opening");
        assert_eq!(catalog.projects[0].levels[0].duration_seconds, 90.0);
        assert_eq!(shapes.shapes[0].shape.occupied_mask, 7);
    }

    #[test]
    fn edit_capability_requires_an_exact_match() {
        assert!(has_edit_timeline(&["edit_timeline".to_owned()]));
        assert!(!has_edit_timeline(&["custom:edit_timeline".to_owned()]));
        assert!(!has_edit_timeline(&["edit_timeline_extra".to_owned()]));
    }

    #[test]
    fn configuration_capabilities_require_exact_matches() {
        let project = project("ws-a", &["manage_members", "manage_theme"]);
        assert!(project.can_manage_members());
        assert!(project.can_manage_theme());
        assert!(!has_capability(
            &["custom:manage_members".to_owned()],
            MANAGE_MEMBERS_CAPABILITY
        ));
    }

    #[test]
    fn draft_keys_are_scoped_to_user_project_and_level() {
        let key = draft_storage_key("usr-a", "prj-b", "lvl-c");
        assert_eq!(key, "oreak.draft.usr-a.prj-b.lvl-c.v1");
        assert_ne!(key, draft_storage_key("usr-b", "prj-b", "lvl-c"));
        assert_ne!(key, draft_storage_key("usr-a", "prj-c", "lvl-c"));
        assert_ne!(key, draft_storage_key("usr-a", "prj-b", "lvl-d"));
    }

    #[test]
    fn api_errors_capture_server_message_and_retry_after() {
        let error = parse_api_error(429, r#"{"error":"slow down"}"#, Some("60"));
        assert_eq!(error.status, Some(429));
        assert_eq!(error.message, "slow down");
        assert_eq!(error.retry_after_seconds, Some(60));
    }
}

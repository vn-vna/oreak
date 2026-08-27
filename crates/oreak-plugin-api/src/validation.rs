use std::{collections::BTreeMap, error::Error, fmt};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    Capability, CustomEntityInstance, FieldControl, HookKind, HookOutput, LegacyCodecDeclaration,
    LocalId, MANIFEST_FORMAT_VERSION, MAX_OCCUPIED_CELLS, PluginManifest, RenderGeometry,
    RenderPoint, RenderRect, SpatialEnvelope,
};

const MAX_TEXT_LEN: usize = 4_096;
const MAX_OUTPUT_ITEMS: usize = 4_096;
const MAX_JSON_DEPTH: usize = 64;
const MAX_JSON_NODES: usize = 100_000;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationIssue {
    pub path: String,
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationErrors {
    pub issues: Vec<ValidationIssue>,
}

impl fmt::Display for ValidationErrors {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "contract validation failed with {} issue(s)",
            self.issues.len()
        )
    }
}

impl Error for ValidationErrors {}

pub trait Validate {
    fn validate(&self) -> Result<(), ValidationErrors>;
}

impl Validate for PluginManifest {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut issues = Vec::new();
        if self.manifest_version != MANIFEST_FORMAT_VERSION {
            issue(
                &mut issues,
                "/manifest_version",
                "unsupported_manifest_version",
                "manifest_version is not supported by this API",
            );
        }

        let hooks = collect_hooks(self, &mut issues);
        for (index, hook) in self.hooks.iter().enumerate() {
            let capability = match hook.kind {
                HookKind::Command => Capability::ProposeCommands,
                HookKind::InspectorAction | HookKind::DialogSubmit => Capability::ContributeUi,
                HookKind::Validate => Capability::Validate,
                HookKind::Generate => Capability::Generate,
                HookKind::Render => Capability::Render,
                HookKind::MigrateSchema => Capability::ContributeSchema,
                HookKind::EncodeLegacyLevelData | HookKind::DecodeLegacyLevelData => {
                    Capability::LegacyLevelDataCodec
                }
            };
            if !self.capabilities.contains(&capability) {
                issue(
                    &mut issues,
                    &format!("/hooks/{index}/kind"),
                    "missing_capability",
                    &format!("hook kind requires the {capability:?} capability"),
                );
            }
        }
        unique_ids(
            &mut issues,
            "/schemas",
            self.schemas.iter().map(|item| &item.id),
        );
        unique_ids(
            &mut issues,
            "/custom_entities",
            self.custom_entities.iter().map(|item| &item.id),
        );
        unique_ids(
            &mut issues,
            "/commands",
            self.commands.iter().map(|item| &item.id),
        );
        unique_ids(
            &mut issues,
            "/menus",
            self.menus.iter().map(|item| &item.id),
        );
        unique_ids(
            &mut issues,
            "/shortcuts",
            self.shortcuts.iter().map(|item| &item.id),
        );
        unique_ids(
            &mut issues,
            "/inspectors",
            self.inspectors.iter().map(|item| &item.id),
        );
        unique_ids(
            &mut issues,
            "/dialogs",
            self.dialogs.iter().map(|item| &item.id),
        );
        unique_ids(
            &mut issues,
            "/renderers",
            self.renderers.iter().map(|item| &item.id),
        );
        unique_ids(
            &mut issues,
            "/theme_tokens",
            self.theme_tokens.iter().map(|item| &item.id),
        );
        unique_ids(
            &mut issues,
            "/validators",
            self.validators.iter().map(|item| &item.id),
        );
        unique_ids(
            &mut issues,
            "/generators",
            self.generators.iter().map(|item| &item.id),
        );
        unique_ids(
            &mut issues,
            "/image_assets",
            self.image_assets.iter().map(|item| &item.id),
        );

        require_capability(
            self,
            &mut issues,
            Capability::ContributeSchema,
            !self.schemas.is_empty() || !self.migrations.is_empty(),
            "/capabilities",
        );
        require_capability(
            self,
            &mut issues,
            Capability::ContributeCustomEntities,
            !self.custom_entities.is_empty(),
            "/capabilities",
        );
        require_capability(
            self,
            &mut issues,
            Capability::ProposeCommands,
            !self.commands.is_empty(),
            "/capabilities",
        );
        require_capability(
            self,
            &mut issues,
            Capability::ContributeUi,
            !self.menus.is_empty()
                || !self.shortcuts.is_empty()
                || !self.inspectors.is_empty()
                || !self.dialogs.is_empty(),
            "/capabilities",
        );
        require_capability(
            self,
            &mut issues,
            Capability::Render,
            !self.renderers.is_empty() || !self.theme_tokens.is_empty(),
            "/capabilities",
        );
        require_capability(
            self,
            &mut issues,
            Capability::Validate,
            !self.validators.is_empty(),
            "/capabilities",
        );
        require_capability(
            self,
            &mut issues,
            Capability::Generate,
            !self.generators.is_empty(),
            "/capabilities",
        );
        require_capability(
            self,
            &mut issues,
            Capability::LegacyLevelDataCodec,
            matches!(
                self.legacy_level_data_codec,
                LegacyCodecDeclaration::Provided { .. }
            ),
            "/capabilities",
        );

        for (index, schema) in self.schemas.iter().enumerate() {
            nonempty_text(
                &mut issues,
                &format!("/schemas/{index}/title"),
                &schema.title,
            );
            validate_json(
                &mut issues,
                &format!("/schemas/{index}/root_schema"),
                &schema.root_schema,
            );
        }
        for (index, migration) in self.migrations.iter().enumerate() {
            if migration.from_version >= migration.to_version {
                issue(
                    &mut issues,
                    &format!("/migrations/{index}"),
                    "invalid_migration_range",
                    "migration must increase the schema version",
                );
            }
            if !self
                .schemas
                .iter()
                .any(|schema| schema.id == migration.schema_id)
            {
                issue(
                    &mut issues,
                    &format!("/migrations/{index}/schema_id"),
                    "unknown_schema",
                    "migration references an undeclared schema",
                );
            }
            require_hook(
                &hooks,
                &mut issues,
                &format!("/migrations/{index}/hook"),
                &migration.hook,
                HookKind::MigrateSchema,
            );
        }
        for (index, entity) in self.custom_entities.iter().enumerate() {
            nonempty_text(
                &mut issues,
                &format!("/custom_entities/{index}/display_name"),
                &entity.display_name,
            );
            if !self.schemas.iter().any(|schema| {
                schema.id == entity.data_schema && schema.version == entity.data_schema_version
            }) {
                issue(
                    &mut issues,
                    &format!("/custom_entities/{index}/data_schema"),
                    "unknown_schema_version",
                    "custom entity references an undeclared schema version",
                );
            }
        }
        for (index, command) in self.commands.iter().enumerate() {
            nonempty_text(
                &mut issues,
                &format!("/commands/{index}/label"),
                &command.label,
            );
            require_hook(
                &hooks,
                &mut issues,
                &format!("/commands/{index}/hook"),
                &command.hook,
                HookKind::Command,
            );
        }
        for (index, menu) in self.menus.iter().enumerate() {
            require_command(self, &mut issues, index, "menus", &menu.command);
        }
        for (index, shortcut) in self.shortcuts.iter().enumerate() {
            require_command(self, &mut issues, index, "shortcuts", &shortcut.command);
            nonempty_text(
                &mut issues,
                &format!("/shortcuts/{index}/key"),
                &shortcut.key,
            );
        }
        for (index, inspector) in self.inspectors.iter().enumerate() {
            if !self
                .custom_entities
                .iter()
                .any(|entity| entity.id == inspector.entity_type)
            {
                issue(
                    &mut issues,
                    &format!("/inspectors/{index}/entity_type"),
                    "unknown_entity_type",
                    "inspector references an undeclared custom entity type",
                );
            }
            nonempty_text(
                &mut issues,
                &format!("/inspectors/{index}/title"),
                &inspector.title,
            );
            for (field_index, field) in inspector.fields.iter().enumerate() {
                validate_pointer(
                    &mut issues,
                    &format!("/inspectors/{index}/fields/{field_index}/data_pointer"),
                    &field.data_pointer,
                );
                validate_control(
                    &mut issues,
                    &format!("/inspectors/{index}/fields/{field_index}/control"),
                    &field.control,
                );
            }
            for (action_index, action) in inspector.actions.iter().enumerate() {
                require_hook(
                    &hooks,
                    &mut issues,
                    &format!("/inspectors/{index}/actions/{action_index}/hook"),
                    &action.hook,
                    HookKind::InspectorAction,
                );
            }
        }
        for (index, dialog) in self.dialogs.iter().enumerate() {
            require_hook(
                &hooks,
                &mut issues,
                &format!("/dialogs/{index}/submit_hook"),
                &dialog.submit_hook,
                HookKind::DialogSubmit,
            );
            for (field_index, field) in dialog.fields.iter().enumerate() {
                validate_control(
                    &mut issues,
                    &format!("/dialogs/{index}/fields/{field_index}/control"),
                    &field.control,
                );
                if let Some(default_value) = &field.default_value {
                    validate_json(
                        &mut issues,
                        &format!("/dialogs/{index}/fields/{field_index}/default_value"),
                        default_value,
                    );
                }
            }
        }
        for (index, renderer) in self.renderers.iter().enumerate() {
            if !self
                .custom_entities
                .iter()
                .any(|entity| entity.id == renderer.entity_type)
            {
                issue(
                    &mut issues,
                    &format!("/renderers/{index}/entity_type"),
                    "unknown_entity_type",
                    "renderer references an undeclared custom entity type",
                );
            }
            require_hook(
                &hooks,
                &mut issues,
                &format!("/renderers/{index}/hook"),
                &renderer.hook,
                HookKind::Render,
            );
        }
        for (index, validator) in self.validators.iter().enumerate() {
            require_hook(
                &hooks,
                &mut issues,
                &format!("/validators/{index}/hook"),
                &validator.hook,
                HookKind::Validate,
            );
        }
        for (index, generator) in self.generators.iter().enumerate() {
            require_hook(
                &hooks,
                &mut issues,
                &format!("/generators/{index}/hook"),
                &generator.hook,
                HookKind::Generate,
            );
            if !self.schemas.iter().any(|schema| {
                schema.id == generator.output_schema
                    && schema.version == generator.output_schema_version
            }) {
                issue(
                    &mut issues,
                    &format!("/generators/{index}/output_schema"),
                    "unknown_schema_version",
                    "generator references an undeclared output schema version",
                );
            }
        }
        for (index, image) in self.image_assets.iter().enumerate() {
            if image.width == 0 || image.height == 0 {
                issue(
                    &mut issues,
                    &format!("/image_assets/{index}"),
                    "invalid_image_dimensions",
                    "image dimensions must be non-zero",
                );
            }
        }
        if let LegacyCodecDeclaration::Provided {
            encode_hook,
            decode_hook,
            ..
        } = &self.legacy_level_data_codec
        {
            require_hook(
                &hooks,
                &mut issues,
                "/legacy_level_data_codec/encode_hook",
                encode_hook,
                HookKind::EncodeLegacyLevelData,
            );
            require_hook(
                &hooks,
                &mut issues,
                "/legacy_level_data_codec/decode_hook",
                decode_hook,
                HookKind::DecodeLegacyLevelData,
            );
        }

        finish(issues)
    }
}

impl Validate for SpatialEnvelope {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut issues = Vec::new();
        if self.occupied_cells.is_empty() {
            issue(
                &mut issues,
                "/occupied_cells",
                "empty_spatial_envelope",
                "at least one occupied cell is required",
            );
        }
        if self.occupied_cells.len() > MAX_OCCUPIED_CELLS {
            issue(
                &mut issues,
                "/occupied_cells",
                "spatial_envelope_too_large",
                "occupied cell count exceeds the contract limit",
            );
        }
        let mut cells = self.occupied_cells.clone();
        cells.sort_unstable();
        if cells.windows(2).any(|pair| pair[0] == pair[1]) {
            issue(
                &mut issues,
                "/occupied_cells",
                "duplicate_occupied_cell",
                "occupied cells must be unique",
            );
        }
        finish(issues)
    }
}

impl Validate for CustomEntityInstance {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut issues = self
            .envelope
            .validate()
            .err()
            .map_or_else(Vec::new, |errors| {
                errors
                    .issues
                    .into_iter()
                    .map(|mut issue| {
                        issue.path = format!("/envelope{}", issue.path);
                        issue
                    })
                    .collect()
            });
        nonempty_text(&mut issues, "/entity_id", &self.entity_id);
        validate_json(&mut issues, "/plugin_data", &self.plugin_data);
        finish(issues)
    }
}

impl Validate for HookOutput {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut issues = Vec::new();
        let item_count = self.proposals.len()
            + self.diagnostics.len()
            + self.render_primitives.len()
            + self.generated_artifacts.len();
        if item_count > MAX_OUTPUT_ITEMS {
            issue(
                &mut issues,
                "",
                "too_many_output_items",
                "hook output exceeds the item count limit",
            );
        }
        for (index, proposal) in self.proposals.iter().enumerate() {
            validate_json(
                &mut issues,
                &format!("/proposals/{index}/payload"),
                &proposal.payload,
            );
        }
        for (index, diagnostic) in self.diagnostics.iter().enumerate() {
            nonempty_text(
                &mut issues,
                &format!("/diagnostics/{index}/message"),
                &diagnostic.message,
            );
        }
        for (index, primitive) in self.render_primitives.iter().enumerate() {
            validate_geometry(
                &mut issues,
                &format!("/render_primitives/{index}/geometry"),
                &primitive.geometry,
            );
        }
        for (index, artifact) in self.generated_artifacts.iter().enumerate() {
            validate_json(
                &mut issues,
                &format!("/generated_artifacts/{index}/value"),
                &artifact.value,
            );
        }
        finish(issues)
    }
}

fn collect_hooks<'a>(
    manifest: &'a PluginManifest,
    issues: &mut Vec<ValidationIssue>,
) -> BTreeMap<&'a LocalId, HookKind> {
    let mut hooks = BTreeMap::new();
    let mut exports = BTreeMap::new();
    for (index, hook) in manifest.hooks.iter().enumerate() {
        if hooks.insert(&hook.id, hook.kind).is_some() {
            issue(
                issues,
                &format!("/hooks/{index}/id"),
                "duplicate_id",
                "hook id is duplicated",
            );
        }
        if exports.insert(&hook.export, index).is_some() {
            issue(
                issues,
                &format!("/hooks/{index}/export"),
                "duplicate_hook_export",
                "hook export is duplicated",
            );
        }
    }
    hooks
}

fn unique_ids<'a>(
    issues: &mut Vec<ValidationIssue>,
    path: &str,
    ids: impl Iterator<Item = &'a LocalId>,
) {
    let mut seen = BTreeMap::new();
    for (index, id) in ids.enumerate() {
        if seen.insert(id, index).is_some() {
            issue(
                issues,
                &format!("{path}/{index}/id"),
                "duplicate_id",
                "contribution id is duplicated in its category",
            );
        }
    }
}

fn require_capability(
    manifest: &PluginManifest,
    issues: &mut Vec<ValidationIssue>,
    capability: Capability,
    needed: bool,
    path: &str,
) {
    if needed && !manifest.capabilities.contains(&capability) {
        issue(
            issues,
            path,
            "missing_capability",
            &format!("contributions require the {capability:?} capability"),
        );
    }
}

fn require_hook(
    hooks: &BTreeMap<&LocalId, HookKind>,
    issues: &mut Vec<ValidationIssue>,
    path: &str,
    hook: &LocalId,
    expected: HookKind,
) {
    match hooks.get(hook) {
        Some(actual) if *actual == expected => {}
        Some(_) => issue(
            issues,
            path,
            "wrong_hook_kind",
            "hook kind does not match its contribution",
        ),
        None => issue(
            issues,
            path,
            "unknown_hook",
            "contribution references an undeclared hook",
        ),
    }
}

fn require_command(
    manifest: &PluginManifest,
    issues: &mut Vec<ValidationIssue>,
    index: usize,
    category: &str,
    command: &LocalId,
) {
    if !manifest.commands.iter().any(|item| item.id == *command) {
        issue(
            issues,
            &format!("/{category}/{index}/command"),
            "unknown_command",
            "contribution references an undeclared command",
        );
    }
}

fn validate_pointer(issues: &mut Vec<ValidationIssue>, path: &str, pointer: &str) {
    if !pointer.is_empty() && !pointer.starts_with('/') {
        issue(
            issues,
            path,
            "invalid_json_pointer",
            "plugin data pointer must be empty or begin with '/'",
        );
    }
}

fn validate_control(issues: &mut Vec<ValidationIssue>, path: &str, control: &FieldControl) {
    match control {
        FieldControl::Number { min, max, step } => {
            if min.is_some_and(|value| !value.is_finite())
                || max.is_some_and(|value| !value.is_finite())
                || step.is_some_and(|value| !value.is_finite() || value <= 0.0)
            {
                issue(
                    issues,
                    path,
                    "invalid_number_control",
                    "number bounds must be finite and step must be positive",
                );
            }
            if matches!((min, max), (Some(min), Some(max)) if min > max) {
                issue(
                    issues,
                    path,
                    "invalid_number_range",
                    "number control minimum exceeds its maximum",
                );
            }
        }
        FieldControl::Select { options } if options.is_empty() => issue(
            issues,
            path,
            "empty_select_options",
            "select controls require at least one option",
        ),
        _ => {}
    }
}

fn validate_geometry(issues: &mut Vec<ValidationIssue>, path: &str, geometry: &RenderGeometry) {
    match geometry {
        RenderGeometry::Rect { rect, .. } | RenderGeometry::Image { rect, .. } => {
            validate_rect(issues, path, rect)
        }
        RenderGeometry::Circle { center, radius, .. } => {
            validate_point(issues, path, center);
            if !radius.is_finite() || *radius <= 0.0 {
                issue(
                    issues,
                    path,
                    "invalid_circle",
                    "circle radius must be finite and positive",
                );
            }
        }
        RenderGeometry::Polyline { points, width, .. } => {
            if points.len() < 2 || !width.is_finite() || *width <= 0.0 {
                issue(
                    issues,
                    path,
                    "invalid_polyline",
                    "polyline requires two points and a finite positive width",
                );
            }
            for point in points {
                validate_point(issues, path, point);
            }
        }
        RenderGeometry::Text {
            position,
            text,
            size,
            ..
        } => {
            validate_point(issues, path, position);
            nonempty_text(issues, path, text);
            if !size.is_finite() || *size <= 0.0 {
                issue(
                    issues,
                    path,
                    "invalid_text_size",
                    "text size must be finite and positive",
                );
            }
        }
    }
}

fn validate_rect(issues: &mut Vec<ValidationIssue>, path: &str, rect: &RenderRect) {
    if ![rect.x, rect.y, rect.width, rect.height]
        .iter()
        .all(|value| value.is_finite())
        || rect.width <= 0.0
        || rect.height <= 0.0
    {
        issue(
            issues,
            path,
            "invalid_rect",
            "rectangle coordinates must be finite and dimensions positive",
        );
    }
}

fn validate_point(issues: &mut Vec<ValidationIssue>, path: &str, point: &RenderPoint) {
    if !point.x.is_finite() || !point.y.is_finite() {
        issue(
            issues,
            path,
            "invalid_point",
            "render coordinates must be finite",
        );
    }
}

fn nonempty_text(issues: &mut Vec<ValidationIssue>, path: &str, text: &str) {
    if text.trim().is_empty() || text.len() > MAX_TEXT_LEN {
        issue(
            issues,
            path,
            "invalid_text",
            "text must be non-empty and at most 4096 bytes",
        );
    }
}

fn validate_json(issues: &mut Vec<ValidationIssue>, path: &str, value: &Value) {
    let mut nodes = 0;
    if !json_within_limits(value, 0, &mut nodes) {
        issue(
            issues,
            path,
            "json_too_complex",
            "JSON exceeds the depth or node-count limit",
        );
    }
}

fn json_within_limits(value: &Value, depth: usize, nodes: &mut usize) -> bool {
    *nodes += 1;
    if depth > MAX_JSON_DEPTH || *nodes > MAX_JSON_NODES {
        return false;
    }
    match value {
        Value::Array(values) => values
            .iter()
            .all(|value| json_within_limits(value, depth + 1, nodes)),
        Value::Object(values) => values
            .values()
            .all(|value| json_within_limits(value, depth + 1, nodes)),
        _ => true,
    }
}

fn issue(issues: &mut Vec<ValidationIssue>, path: &str, code: &str, message: &str) {
    issues.push(ValidationIssue {
        path: path.to_owned(),
        code: code.to_owned(),
        message: message.to_owned(),
    });
}

fn finish(issues: Vec<ValidationIssue>) -> Result<(), ValidationErrors> {
    if issues.is_empty() {
        Ok(())
    } else {
        Err(ValidationErrors { issues })
    }
}

use std::collections::BTreeSet;

use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ContentHash, LocalId, MANIFEST_FORMAT_VERSION, PluginId, ProjectPluginPin};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    ReadProject,
    ProposeCommands,
    ContributeSchema,
    ContributeCustomEntities,
    ContributeUi,
    Render,
    Validate,
    Generate,
    LegacyLevelDataCodec,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HookKind {
    Command,
    InspectorAction,
    DialogSubmit,
    Validate,
    Generate,
    Render,
    MigrateSchema,
    EncodeLegacyLevelData,
    DecodeLegacyLevelData,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HookDeclaration {
    pub id: LocalId,
    pub kind: HookKind,
    pub export: LocalId,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SchemaContribution {
    pub id: LocalId,
    pub version: u32,
    pub title: String,
    pub root_schema: Value,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SchemaMigration {
    pub schema_id: LocalId,
    pub from_version: u32,
    pub to_version: u32,
    pub hook: LocalId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CustomEntityContribution {
    pub id: LocalId,
    pub display_name: String,
    pub data_schema: LocalId,
    pub data_schema_version: u32,
    pub runtime: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommandContribution {
    pub id: LocalId,
    pub label: String,
    pub hook: LocalId,
    #[serde(default)]
    pub selection: SelectionRequirement,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionRequirement {
    #[default]
    None,
    AnyEntity,
    CustomEntity,
    Cells,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MenuLocation {
    Project,
    Edit,
    Entity,
    Tools,
    Context,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MenuContribution {
    pub id: LocalId,
    pub command: LocalId,
    pub location: MenuLocation,
    #[serde(default)]
    pub group: Option<LocalId>,
    #[serde(default)]
    pub order: i32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ShortcutModifier {
    Control,
    Alt,
    Shift,
    Meta,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ShortcutContribution {
    pub id: LocalId,
    pub command: LocalId,
    pub key: String,
    #[serde(default)]
    pub modifiers: BTreeSet<ShortcutModifier>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InspectorContribution {
    pub id: LocalId,
    pub entity_type: LocalId,
    pub title: String,
    #[serde(default)]
    pub fields: Vec<InspectorField>,
    #[serde(default)]
    pub actions: Vec<InspectorAction>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InspectorField {
    pub id: LocalId,
    pub label: String,
    /// RFC 6901 pointer into plugin-owned data, never into a core document.
    pub data_pointer: String,
    pub control: FieldControl,
    #[serde(default)]
    pub read_only: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum FieldControl {
    Text {
        #[serde(default)]
        multiline: bool,
        #[serde(default)]
        max_length: Option<u32>,
    },
    Number {
        #[serde(default)]
        min: Option<f64>,
        #[serde(default)]
        max: Option<f64>,
        #[serde(default)]
        step: Option<f64>,
    },
    Toggle,
    Select {
        options: Vec<SelectOption>,
    },
    ImageAsset,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SelectOption {
    pub value: String,
    pub label: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InspectorAction {
    pub id: LocalId,
    pub label: String,
    pub hook: LocalId,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DialogContribution {
    pub id: LocalId,
    pub title: String,
    pub submit_label: String,
    pub submit_hook: LocalId,
    #[serde(default)]
    pub fields: Vec<DialogField>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DialogField {
    pub id: LocalId,
    pub label: String,
    pub control: FieldControl,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub default_value: Option<Value>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidatorScope {
    Project,
    Level,
    Selection,
    CustomEntity,
    Release,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ValidatorContribution {
    pub id: LocalId,
    pub hook: LocalId,
    pub scope: ValidatorScope,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GeneratorContribution {
    pub id: LocalId,
    pub label: String,
    pub hook: LocalId,
    pub output_schema: LocalId,
    pub output_schema_version: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RenderContribution {
    pub id: LocalId,
    pub entity_type: LocalId,
    pub hook: LocalId,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeRole {
    EntityFill,
    EntityStroke,
    EntityText,
    DiagnosticError,
    DiagnosticWarning,
    DiagnosticInfo,
    Selection,
    Accent,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeTokenContribution {
    pub id: LocalId,
    pub role: ThemeRole,
    pub fallback: Rgba8,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Rgba8 {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub alpha: u8,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageMediaType {
    Png,
    Webp,
}

/// Content-addressed image metadata. Hosts decide how bytes are obtained.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImageAssetReference {
    pub id: LocalId,
    pub content_hash: ContentHash,
    pub media_type: ImageMediaType,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum LegacyCodecDeclaration {
    #[default]
    NotProvided,
    Provided {
        codec_version: Version,
        encode_hook: LocalId,
        decode_hook: LocalId,
        fixture_set_hash: ContentHash,
    },
}

/// Versioned plugin metadata and all capabilities visible to a host.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    pub manifest_version: u32,
    pub plugin_id: PluginId,
    pub plugin_version: Version,
    pub api_version: Version,
    pub source_hash: ContentHash,
    #[serde(default)]
    pub capabilities: BTreeSet<Capability>,
    #[serde(default)]
    pub hooks: Vec<HookDeclaration>,
    #[serde(default)]
    pub schemas: Vec<SchemaContribution>,
    #[serde(default)]
    pub migrations: Vec<SchemaMigration>,
    #[serde(default)]
    pub custom_entities: Vec<CustomEntityContribution>,
    #[serde(default)]
    pub commands: Vec<CommandContribution>,
    #[serde(default)]
    pub menus: Vec<MenuContribution>,
    #[serde(default)]
    pub shortcuts: Vec<ShortcutContribution>,
    #[serde(default)]
    pub inspectors: Vec<InspectorContribution>,
    #[serde(default)]
    pub dialogs: Vec<DialogContribution>,
    #[serde(default)]
    pub renderers: Vec<RenderContribution>,
    #[serde(default)]
    pub theme_tokens: Vec<ThemeTokenContribution>,
    #[serde(default)]
    pub validators: Vec<ValidatorContribution>,
    #[serde(default)]
    pub generators: Vec<GeneratorContribution>,
    #[serde(default)]
    pub image_assets: Vec<ImageAssetReference>,
    #[serde(default)]
    pub legacy_level_data_codec: LegacyCodecDeclaration,
}

impl PluginManifest {
    pub fn new(
        plugin_id: PluginId,
        plugin_version: Version,
        api_version: Version,
        source_hash: ContentHash,
    ) -> Self {
        Self {
            manifest_version: MANIFEST_FORMAT_VERSION,
            plugin_id,
            plugin_version,
            api_version,
            source_hash,
            capabilities: BTreeSet::new(),
            hooks: Vec::new(),
            schemas: Vec::new(),
            migrations: Vec::new(),
            custom_entities: Vec::new(),
            commands: Vec::new(),
            menus: Vec::new(),
            shortcuts: Vec::new(),
            inspectors: Vec::new(),
            dialogs: Vec::new(),
            renderers: Vec::new(),
            theme_tokens: Vec::new(),
            validators: Vec::new(),
            generators: Vec::new(),
            image_assets: Vec::new(),
            legacy_level_data_codec: LegacyCodecDeclaration::NotProvided,
        }
    }

    pub fn exact_pin(&self) -> ProjectPluginPin {
        ProjectPluginPin::from_manifest(self)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GridPoint {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CellOffset {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum CollisionSemantics {
    None,
    Solid {
        layer: LocalId,
        #[serde(default)]
        collides_with: BTreeSet<LocalId>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum SelectionSemantics {
    NotSelectable,
    Origin,
    OccupiedCells,
    Envelope { padding_cells: u16 },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SpatialEnvelope {
    pub origin: GridPoint,
    pub occupied_cells: Vec<CellOffset>,
    pub collision: CollisionSemantics,
    pub selection: SelectionSemantics,
}

/// Canonical custom-entity payload owned by a single exact plugin pin.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CustomEntityInstance {
    pub entity_id: String,
    pub plugin: ProjectPluginPin,
    pub entity_type: LocalId,
    pub runtime: bool,
    pub envelope: SpatialEnvelope,
    pub plugin_data: Value,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum CodecCertificationState {
    NotSubmitted,
    Pending {
        submission_hash: ContentHash,
    },
    Certified {
        codec_version: Version,
        certificate_hash: ContentHash,
    },
    Rejected {
        reason: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyCodecCertification {
    pub plugin: ProjectPluginPin,
    pub state: CodecCertificationState,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum DiagnosticTarget {
    Project,
    Level,
    Entity { entity_id: String },
    Cell { point: GridPoint },
    PluginData { entity_id: String, pointer: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Diagnostic {
    pub severity: DiagnosticSeverity,
    pub code: LocalId,
    pub message: String,
    #[serde(default)]
    pub target: Option<DiagnosticTarget>,
}

/// A requested core command. The host must parse and validate it before use.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommandProposal {
    pub id: LocalId,
    pub command: LocalId,
    #[serde(default)]
    pub payload: Value,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RenderPoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RenderRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ThemeTokenRef(pub LocalId);

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum RenderGeometry {
    Rect {
        rect: RenderRect,
        fill: ThemeTokenRef,
        #[serde(default)]
        stroke: Option<ThemeTokenRef>,
    },
    Circle {
        center: RenderPoint,
        radius: f32,
        fill: ThemeTokenRef,
        #[serde(default)]
        stroke: Option<ThemeTokenRef>,
    },
    Polyline {
        points: Vec<RenderPoint>,
        stroke: ThemeTokenRef,
        width: f32,
        #[serde(default)]
        closed: bool,
    },
    Text {
        position: RenderPoint,
        text: String,
        color: ThemeTokenRef,
        size: f32,
    },
    Image {
        rect: RenderRect,
        asset: LocalId,
        #[serde(default)]
        tint: Option<ThemeTokenRef>,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RenderPrimitive {
    pub id: LocalId,
    #[serde(default)]
    pub z_index: i32,
    pub geometry: RenderGeometry,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GeneratedArtifact {
    pub id: LocalId,
    pub schema: LocalId,
    pub schema_version: u32,
    pub value: Value,
}

/// The complete value a Lua hook may return to its host.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HookOutput {
    #[serde(default)]
    pub proposals: Vec<CommandProposal>,
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
    #[serde(default)]
    pub render_primitives: Vec<RenderPrimitive>,
    #[serde(default)]
    pub generated_artifacts: Vec<GeneratedArtifact>,
}

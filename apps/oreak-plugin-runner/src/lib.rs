//! Native Lua 5.4 sandbox and NDJSON protocol for untrusted Oreak plugins.

#![forbid(unsafe_code)]

use std::{
    cell::Cell,
    collections::{BTreeSet, HashSet},
    ffi::c_void,
    rc::Rc,
    time::{Duration, Instant},
};

use mlua::{
    ChunkMode, Error as LuaError, Function, HookTriggers, Lua, LuaOptions, LuaSerdeExt,
    RegistryKey, StdLib, Table, Value as LuaValue, VmState,
};
use oreak_plugin_api::{
    Capability, HookOutput, LocalId, PluginManifest, ProjectPluginPin, Validate, ValidationIssue,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

pub const MAX_REQUEST_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_SOURCE_BYTES: usize = 1024 * 1024;
pub const MAX_MEMORY_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_INSTRUCTIONS: u64 = 10_000_000;
pub const MAX_WALL_TIME_MS: u64 = 5_000;
pub const MAX_OUTPUT_BYTES: usize = 1024 * 1024;

const MIN_MEMORY_BYTES: usize = 1024 * 1024;
const MIN_INSTRUCTIONS: u64 = 1_000;
const HOOK_INTERVAL: u32 = 100;
const MAX_ERROR_BYTES: usize = 4_096;
const MAX_LUA_OUTPUT_DEPTH: usize = 64;
const MAX_LUA_OUTPUT_NODES: usize = 100_000;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ExecutionLimits {
    pub memory_bytes: usize,
    pub instruction_limit: u64,
    pub wall_time_ms: u64,
    pub output_bytes: usize,
}

impl Default for ExecutionLimits {
    fn default() -> Self {
        Self {
            memory_bytes: 16 * 1024 * 1024,
            instruction_limit: 500_000,
            wall_time_ms: 250,
            output_bytes: 256 * 1024,
        }
    }
}

impl ExecutionLimits {
    fn validate(&self) -> Result<(), RunnerError> {
        if !(MIN_MEMORY_BYTES..=MAX_MEMORY_BYTES).contains(&self.memory_bytes) {
            return Err(RunnerError::InvalidLimits(format!(
                "memory_bytes must be between {MIN_MEMORY_BYTES} and {MAX_MEMORY_BYTES}"
            )));
        }
        if !(MIN_INSTRUCTIONS..=MAX_INSTRUCTIONS).contains(&self.instruction_limit) {
            return Err(RunnerError::InvalidLimits(format!(
                "instruction_limit must be between {MIN_INSTRUCTIONS} and {MAX_INSTRUCTIONS}"
            )));
        }
        if self.wall_time_ms == 0 || self.wall_time_ms > MAX_WALL_TIME_MS {
            return Err(RunnerError::InvalidLimits(format!(
                "wall_time_ms must be between 1 and {MAX_WALL_TIME_MS}"
            )));
        }
        if self.output_bytes == 0 || self.output_bytes > MAX_OUTPUT_BYTES {
            return Err(RunnerError::InvalidLimits(format!(
                "output_bytes must be between 1 and {MAX_OUTPUT_BYTES}"
            )));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "request", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Load {
        request_id: String,
        manifest: Box<PluginManifest>,
        source: String,
        #[serde(default)]
        limits: ExecutionLimits,
    },
    Validate {
        request_id: String,
    },
    InvokeHook {
        request_id: String,
        hook: LocalId,
        input: Value,
    },
    Shutdown {
        request_id: String,
    },
}

impl Request {
    fn request_id(&self) -> &str {
        match self {
            Self::Load { request_id, .. }
            | Self::Validate { request_id }
            | Self::InvokeHook { request_id, .. }
            | Self::Shutdown { request_id } => request_id,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Response {
    Ok {
        request_id: Option<String>,
        result: Success,
    },
    Error {
        request_id: Option<String>,
        error: ProtocolError,
    },
}

impl Response {
    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::Error {
            request_id: None,
            error: ProtocolError {
                code: "invalid_request".to_owned(),
                message: truncate(message.into()),
            },
        }
    }

    fn error(request_id: String, error: &RunnerError) -> Self {
        Self::Error {
            request_id: Some(request_id),
            error: ProtocolError {
                code: error.code().to_owned(),
                message: truncate(error.to_string()),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Success {
    Loaded { plugin: ProjectPluginPin },
    Validated { hooks: Vec<LocalId> },
    HookResult { output: HookOutput },
    ShuttingDown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Error)]
pub enum RunnerError {
    #[error("manifest rejected: {0:?}")]
    ManifestRejected(Vec<ValidationIssue>),
    #[error("source is larger than {MAX_SOURCE_BYTES} bytes")]
    SourceTooLarge,
    #[error("source hash does not match the exact manifest pin")]
    SourceHashMismatch,
    #[error("invalid execution limits: {0}")]
    InvalidLimits(String),
    #[error("Lua memory control is unavailable")]
    MemoryControlUnavailable,
    #[error("Lua memory limit exhausted")]
    MemoryLimitExceeded,
    #[error("Lua instruction limit exhausted")]
    InstructionLimitExceeded,
    #[error("Lua wall-time limit exhausted")]
    TimeLimitExceeded,
    #[error("Lua execution failed: {0}")]
    LuaExecution(String),
    #[error("plugin exports are invalid: {0}")]
    InvalidExports(String),
    #[error("hook is not declared by the manifest: {0}")]
    HookNotDeclared(LocalId),
    #[error("hook output is malformed: {0}")]
    MalformedOutput(String),
    #[error("hook output exceeds the configured byte limit")]
    OutputLimitExceeded,
    #[error("no plugin is loaded")]
    NoPluginLoaded,
}

impl RunnerError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::ManifestRejected(_) => "manifest_rejected",
            Self::SourceTooLarge => "source_too_large",
            Self::SourceHashMismatch => "source_hash_mismatch",
            Self::InvalidLimits(_) => "invalid_limits",
            Self::MemoryControlUnavailable => "memory_control_unavailable",
            Self::MemoryLimitExceeded => "memory_limit_exceeded",
            Self::InstructionLimitExceeded => "instruction_limit_exceeded",
            Self::TimeLimitExceeded => "time_limit_exceeded",
            Self::LuaExecution(_) => "lua_execution_failed",
            Self::InvalidExports(_) => "invalid_exports",
            Self::HookNotDeclared(_) => "hook_not_declared",
            Self::MalformedOutput(_) => "malformed_output",
            Self::OutputLimitExceeded => "output_limit_exceeded",
            Self::NoPluginLoaded => "no_plugin_loaded",
        }
    }
}

pub struct Sandbox {
    lua: Lua,
    manifest: PluginManifest,
    exports: RegistryKey,
    readonly: RegistryKey,
    limits: ExecutionLimits,
}

impl Sandbox {
    pub fn load(
        manifest: PluginManifest,
        source: &str,
        limits: ExecutionLimits,
    ) -> Result<Self, RunnerError> {
        limits.validate()?;
        manifest
            .validate()
            .map_err(|errors| RunnerError::ManifestRejected(errors.issues))?;
        if source.len() > MAX_SOURCE_BYTES {
            return Err(RunnerError::SourceTooLarge);
        }
        if oreak_plugin_api::ContentHash::blake3(source.as_bytes()) != manifest.source_hash {
            return Err(RunnerError::SourceHashMismatch);
        }

        let libs = StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8;
        let lua = Lua::new_with(libs, LuaOptions::default()).map_err(map_lua_error)?;
        lua.set_memory_limit(limits.memory_bytes)
            .map_err(|_| RunnerError::MemoryControlUnavailable)?;

        let readonly_function: Function = lua
            .load(READONLY_HELPER)
            .set_name("=oreak-readonly")
            .set_mode(ChunkMode::Text)
            .eval()
            .map_err(map_lua_error)?;
        let readonly = lua
            .create_registry_value(readonly_function)
            .map_err(map_lua_error)?;
        let environment = build_safe_environment(&lua).map_err(map_lua_error)?;
        let exports: Table = run_with_budget(&lua, &limits, || {
            lua.load(source)
                .set_name("=plugin")
                .set_mode(ChunkMode::Text)
                .set_environment(environment)
                .eval()
        })?;
        validate_exports(&manifest, &exports)?;
        let exports = lua.create_registry_value(exports).map_err(map_lua_error)?;

        Ok(Self {
            lua,
            manifest,
            exports,
            readonly,
            limits,
        })
    }

    pub fn plugin_pin(&self) -> ProjectPluginPin {
        self.manifest.exact_pin()
    }

    pub fn validate(&self) -> Result<Vec<LocalId>, RunnerError> {
        self.manifest
            .validate()
            .map_err(|errors| RunnerError::ManifestRejected(errors.issues))?;
        let exports: Table = self
            .lua
            .registry_value(&self.exports)
            .map_err(map_lua_error)?;
        validate_exports(&self.manifest, &exports)?;
        Ok(self
            .manifest
            .hooks
            .iter()
            .map(|hook| hook.id.clone())
            .collect())
    }

    pub fn invoke_hook(&self, hook: &LocalId, input: &Value) -> Result<HookOutput, RunnerError> {
        let declaration = self
            .manifest
            .hooks
            .iter()
            .find(|declaration| declaration.id == *hook)
            .ok_or_else(|| RunnerError::HookNotDeclared(hook.clone()))?;
        let exports: Table = self
            .lua
            .registry_value(&self.exports)
            .map_err(map_lua_error)?;
        let function: Function = exports
            .raw_get(declaration.export.as_str())
            .map_err(map_lua_error)?;
        let input = self.lua.to_value(input).map_err(map_lua_error)?;
        let readonly: Function = self
            .lua
            .registry_value(&self.readonly)
            .map_err(map_lua_error)?;
        let value: LuaValue = run_with_budget(&self.lua, &self.limits, || {
            let input: LuaValue = readonly.call(input)?;
            function.call(input)
        })?;
        preflight_lua_output(&value)?;
        let output: HookOutput = self
            .lua
            .from_value(value)
            .map_err(|error| RunnerError::MalformedOutput(truncate(error.to_string())))?;
        output.validate().map_err(|errors| {
            RunnerError::MalformedOutput(truncate(format!("{:?}", errors.issues)))
        })?;
        validate_output_capabilities(&self.manifest, &output)?;
        let output_size = serde_json::to_vec(&output)
            .map_err(|error| RunnerError::MalformedOutput(error.to_string()))?
            .len();
        if output_size > self.limits.output_bytes {
            return Err(RunnerError::OutputLimitExceeded);
        }
        Ok(output)
    }
}

#[derive(Default)]
pub struct RunnerState {
    sandbox: Option<Sandbox>,
}

impl RunnerState {
    pub fn handle(&mut self, request: Request) -> (Response, bool) {
        let request_id = request.request_id().to_owned();
        let result = match request {
            Request::Load {
                manifest,
                source,
                limits,
                ..
            } => Sandbox::load(*manifest, &source, limits).map(|sandbox| {
                let plugin = sandbox.plugin_pin();
                self.sandbox = Some(sandbox);
                Success::Loaded { plugin }
            }),
            Request::Validate { .. } => self
                .sandbox
                .as_ref()
                .ok_or(RunnerError::NoPluginLoaded)
                .and_then(Sandbox::validate)
                .map(|hooks| Success::Validated { hooks }),
            Request::InvokeHook { hook, input, .. } => self
                .sandbox
                .as_ref()
                .ok_or(RunnerError::NoPluginLoaded)
                .and_then(|sandbox| sandbox.invoke_hook(&hook, &input))
                .map(|output| Success::HookResult { output }),
            Request::Shutdown { .. } => {
                return (
                    Response::Ok {
                        request_id: Some(request_id),
                        result: Success::ShuttingDown,
                    },
                    true,
                );
            }
        };

        match result {
            Ok(result) => (
                Response::Ok {
                    request_id: Some(request_id),
                    result,
                },
                false,
            ),
            Err(error) => (Response::error(request_id, &error), false),
        }
    }
}

#[derive(Clone, Copy)]
enum BudgetFailure {
    Instructions,
    Time,
}

fn run_with_budget<T>(
    lua: &Lua,
    limits: &ExecutionLimits,
    operation: impl FnOnce() -> mlua::Result<T>,
) -> Result<T, RunnerError> {
    let consumed = Rc::new(Cell::new(0_u64));
    let failure = Rc::new(Cell::new(None));
    let deadline = Instant::now() + Duration::from_millis(limits.wall_time_ms);
    let instruction_limit = limits.instruction_limit;
    let hook_consumed = Rc::clone(&consumed);
    let hook_failure = Rc::clone(&failure);
    lua.set_hook(
        HookTriggers::new().every_nth_instruction(HOOK_INTERVAL),
        move |_, _| {
            let next = hook_consumed.get().saturating_add(u64::from(HOOK_INTERVAL));
            hook_consumed.set(next);
            if next >= instruction_limit {
                hook_failure.set(Some(BudgetFailure::Instructions));
                return Err(LuaError::runtime("instruction budget exhausted"));
            }
            if Instant::now() >= deadline {
                hook_failure.set(Some(BudgetFailure::Time));
                return Err(LuaError::runtime("wall-time budget exhausted"));
            }
            Ok(VmState::Continue)
        },
    )
    .map_err(map_lua_error)?;

    let result = operation();
    lua.remove_hook();

    match failure.get() {
        Some(BudgetFailure::Instructions) => Err(RunnerError::InstructionLimitExceeded),
        Some(BudgetFailure::Time) => Err(RunnerError::TimeLimitExceeded),
        None if Instant::now() >= deadline => Err(RunnerError::TimeLimitExceeded),
        None => result.map_err(map_lua_error),
    }
}

fn build_safe_environment(lua: &Lua) -> mlua::Result<Table> {
    let globals = lua.globals();
    let environment = lua.create_table()?;
    for name in [
        "assert", "error", "ipairs", "next", "pairs", "pcall", "select", "tonumber", "tostring",
        "type", "xpcall",
    ] {
        environment.raw_set(name, globals.raw_get::<LuaValue>(name)?)?;
    }
    environment.raw_set("_VERSION", "Lua 5.4")?;
    let string_library: Table = globals.raw_get("string")?;
    string_library.raw_set("dump", LuaValue::Nil)?;
    let math_library: Table = globals.raw_get("math")?;
    math_library.raw_set("random", LuaValue::Nil)?;
    math_library.raw_set("randomseed", LuaValue::Nil)?;
    environment.raw_set("table", clone_library(lua, globals.raw_get("table")?, &[])?)?;
    environment.raw_set("string", clone_library(lua, string_library, &[])?)?;
    environment.raw_set("math", clone_library(lua, math_library, &[])?)?;
    environment.raw_set("utf8", clone_library(lua, globals.raw_get("utf8")?, &[])?)?;
    Ok(environment)
}

fn clone_library(lua: &Lua, source: Table, excluded: &[&str]) -> mlua::Result<Table> {
    let target = lua.create_table()?;
    for pair in source.pairs::<LuaValue, LuaValue>() {
        let (key, value) = pair?;
        let is_excluded = match &key {
            LuaValue::String(key) => excluded
                .iter()
                .any(|excluded| key.as_bytes().as_ref() == excluded.as_bytes()),
            _ => false,
        };
        if !is_excluded {
            target.raw_set(key, value)?;
        }
    }
    Ok(target)
}

fn validate_exports(manifest: &PluginManifest, exports: &Table) -> Result<(), RunnerError> {
    let declared: BTreeSet<&str> = manifest
        .hooks
        .iter()
        .map(|hook| hook.export.as_str())
        .collect();

    for hook in &manifest.hooks {
        match exports
            .raw_get::<LuaValue>(hook.export.as_str())
            .map_err(map_lua_error)?
        {
            LuaValue::Function(_) => {}
            _ => {
                return Err(RunnerError::InvalidExports(format!(
                    "export '{}' is missing or is not a function",
                    hook.export
                )));
            }
        }
    }

    for pair in exports.clone().pairs::<LuaValue, LuaValue>() {
        let (key, value) = pair.map_err(map_lua_error)?;
        let LuaValue::String(key) = key else {
            return Err(RunnerError::InvalidExports(
                "export keys must be strings".to_owned(),
            ));
        };
        let key = key
            .to_str()
            .map_err(|_| RunnerError::InvalidExports("export keys must be UTF-8".to_owned()))?;
        if !declared.contains(key.as_ref()) {
            return Err(RunnerError::InvalidExports(format!(
                "undeclared export '{key}'"
            )));
        }
        if !matches!(value, LuaValue::Function(_)) {
            return Err(RunnerError::InvalidExports(format!(
                "export '{key}' is not a function"
            )));
        }
    }
    Ok(())
}

fn preflight_lua_output(root: &LuaValue) -> Result<(), RunnerError> {
    let mut stack = vec![(root.clone(), 0_usize)];
    let mut tables = HashSet::<*const c_void>::new();
    let mut nodes = 0_usize;

    while let Some((value, depth)) = stack.pop() {
        nodes = nodes.saturating_add(1);
        if nodes > MAX_LUA_OUTPUT_NODES || depth > MAX_LUA_OUTPUT_DEPTH {
            return Err(RunnerError::MalformedOutput(
                "Lua output exceeds the depth or node-count limit".to_owned(),
            ));
        }
        match value {
            LuaValue::Nil
            | LuaValue::Boolean(_)
            | LuaValue::Integer(_)
            | LuaValue::Number(_)
            | LuaValue::String(_) => {}
            LuaValue::Table(table) => {
                if !tables.insert(table.to_pointer()) {
                    return Err(RunnerError::MalformedOutput(
                        "Lua output contains a recursive or aliased table".to_owned(),
                    ));
                }
                for pair in table.pairs::<LuaValue, LuaValue>() {
                    let (key, value) = pair.map_err(map_lua_error)?;
                    stack.push((key, depth + 1));
                    stack.push((value, depth + 1));
                }
            }
            _ => {
                return Err(RunnerError::MalformedOutput(
                    "Lua output contains a non-JSON value".to_owned(),
                ));
            }
        }
    }
    Ok(())
}

fn validate_output_capabilities(
    manifest: &PluginManifest,
    output: &HookOutput,
) -> Result<(), RunnerError> {
    if !output.proposals.is_empty() && !manifest.capabilities.contains(&Capability::ProposeCommands)
    {
        return Err(RunnerError::MalformedOutput(
            "command proposals require the propose_commands capability".to_owned(),
        ));
    }
    if !output.render_primitives.is_empty() && !manifest.capabilities.contains(&Capability::Render)
    {
        return Err(RunnerError::MalformedOutput(
            "render primitives require the render capability".to_owned(),
        ));
    }
    if !output.generated_artifacts.is_empty()
        && !manifest.capabilities.contains(&Capability::Generate)
    {
        return Err(RunnerError::MalformedOutput(
            "generated artifacts require the generate capability".to_owned(),
        ));
    }
    Ok(())
}

fn map_lua_error(error: LuaError) -> RunnerError {
    match error {
        LuaError::MemoryError(_) => RunnerError::MemoryLimitExceeded,
        LuaError::MemoryControlNotAvailable => RunnerError::MemoryControlUnavailable,
        other => RunnerError::LuaExecution(truncate(other.to_string())),
    }
}

fn truncate(mut value: String) -> String {
    if value.len() <= MAX_ERROR_BYTES {
        return value;
    }
    let mut boundary = MAX_ERROR_BYTES;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value.truncate(boundary);
    value.push_str("...[truncated]");
    value
}

const READONLY_HELPER: &str = r#"
local error = error
local next = next
local pairs = pairs
local setmetatable = setmetatable
local type = type

local function readonly(value, seen)
    if type(value) ~= "table" then
        return value
    end
    seen = seen or {}
    if seen[value] then
        return seen[value]
    end

    local backing = {}
    local proxy = {}
    seen[value] = proxy
    for key, child in pairs(value) do
        backing[readonly(key, seen)] = readonly(child, seen)
    end

    return setmetatable(proxy, {
        __index = backing,
        __newindex = function()
            error("hook input is immutable", 2)
        end,
        __len = function()
            return #backing
        end,
        __pairs = function()
            return next, backing, nil
        end,
        __metatable = false,
    })
end

return readonly
"#;

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use oreak_plugin_api::{
        Capability, ContentHash, HookDeclaration, HookKind, PluginId, PluginManifest,
        ValidatorContribution, ValidatorScope, Version,
    };
    use serde_json::json;

    use super::*;

    fn id(value: &str) -> LocalId {
        LocalId::new(value).unwrap()
    }

    fn validator_manifest(source: &str) -> PluginManifest {
        let mut manifest = PluginManifest::new(
            PluginId::new("com.example.runner_test").unwrap(),
            Version::new(1, 0, 0),
            Version::new(1, 0, 0),
            ContentHash::blake3(source.as_bytes()),
        );
        manifest.capabilities = BTreeSet::from([Capability::Validate]);
        manifest.hooks.push(HookDeclaration {
            id: id("validate_level"),
            kind: HookKind::Validate,
            export: id("validate_level"),
        });
        manifest.validators.push(ValidatorContribution {
            id: id("validator"),
            hook: id("validate_level"),
            scope: ValidatorScope::Level,
        });
        manifest
    }

    #[test]
    fn rejects_invalid_manifest() {
        let source = "return { validate_level = function(_) return {} end }";
        let mut manifest = validator_manifest(source);
        manifest.capabilities.clear();

        assert!(matches!(
            Sandbox::load(manifest, source, ExecutionLimits::default()),
            Err(RunnerError::ManifestRejected(_))
        ));
    }

    #[test]
    fn forbidden_globals_are_absent() {
        let source = r#"
            if io ~= nil or os ~= nil or package ~= nil or debug ~= nil
                or require ~= nil or dofile ~= nil or loadfile ~= nil
                or load ~= nil or collectgarbage ~= nil or print ~= nil
                or rawget ~= nil or rawset ~= nil or setmetatable ~= nil
                or getmetatable ~= nil or coroutine ~= nil then
                error("forbidden global exposed")
            end
            if math.random ~= nil or math.randomseed ~= nil or string.dump ~= nil
                or ("").dump ~= nil then
                error("unsafe library member exposed")
            end
            return { validate_level = function(_) return {} end }
        "#;
        let sandbox = Sandbox::load(
            validator_manifest(source),
            source,
            ExecutionLimits::default(),
        )
        .unwrap();

        sandbox
            .invoke_hook(&id("validate_level"), &json!({}))
            .unwrap();
    }

    #[test]
    fn instruction_exhaustion_stops_an_infinite_hook() {
        let source = r#"
            return {
                validate_level = function(_)
                    local value = 0
                    while true do value = value + 1 end
                end
            }
        "#;
        let limits = ExecutionLimits {
            instruction_limit: 5_000,
            wall_time_ms: 1_000,
            ..ExecutionLimits::default()
        };
        let sandbox = Sandbox::load(validator_manifest(source), source, limits).unwrap();

        assert!(matches!(
            sandbox.invoke_hook(&id("validate_level"), &json!({})),
            Err(RunnerError::InstructionLimitExceeded)
        ));
    }

    #[test]
    fn wall_time_exhaustion_is_detected() {
        let source = r#"
            return {
                validate_level = function(_)
                    local value = 0
                    while true do value = value + 1 end
                end
            }
        "#;
        let limits = ExecutionLimits {
            instruction_limit: MAX_INSTRUCTIONS,
            wall_time_ms: 1,
            ..ExecutionLimits::default()
        };
        let sandbox = Sandbox::load(validator_manifest(source), source, limits).unwrap();

        assert!(matches!(
            sandbox.invoke_hook(&id("validate_level"), &json!({})),
            Err(RunnerError::TimeLimitExceeded)
        ));
    }

    #[test]
    fn malformed_hook_output_is_rejected() {
        let source = r#"
            return {
                validate_level = function(_)
                    return { diagnostics = "not-an-array" }
                end
            }
        "#;
        let sandbox = Sandbox::load(
            validator_manifest(source),
            source,
            ExecutionLimits::default(),
        )
        .unwrap();

        assert!(matches!(
            sandbox.invoke_hook(&id("validate_level"), &json!({})),
            Err(RunnerError::MalformedOutput(_))
        ));
    }

    #[test]
    fn valid_hook_receives_immutable_input_and_returns_proposals() {
        let source = r#"
            return {
                validate_level = function(input)
                    local mutable = pcall(function()
                        input.project.name = "changed"
                    end)
                    if mutable then error("input was mutable") end
                    return {
                        proposals = {
                            {
                                id = "proposal_1",
                                command = "set_plugin_data",
                                payload = { entity_id = input.entity_id, enabled = true }
                            }
                        },
                        diagnostics = {
                            {
                                severity = "info",
                                code = "checked",
                                message = "input remained immutable"
                            }
                        }
                    }
                end
            }
        "#;
        let mut manifest = validator_manifest(source);
        manifest.capabilities.insert(Capability::ProposeCommands);
        let sandbox = Sandbox::load(manifest, source, ExecutionLimits::default()).unwrap();

        let output = sandbox
            .invoke_hook(
                &id("validate_level"),
                &json!({"entity_id": "entity-7", "project": {"name": "demo"}}),
            )
            .unwrap();

        assert_eq!(output.proposals.len(), 1);
        assert_eq!(output.proposals[0].payload["entity_id"], "entity-7");
        assert_eq!(output.diagnostics.len(), 1);
    }

    #[test]
    fn protocol_load_validate_invoke_and_shutdown_are_typed() {
        let source = "return { validate_level = function(_) return {} end }";
        let mut state = RunnerState::default();
        let (loaded, stop) = state.handle(Request::Load {
            request_id: "1".to_owned(),
            manifest: Box::new(validator_manifest(source)),
            source: source.to_owned(),
            limits: ExecutionLimits::default(),
        });
        assert!(matches!(loaded, Response::Ok { .. }));
        assert!(!stop);

        let (validated, _) = state.handle(Request::Validate {
            request_id: "2".to_owned(),
        });
        assert!(matches!(validated, Response::Ok { .. }));

        let (invoked, _) = state.handle(Request::InvokeHook {
            request_id: "3".to_owned(),
            hook: id("validate_level"),
            input: json!({}),
        });
        assert!(matches!(invoked, Response::Ok { .. }));

        let (_, stop) = state.handle(Request::Shutdown {
            request_id: "4".to_owned(),
        });
        assert!(stop);
    }
}

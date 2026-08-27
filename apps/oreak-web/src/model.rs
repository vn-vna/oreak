use std::collections::{BTreeMap, BTreeSet};

use oreak_core::{
    ActorId, ApplyOutcome, BlameEntry, CellKind, CommandEnvelope, CommandMetadata, GridPoint,
    HistoryEvent, LevelCommand, LevelSnapshot, LevelTarget, LevelTimeline, TimelineError,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Select,
    Map,
    Brush,
    Sandbox,
}

#[cfg(target_arch = "wasm32")]
impl Mode {
    pub const ALL: [Self; 4] = [Self::Select, Self::Map, Self::Brush, Self::Sandbox];

    pub const fn key(self) -> char {
        match self {
            Self::Select => 'Q',
            Self::Map => 'W',
            Self::Brush => 'B',
            Self::Sandbox => 'P',
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Select => "Select",
            Self::Map => "Map",
            Self::Brush => "Brush",
            Self::Sandbox => "Sandbox",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::Select => "Inspect cells and collaborative provenance",
            Self::Map => "Paint logical floor and wall cells",
            Self::Brush => "Parity gated: source brush semantics are not available yet",
            Self::Sandbox => "Parity gated: source sandbox behavior is not available yet",
        }
    }

    pub const fn is_parity_gated(self) -> bool {
        matches!(self, Self::Brush | Self::Sandbox)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShortcutScope {
    Workspace,
    TextEntry,
    Palette,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shortcut {
    SelectMode(Mode),
    Undo,
    TogglePalette,
    CloseOverlay,
    PaletteSubmit,
}

pub fn resolve_shortcut(
    key: &str,
    command_modifier: bool,
    shift: bool,
    scope: ShortcutScope,
) -> Option<Shortcut> {
    let key = key.to_ascii_lowercase();

    if command_modifier && key == "k" {
        return Some(Shortcut::TogglePalette);
    }
    if key == "escape" {
        return Some(Shortcut::CloseOverlay);
    }
    if scope == ShortcutScope::Palette && key == "enter" {
        return Some(Shortcut::PaletteSubmit);
    }
    if scope != ShortcutScope::Workspace {
        return None;
    }
    if command_modifier && !shift && key == "z" {
        return Some(Shortcut::Undo);
    }
    if command_modifier {
        return None;
    }

    match key.as_str() {
        "q" => Some(Shortcut::SelectMode(Mode::Select)),
        "w" => Some(Shortcut::SelectMode(Mode::Map)),
        "b" => Some(Shortcut::SelectMode(Mode::Brush)),
        "p" => Some(Shortcut::SelectMode(Mode::Sandbox)),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelChange {
    Applied { sequence: u64 },
    NoChange,
}

#[derive(Debug)]
pub struct EditorModel {
    timeline: LevelTimeline,
    actor: ActorId,
    command_prefix: String,
    command_nonce: u64,
}

#[derive(Debug)]
pub struct HydratedHistory {
    pub seen_commands: BTreeSet<String>,
    pub cell_blame: BTreeMap<GridPoint, BlameEntry>,
}

pub fn hydrate_history(
    expected_snapshot: &LevelSnapshot,
    events: &[HistoryEvent],
) -> Result<HydratedHistory, String> {
    let size = expected_snapshot.size();
    let initial = LevelSnapshot::new(size.width(), size.height())
        .map_err(|error| format!("history base snapshot is invalid: {error}"))?;
    let mut timeline = LevelTimeline::new(initial)
        .map_err(|error| format!("history base timeline is invalid: {error}"))?;

    for (index, event) in events.iter().enumerate() {
        let expected_sequence = index as u64 + 1;
        if event.sequence != expected_sequence {
            return Err(format!(
                "history sequence {} appeared where {expected_sequence} was required",
                event.sequence
            ));
        }
        let reconstructed = if event.reverts_sequence.is_some() {
            timeline
                .undo_latest(event.metadata.clone())
                .map_err(|error| format!("history undo #{} is invalid: {error}", event.sequence))?
        } else {
            match timeline
                .apply(CommandEnvelope::new(
                    event.metadata.clone(),
                    event.command.clone(),
                ))
                .map_err(|error| format!("history event #{} is invalid: {error}", event.sequence))?
            {
                ApplyOutcome::Applied(reconstructed) => reconstructed,
                ApplyOutcome::NoChange { .. } => {
                    return Err(format!(
                        "history event #{} reconstructed as a no-op",
                        event.sequence
                    ));
                }
            }
        };
        if reconstructed != *event {
            return Err(format!(
                "history event #{} does not match deterministic core semantics",
                event.sequence
            ));
        }
    }

    if timeline.snapshot() != expected_snapshot {
        return Err("hydrated history does not produce the subscribed snapshot".to_owned());
    }

    let cell_targets: BTreeSet<_> = events
        .iter()
        .flat_map(|event| event.changes.iter())
        .filter_map(|change| match &change.target {
            LevelTarget::Cell(point) => Some(*point),
            _ => None,
        })
        .collect();
    let cell_blame = cell_targets
        .into_iter()
        .filter_map(|point| timeline.blame_cell(point).map(|blame| (point, blame)))
        .collect();
    let seen_commands = events
        .iter()
        .map(|event| event.metadata.id.to_string())
        .collect();

    Ok(HydratedHistory {
        seen_commands,
        cell_blame,
    })
}

impl EditorModel {
    pub fn blank(actor_id: &str, command_session_prefix: &str) -> Self {
        Self::from_snapshot(
            LevelSnapshot::new(8, 8).expect("the fixed editor grid is valid"),
            actor_id,
            command_session_prefix,
        )
    }

    pub fn from_snapshot(
        snapshot: LevelSnapshot,
        actor_id: &str,
        command_session_prefix: &str,
    ) -> Self {
        Self {
            timeline: LevelTimeline::new(snapshot).expect("persisted snapshots are validated"),
            actor: ActorId::new(actor_id),
            command_prefix: format!("web-{command_session_prefix}"),
            command_nonce: 1,
        }
    }

    pub const fn timeline(&self) -> &LevelTimeline {
        &self.timeline
    }

    pub const fn actor(&self) -> &ActorId {
        &self.actor
    }

    pub fn replace_snapshot(&mut self, snapshot: LevelSnapshot) -> Result<(), TimelineError> {
        self.timeline = LevelTimeline::new(snapshot)?;
        Ok(())
    }

    pub fn cell(&self, point: GridPoint) -> Result<CellKind, TimelineError> {
        self.timeline
            .snapshot()
            .cell(point)
            .map_err(TimelineError::from)
    }

    #[cfg(test)]
    pub fn toggle_cell(
        &mut self,
        point: GridPoint,
        occurred_at_ms: i64,
    ) -> Result<ModelChange, TimelineError> {
        let kind = match self.cell(point)? {
            CellKind::Floor => CellKind::Wall,
            CellKind::Wall => CellKind::Floor,
        };
        self.set_cell(point, kind, occurred_at_ms)
    }

    pub fn set_cell(
        &mut self,
        point: GridPoint,
        kind: CellKind,
        occurred_at_ms: i64,
    ) -> Result<ModelChange, TimelineError> {
        let envelope = self.prepare_set_cell(point, kind, occurred_at_ms);
        self.apply_envelope(envelope)
    }

    pub fn prepare_set_cell(
        &mut self,
        point: GridPoint,
        kind: CellKind,
        occurred_at_ms: i64,
    ) -> CommandEnvelope {
        CommandEnvelope::new(
            self.next_metadata("edit", occurred_at_ms),
            LevelCommand::SetCell { point, kind },
        )
    }

    pub fn apply_envelope(
        &mut self,
        envelope: CommandEnvelope,
    ) -> Result<ModelChange, TimelineError> {
        let outcome = self.timeline.apply(envelope)?;

        Ok(match outcome {
            ApplyOutcome::Applied(event) => ModelChange::Applied {
                sequence: event.sequence,
            },
            ApplyOutcome::NoChange { .. } => ModelChange::NoChange,
        })
    }

    pub fn undo(&mut self, occurred_at_ms: i64) -> Result<HistoryEvent, TimelineError> {
        let metadata = self.prepare_undo(occurred_at_ms);
        self.timeline.undo_latest(metadata)
    }

    pub fn prepare_undo(&mut self, occurred_at_ms: i64) -> CommandMetadata {
        self.next_metadata("undo", occurred_at_ms)
    }

    pub fn apply_server_event(
        &mut self,
        event: &HistoryEvent,
    ) -> Result<ModelChange, TimelineError> {
        self.apply_envelope(CommandEnvelope::new(
            event.metadata.clone(),
            event.command.clone(),
        ))
    }

    fn next_metadata(&mut self, operation: &str, occurred_at_ms: i64) -> CommandMetadata {
        let id = format!(
            "{}-{operation}-{:016x}",
            self.command_prefix, self.command_nonce
        );
        self.command_nonce += 1;
        CommandMetadata::new(id, self.actor.clone(), occurred_at_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_shortcuts_select_all_modes_and_undo() {
        let scope = ShortcutScope::Workspace;
        assert_eq!(
            resolve_shortcut("q", false, false, scope),
            Some(Shortcut::SelectMode(Mode::Select))
        );
        assert_eq!(
            resolve_shortcut("W", false, false, scope),
            Some(Shortcut::SelectMode(Mode::Map))
        );
        assert_eq!(
            resolve_shortcut("b", false, false, scope),
            Some(Shortcut::SelectMode(Mode::Brush))
        );
        assert_eq!(
            resolve_shortcut("p", false, false, scope),
            Some(Shortcut::SelectMode(Mode::Sandbox))
        );
        assert_eq!(
            resolve_shortcut("z", true, false, scope),
            Some(Shortcut::Undo)
        );
    }

    #[test]
    fn text_entry_scope_does_not_steal_editor_keys() {
        let scope = ShortcutScope::TextEntry;
        assert_eq!(resolve_shortcut("q", false, false, scope), None);
        assert_eq!(resolve_shortcut("z", true, false, scope), None);
        assert_eq!(
            resolve_shortcut("k", true, false, scope),
            Some(Shortcut::TogglePalette)
        );
        assert_eq!(
            resolve_shortcut("Escape", false, false, scope),
            Some(Shortcut::CloseOverlay)
        );
    }

    #[test]
    fn palette_scope_submits_without_enabling_workspace_shortcuts() {
        let scope = ShortcutScope::Palette;
        assert_eq!(
            resolve_shortcut("Enter", false, false, scope),
            Some(Shortcut::PaletteSubmit)
        );
        assert_eq!(resolve_shortcut("b", false, false, scope), None);
    }

    #[test]
    fn reducer_uses_core_for_toggle_blame_and_user_scoped_undo() {
        let point = GridPoint::new(3, 4);
        let mut model = EditorModel::blank("user-123", "test-session");

        assert_eq!(
            model.toggle_cell(point, 1_000).unwrap(),
            ModelChange::Applied { sequence: 1 }
        );
        assert_eq!(model.cell(point).unwrap(), CellKind::Wall);
        let blame = model.timeline().blame_cell(point).unwrap();
        assert_eq!(blame.actor.as_str(), "user-123");
        assert_eq!(blame.occurred_at_ms, 1_000);

        let undo = model.undo(2_000).unwrap();
        assert_eq!(undo.sequence, 2);
        assert_eq!(undo.reverts_sequence, Some(1));
        assert_eq!(model.cell(point).unwrap(), CellKind::Floor);
    }

    #[test]
    fn command_ids_are_session_unique_and_survive_snapshot_replacement() {
        let point = GridPoint::new(0, 0);
        let mut first = EditorModel::blank("same-user", "session-a");
        let mut second = EditorModel::blank("same-user", "session-b");

        let first_command = first.prepare_set_cell(point, CellKind::Wall, 100);
        let second_command = second.prepare_set_cell(point, CellKind::Wall, 100);
        assert_eq!(first.actor().as_str(), "same-user");
        assert_ne!(first_command.metadata.id, second_command.metadata.id);
        assert_eq!(first_command.metadata.actor, second_command.metadata.actor);

        first
            .replace_snapshot(LevelSnapshot::new(8, 8).unwrap())
            .unwrap();
        let after_resync = first.prepare_set_cell(point, CellKind::Wall, 101);
        assert_ne!(first_command.metadata.id, after_resync.metadata.id);
        assert_eq!(first_command.metadata.actor, after_resync.metadata.actor);
    }

    #[test]
    fn authoritative_event_is_revalidated_by_the_shared_core() {
        let point = GridPoint::new(6, 2);
        let mut source = EditorModel::blank("remote-user", "remote-session");
        source.set_cell(point, CellKind::Wall, 500).unwrap();
        let event = source.timeline().events()[0].clone();

        let mut replica = EditorModel::blank("local-user", "local-session");
        assert_eq!(
            replica.apply_server_event(&event).unwrap(),
            ModelChange::Applied { sequence: 1 }
        );
        assert_eq!(replica.cell(point).unwrap(), CellKind::Wall);
    }

    #[test]
    fn hydrated_history_reconstructs_reverted_cell_provenance() {
        let reverted = GridPoint::new(1, 1);
        let active = GridPoint::new(2, 2);
        let mut source = LevelTimeline::new(LevelSnapshot::new(8, 8).unwrap()).unwrap();
        source
            .apply(CommandEnvelope::new(
                CommandMetadata::new("alice-wall", "alice", 100),
                LevelCommand::SetCell {
                    point: reverted,
                    kind: CellKind::Wall,
                },
            ))
            .unwrap();
        source
            .apply(CommandEnvelope::new(
                CommandMetadata::new("bob-wall", "bob", 110),
                LevelCommand::SetCell {
                    point: active,
                    kind: CellKind::Wall,
                },
            ))
            .unwrap();
        source
            .undo_latest(CommandMetadata::new("alice-undo", "alice", 120))
            .unwrap();

        let hydration = hydrate_history(source.snapshot(), source.events()).unwrap();
        let reverted_blame = hydration.cell_blame.get(&reverted).unwrap();
        assert_eq!(reverted_blame.sequence, 3);
        assert_eq!(reverted_blame.command_id.as_str(), "alice-undo");
        assert_eq!(reverted_blame.reverts_sequence, Some(1));
        let active_blame = hydration.cell_blame.get(&active).unwrap();
        assert_eq!(active_blame.sequence, 2);
        assert_eq!(active_blame.actor.as_str(), "bob");
        assert_eq!(hydration.seen_commands.len(), 3);
    }
}

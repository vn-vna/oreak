//! Preview-only Distribution workspace. Writes use ordinary guarded, atomic commands.
use super::*;
use oreak_core::{
    DISABLED_COLLECT_COLOR_INDEX, DistributionLayerSelection, DistributionPlan,
    DistributionPoolSelection, DistributionRequest, MAX_POOL_DISTRIBUTION_GROUP_NAME_CHARS,
    MAX_POOL_DISTRIBUTION_GROUPS, PoolDistributionGroup, distribution_plan,
};

use oreak_core::distribution::{MAX_DISTRIBUTION_WEIGHT, pool_all_group_counts};

type GroupTotals = BTreeMap<u32, (u64, u64)>;

#[derive(Default)]
pub(super) struct DistributionState {
    preview: Option<DistributionPreview>,
    division: Option<PoolDivision>,
    pub(super) hover: Option<(EntityId, u32)>,
}
struct DistributionPreview {
    target: ProjectLevelTarget,
    selected: Vec<EntityId>,
    request: DistributionRequest,
    expected: Vec<PlaceableEntity>,
    plan: Result<DistributionPlan, String>,
    group_counts: BTreeMap<EntityId, Result<GroupTotals, String>>,
    invalid_weights: BTreeSet<(EntityId, usize)>,
    pending_command: Option<String>,
    apply_error: Option<String>,
    quick: bool,
}
impl DistributionPreview {
    fn normalized_request(&self) -> DistributionRequest {
        let mut request = self.request.clone();
        request.pools.retain(|p| !p.group_ids.is_empty());
        request
    }
}

struct PoolDivision {
    target: ProjectLevelTarget,
    expected: PlaceableEntity,
    groups: Vec<PoolDistributionGroup>,
    active: u32,
    name: String,
    freehand: bool,
    selection: BTreeSet<BlindPixel>,
    gesture: Option<RegionGesture>,
    canvas: NodeRef,
}
struct RegionGesture {
    pointer: i32,
    points: Vec<(f64, f64)>,
}
pub(crate) enum DistributionMsg {
    Preview(bool),
    Cancel,
    Refresh,
    Apply,
    Group(EntityId, u32, bool),
    Layer(EntityId, usize, bool),
    Weight(EntityId, usize, String),
    Equalize(u16),
    Hover(Option<(EntityId, u32)>),
    Divide,
    CloseDivision,
    SaveGroups,
    DivisionTool(bool),
    ActiveGroup(u32),
    NewName(String),
    AddGroup,
    DeleteGroup(u32),
    RenameGroup(u32, String),
    Assign,
    ClearRegion,
    RegionDown(PointerEvent),
    RegionMove(PointerEvent),
    RegionUp(PointerEvent),
    RegionCancel(PointerEvent),
}

fn group_contains(groups: &[PoolDistributionGroup], group_id: u32, pixel: BlindPixel) -> bool {
    if group_id == 0 {
        !groups
            .iter()
            .any(|g| g.pixels.binary_search(&pixel).is_ok())
    } else {
        groups
            .iter()
            .find(|g| g.id == group_id)
            .is_some_and(|g| g.pixels.binary_search(&pixel).is_ok())
    }
}

// Pixel y is lower-left inside each tile; shape-cell y follows the board's top-left grid.
fn pool_pixels(
    entity: &PlaceableEntity,
) -> impl Iterator<Item = (BlindPixel, (f64, f64, f64, f64), u8)> + '_ {
    let blind = entity.as_blind();
    entity.shape().occupied_cells().flat_map(move |cell| {
        let resolution = blind.map_or(1, |b| b.pixels_per_cell());
        let tile = blind.and_then(|b| b.tile_for_cell(entity.shape(), cell));
        let geometry = pool_tile_geometry(
            shape_boundary_edges(entity.shape(), cell),
            CELL_SIZE,
            BLIND_INSET,
        );
        let w = geometry.width / f64::from(resolution);
        let h = geometry.height / f64::from(resolution);
        (0..resolution).flat_map(move |y| {
            (0..resolution).map(move |x| {
                let pixel = BlindPixel::new(
                    u16::from(cell.x) * u16::from(resolution) + u16::from(x),
                    u16::from(cell.y) * u16::from(resolution) + u16::from(y),
                );
                (
                    pixel,
                    (
                        f64::from(cell.x) * CELL_SIZE + geometry.x + f64::from(x) * w,
                        f64::from(cell.y) * CELL_SIZE
                            + geometry.y
                            + f64::from(resolution - 1 - y) * h,
                        w,
                        h,
                    ),
                    tile.and_then(|t| t.color(x, y)).unwrap_or(0),
                )
            })
        })
    })
}

fn point_in_polygon(point: (f64, f64), path: &[(f64, f64)]) -> bool {
    if path.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut previous = path[path.len() - 1];
    for &next in path {
        if (next.1 > point.1) != (previous.1 > point.1)
            && point.0 < (previous.0 - next.0) * (point.1 - next.1) / (previous.1 - next.1) + next.0
        {
            inside = !inside;
        }
        previous = next;
    }
    inside
}

impl PoolDivision {
    fn scale(&self) -> f64 {
        600.0
            / (f64::from(
                self.expected
                    .shape()
                    .width()
                    .max(self.expected.shape().height()),
            ) * CELL_SIZE)
    }
    fn point(&self, event: &PointerEvent) -> Option<(f64, f64)> {
        let canvas = self.canvas.cast::<HtmlCanvasElement>()?;
        let rect = canvas.get_bounding_client_rect();
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            return None;
        }
        Some((
            (f64::from(event.client_x()) - rect.left()) * 640.0 / rect.width() / self.scale()
                - 20.0 / self.scale(),
            (f64::from(event.client_y()) - rect.top()) * 640.0 / rect.height() / self.scale()
                - 20.0 / self.scale(),
        ))
    }
    fn assign(&mut self) {
        for group in &mut self.groups {
            group.pixels.retain(|p| !self.selection.contains(p));
            if group.id == self.active {
                group.pixels.extend(self.selection.iter().copied());
                group.pixels.sort_unstable();
                group.pixels.dedup();
            }
        }
        self.selection.clear();
    }
    fn selected_region(&self) -> BTreeSet<BlindPixel> {
        let Some(gesture) = &self.gesture else {
            return self.selection.clone();
        };
        let start = gesture.points[0];
        let end = *gesture.points.last().unwrap_or(&start);
        pool_pixels(&self.expected)
            .filter_map(|(pixel, (x, y, w, h), _)| {
                let click = (start.0 - end.0).hypot(start.1 - end.1) < 0.1;
                let selected = if click {
                    start.0 >= x && start.0 < x + w && start.1 >= y && start.1 < y + h
                } else if self.freehand {
                    point_in_polygon((x + w / 2.0, y + h / 2.0), &gesture.points)
                } else {
                    // Include touched pixels, allowing subpixel/one-pixel rectangles at every zoom.
                    x + w > start.0.min(end.0)
                        && x < start.0.max(end.0)
                        && y + h > start.1.min(end.1)
                        && y < start.1.max(end.1)
                };
                selected.then_some(pixel)
            })
            .collect()
    }
}

impl App {
    pub(super) fn distribution_modal_open(&self) -> bool {
        self.distribution.division.is_some()
    }

    pub(super) fn distribution_command_finished(
        &mut self,
        command_id: &str,
        error: Option<&String>,
    ) {
        let Some(preview) = &mut self.distribution.preview else {
            return;
        };
        if preview.pending_command.as_deref() != Some(command_id) {
            return;
        }
        if let Some(error) = error {
            preview.pending_command = None;
            preview.apply_error =
                Some(format!("Apply rejected: {error}. Refresh before retrying."));
        } else {
            self.distribution.preview = None;
            self.distribution.hover = None;
        }
    }

    fn distribution_gate(&self) -> Option<String> {
        if !self.can_edit_timeline {
            Some("Editing permission is required.".into())
        } else if !matches!(self.rpc_state, RpcState::Online) || self.rpc.is_none() {
            Some("Reconnect before applying changes.".into())
        } else if !self.pending_commands.is_empty() {
            Some("Wait for pending changes to finish.".into())
        } else {
            None
        }
    }
    fn capture_distribution_guard(&mut self) {
        let Some(preview) = &mut self.distribution.preview else {
            return;
        };
        preview.invalid_weights.retain(|(id, index)| {
            preview
                .request
                .layers
                .iter()
                .any(|l| l.entity_id == *id && l.layer_index == *index)
        });
        let ids: BTreeSet<_> = preview
            .request
            .pools
            .iter()
            .filter(|p| !p.group_ids.is_empty())
            .map(|p| p.entity_id.clone())
            .chain(preview.request.layers.iter().map(|l| l.entity_id.clone()))
            .collect();
        preview.expected = ids
            .iter()
            .filter_map(|id| self.model.timeline().snapshot().entity(id).cloned())
            .collect();
        preview.group_counts = preview
            .selected
            .iter()
            .filter_map(|id| self.model.timeline().snapshot().entity(id))
            .filter(|e| e.as_blind().is_some())
            .map(|entity| {
                let counts = pool_all_group_counts(entity)
                    .map(|counts| {
                        counts
                            .into_iter()
                            .map(|(id, colors)| {
                                (
                                    id,
                                    colors.iter().fold((0u64, 0u64), |(p, e), c| {
                                        (p + c.painted, e + c.effective)
                                    }),
                                )
                            })
                            .collect()
                    })
                    .map_err(|e| e.to_string());
                (entity.id().clone(), counts)
            })
            .collect();
        self.recompute_distribution_plan();
    }

    fn recompute_distribution_plan(&mut self) {
        if let Some(preview) = &mut self.distribution.preview {
            preview.plan = distribution_plan(
                self.model.timeline().snapshot(),
                &preview.normalized_request(),
            )
            .map_err(|e| e.to_string());
        }
    }
    pub(super) fn sync_distribution_selection(&mut self) {
        let selected = self.selected_entity_ids().to_vec();
        let Some(preview) = &mut self.distribution.preview else {
            return;
        };
        if preview.selected == selected {
            return;
        }
        let snapshot = self.model.timeline().snapshot();
        preview
            .request
            .pools
            .retain(|p| selected.contains(&p.entity_id));
        preview
            .request
            .layers
            .retain(|l| selected.contains(&l.entity_id));
        for id in &selected {
            if preview.selected.contains(id) {
                continue;
            }
            let Some(entity) = snapshot.entity(id) else {
                continue;
            };
            match entity.kind() {
                PlaceableEntityKind::Blind(blind) => {
                    preview.request.pools.push(DistributionPoolSelection {
                        entity_id: id.clone(),
                        group_ids: std::iter::once(0)
                            .chain(blind.distribution_groups().iter().map(|g| g.id))
                            .collect(),
                    })
                }
                PlaceableEntityKind::Block(block) => preview.request.layers.extend(
                    block
                        .collect_layers()
                        .iter()
                        .enumerate()
                        .filter(|(_, layer)| layer.color_index() != DISABLED_COLLECT_COLOR_INDEX)
                        .map(|(layer_index, _)| DistributionLayerSelection {
                            entity_id: id.clone(),
                            layer_index,
                            weight: 1,
                        }),
                ),
            }
        }
        preview.selected = selected;
        preview.quick = false;
        self.capture_distribution_guard();
        self.distribution.hover = None;
    }
    fn distribution_plan(&self) -> Result<&DistributionPlan, String> {
        let preview = self
            .distribution
            .preview
            .as_ref()
            .ok_or("Select Pools and Blocks, then choose Distribute.")?;
        if preview.target != self.target {
            return Err("Level changed. Cancel and start a new preview.".into());
        }
        if preview
            .expected
            .iter()
            .any(|e| self.model.timeline().snapshot().entity(e.id()) != Some(e))
        {
            return Err(
                "Stale preview: a selected entity changed. Refresh to capture the latest state."
                    .into(),
            );
        }
        if preview.request.pools.is_empty()
            || preview.request.pools.iter().all(|p| p.group_ids.is_empty())
        {
            return Err("Select at least one Pool group.".into());
        }
        if preview.request.layers.is_empty() {
            return Err("Select at least one Block layer.".into());
        }
        if let Some(error) = &preview.apply_error {
            return Err(error.clone());
        }
        if !preview.invalid_weights.is_empty() {
            return Err(format!(
                "Weights must be whole numbers from 0 to {MAX_DISTRIBUTION_WEIGHT}. Correct the weight or Equalize its color."
            ));
        }
        preview.plan.as_ref().map_err(Clone::clone)
    }
    pub(super) fn distribution_pointer_down(&mut self, event: PointerEvent) -> bool {
        event.prevent_default();
        if let Some(root) = self.root_ref.cast::<HtmlElement>() {
            let _ = root.focus();
        }
        let Some(point) = self.point_from_pointer(&event) else {
            return false;
        };
        let additive = event.shift_key() || event.ctrl_key() || event.meta_key();
        self.capture_pointer(event.pointer_id());
        if let Some(entity) = self.model.timeline().snapshot().entity_at(point) {
            self.selection = select_entity(self.selection.as_ref(), entity.id().clone(), additive);
        } else {
            let Some((x, y)) = self.canvas_position(&event) else {
                return false;
            };
            if !additive {
                self.selection = None;
            }
            self.marquee_gesture = Some(MarqueeGesture {
                pointer_id: event.pointer_id(),
                start: point,
                current: point,
                start_canvas_x: x,
                start_canvas_y: y,
                current_canvas_x: x,
                current_canvas_y: y,
                additive,
                active: false,
            });
        }
        self.sync_distribution_selection();
        self.canvas_dirty = true;
        true
    }
    pub(super) fn distribution_message(
        &mut self,
        ctx: &Context<Self>,
        msg: DistributionMsg,
    ) -> bool {
        if self.mode != Mode::Distribution || self.image_studio.apply.is_some() {
            return false;
        }
        let replan = matches!(
            &msg,
            DistributionMsg::Weight(..) | DistributionMsg::Equalize(_)
        );
        match msg {
            DistributionMsg::Preview(quick) => {
                if quick {
                    self.selection = Some(Selection::Entities(
                        self.model
                            .timeline()
                            .snapshot()
                            .entities()
                            .iter()
                            .map(|e| e.id().clone())
                            .collect(),
                    ));
                }
                self.distribution.preview = Some(DistributionPreview {
                    target: self.target.clone(),
                    selected: Vec::new(),
                    request: DistributionRequest {
                        pools: Vec::new(),
                        layers: Vec::new(),
                    },
                    expected: Vec::new(),
                    plan: Err("Select Pools and Blocks.".into()),
                    group_counts: BTreeMap::new(),
                    invalid_weights: BTreeSet::new(),
                    pending_command: None,
                    apply_error: None,
                    quick,
                });
                self.sync_distribution_selection();
                if let Some(p) = &mut self.distribution.preview {
                    p.quick = quick;
                }
                self.right_tab = RightTab::AdvancedDistribution;
                self.right_panel_layout = PanelLayout::Docked;
            }
            DistributionMsg::Cancel => {
                self.distribution.preview = None;
                self.distribution.hover = None;
            }
            DistributionMsg::Refresh => {
                // Deliberate acknowledgement of remote changes, never triggered by weight edits.
                // Remove vanished groups/layers so Refresh cannot leave an invisible invalid selection.
                if let Some(preview) = &mut self.distribution.preview {
                    preview.apply_error = None;
                    let snapshot = self.model.timeline().snapshot();
                    preview.request.pools.retain_mut(|pool| {
                        let Some(blind) =
                            snapshot.entity(&pool.entity_id).and_then(|e| e.as_blind())
                        else {
                            return false;
                        };
                        pool.group_ids.retain(|id| {
                            *id == 0 || blind.distribution_groups().iter().any(|g| g.id == *id)
                        });
                        true
                    });
                    preview.request.layers.retain(|layer| snapshot.entity(&layer.entity_id).is_some_and(|e| {
                        matches!(e.kind(), PlaceableEntityKind::Block(b) if b.collect_layers().get(layer.layer_index).is_some_and(|l| l.color_index() != DISABLED_COLLECT_COLOR_INDEX))
                    }));
                }
                self.capture_distribution_guard();
            }
            DistributionMsg::Apply => {
                if let Some(error) = self
                    .distribution_gate()
                    .or_else(|| self.distribution_plan().err())
                {
                    self.push_toast(error, "warning");
                    return true;
                }
                let Some(preview) = &mut self.distribution.preview else {
                    return false;
                };
                let ids = preview
                    .expected
                    .iter()
                    .map(|e| e.id().clone())
                    .collect::<Vec<_>>();
                let envelope = self.model.prepare_apply_distribution(
                    preview.normalized_request(),
                    preview.expected.clone(),
                    now_ms(),
                );
                preview.pending_command = Some(envelope.metadata.id.to_string());
                self.distribution.hover = None;
                return self.submit_entity_command(ctx, envelope, ids, Vec::new());
            }
            DistributionMsg::Group(id, group_id, checked) => {
                if let Some(p) = self
                    .distribution
                    .preview
                    .as_mut()
                    .and_then(|p| p.request.pools.iter_mut().find(|p| p.entity_id == id))
                {
                    p.group_ids.retain(|g| *g != group_id);
                    if checked {
                        p.group_ids.push(group_id);
                        p.group_ids.sort_unstable();
                    }
                }
                self.capture_distribution_guard();
            }
            DistributionMsg::Layer(id, layer_index, checked) => {
                if checked && !self.model.timeline().snapshot().entity(&id).is_some_and(|e| matches!(e.kind(), PlaceableEntityKind::Block(b) if b.collect_layers().get(layer_index).is_some_and(|l| l.color_index() != DISABLED_COLLECT_COLOR_INDEX))) { return false; }
                if let Some(preview) = &mut self.distribution.preview {
                    preview
                        .request
                        .layers
                        .retain(|l| l.entity_id != id || l.layer_index != layer_index);
                    if checked {
                        preview.request.layers.push(DistributionLayerSelection {
                            entity_id: id,
                            layer_index,
                            weight: 1,
                        });
                    }
                }
                self.capture_distribution_guard();
            }
            DistributionMsg::Weight(id, index, value) => {
                let weight = value
                    .parse::<u32>()
                    .ok()
                    .filter(|w| *w <= MAX_DISTRIBUTION_WEIGHT);
                if let Some(preview) = &mut self.distribution.preview {
                    if weight.is_some() {
                        preview.invalid_weights.remove(&(id.clone(), index));
                    } else {
                        preview.invalid_weights.insert((id.clone(), index));
                    }
                }
                let Some(weight) = weight else {
                    return true;
                };
                if let Some(layer) = self.distribution.preview.as_mut().and_then(|p| {
                    p.request
                        .layers
                        .iter_mut()
                        .find(|l| l.entity_id == id && l.layer_index == index)
                }) {
                    layer.weight = weight;
                }
            }
            DistributionMsg::Equalize(color) => {
                if let Some(preview) = &mut self.distribution.preview {
                    for layer in &mut preview.request.layers {
                        if self
                            .model
                            .timeline()
                            .snapshot()
                            .entity(&layer.entity_id)
                            .and_then(|e| match e.kind() {
                                PlaceableEntityKind::Block(b) => {
                                    b.collect_layers().get(layer.layer_index)
                                }
                                _ => None,
                            })
                            .is_some_and(|l| l.color_index() == color && !l.is_locked())
                        {
                            layer.weight = 1;
                            preview
                                .invalid_weights
                                .remove(&(layer.entity_id.clone(), layer.layer_index));
                        }
                    }
                }
            }
            DistributionMsg::Hover(hover) => self.distribution.hover = hover,
            DistributionMsg::Divide => {
                let Some(entity) = self
                    .selected_entity_id()
                    .and_then(|id| self.model.timeline().snapshot().entity(id))
                    .filter(|e| e.as_blind().is_some())
                    .cloned()
                else {
                    return false;
                };
                let groups = entity
                    .as_blind()
                    .expect("Pool checked")
                    .distribution_groups()
                    .to_vec();
                self.distribution.hover = None;
                self.distribution.division = Some(PoolDivision {
                    target: self.target.clone(),
                    expected: entity,
                    groups,
                    active: 0,
                    name: String::new(),
                    freehand: false,
                    selection: BTreeSet::new(),
                    gesture: None,
                    canvas: NodeRef::default(),
                });
            }
            DistributionMsg::CloseDivision => self.distribution.division = None,
            DistributionMsg::SaveGroups => {
                let Some(d) = &self.distribution.division else {
                    return false;
                };
                if let Some(error) = self.distribution_gate().or_else(|| {
                    (d.target != self.target
                        || self.model.timeline().snapshot().entity(d.expected.id())
                            != Some(&d.expected))
                    .then(|| {
                        "Pool changed while dividing. Cancel and reopen Pool Division to refresh."
                            .into()
                    })
                }) {
                    self.push_toast(error, "warning");
                    return true;
                }
                let d = self.distribution.division.take().expect("division checked");
                let id = d.expected.id().clone();
                let envelope = self.model.prepare_set_pool_distribution_groups(
                    id.clone(),
                    d.expected,
                    d.groups,
                    now_ms(),
                );
                return self.submit_entity_command(ctx, envelope, [id], Vec::new());
            }
            DistributionMsg::DivisionTool(freehand) => {
                if let Some(d) = &mut self.distribution.division {
                    d.freehand = freehand;
                    d.gesture = None;
                }
            }
            DistributionMsg::ActiveGroup(id) => {
                if let Some(d) = &mut self.distribution.division {
                    d.active = id;
                }
            }
            DistributionMsg::NewName(name) => {
                if let Some(d) = &mut self.distribution.division {
                    d.name = name;
                }
            }
            DistributionMsg::AddGroup => {
                if let Some(d) = &mut self.distribution.division {
                    if d.name.trim().is_empty() || d.groups.len() >= MAX_POOL_DISTRIBUTION_GROUPS {
                        return false;
                    }
                    let Some(id) = d
                        .groups
                        .iter()
                        .map(|g| g.id)
                        .max()
                        .unwrap_or(0)
                        .checked_add(1)
                    else {
                        return false;
                    };
                    d.groups.push(PoolDistributionGroup {
                        id,
                        name: d.name.trim().to_owned(),
                        pixels: Vec::new(),
                    });
                    d.active = id;
                    d.name.clear();
                    d.assign();
                }
            }
            DistributionMsg::DeleteGroup(id) => {
                if let Some(d) = &mut self.distribution.division {
                    d.groups.retain(|g| g.id != id);
                    if d.active == id {
                        d.active = 0;
                    }
                }
            }
            DistributionMsg::RenameGroup(id, name) => {
                if let Some(d) = &mut self.distribution.division {
                    if !name.trim().is_empty() {
                        if let Some(g) = d.groups.iter_mut().find(|g| g.id == id) {
                            g.name = name.trim().to_owned();
                        }
                    }
                }
            }
            DistributionMsg::Assign => {
                if let Some(d) = &mut self.distribution.division {
                    d.assign();
                }
            }
            DistributionMsg::ClearRegion => {
                if let Some(d) = &mut self.distribution.division {
                    d.selection.clear();
                    d.gesture = None;
                }
            }
            DistributionMsg::RegionDown(event) => {
                if let Some(d) = &mut self.distribution.division {
                    if event.button() != 0 {
                        return false;
                    }
                    event.prevent_default();
                    event.stop_propagation();
                    if let Some(point) = d.point(&event) {
                        if let Some(canvas) = d.canvas.cast::<HtmlCanvasElement>() {
                            let _ = canvas.set_pointer_capture(event.pointer_id());
                        }
                        d.gesture = Some(RegionGesture {
                            pointer: event.pointer_id(),
                            points: vec![point],
                        });
                    }
                }
            }
            DistributionMsg::RegionMove(event) => {
                if let Some(d) = &mut self.distribution.division {
                    event.prevent_default();
                    let point = d.point(&event);
                    if let Some(gesture) = &mut d.gesture {
                        if gesture.pointer == event.pointer_id() {
                            if let Some(point) = point {
                                if d.freehand {
                                    if gesture
                                        .points
                                        .last()
                                        .is_none_or(|p| (p.0 - point.0).hypot(p.1 - point.1) > 0.5)
                                    {
                                        gesture.points.push(point);
                                    }
                                } else {
                                    gesture.points.truncate(1);
                                    gesture.points.push(point);
                                }
                            }
                        }
                    }
                }
            }
            DistributionMsg::RegionUp(event) => {
                self.distribution_message(ctx, DistributionMsg::RegionMove(event.clone()));
                if let Some(d) = &mut self.distribution.division {
                    if d.gesture
                        .as_ref()
                        .is_some_and(|g| g.pointer == event.pointer_id())
                    {
                        d.selection = d.selected_region();
                        d.gesture = None;
                        if let Some(canvas) = d.canvas.cast::<HtmlCanvasElement>() {
                            let _ = canvas.release_pointer_capture(event.pointer_id());
                        }
                    }
                }
            }
            DistributionMsg::RegionCancel(event) => {
                if let Some(d) = &mut self.distribution.division {
                    d.gesture = None;
                    if let Some(canvas) = d.canvas.cast::<HtmlCanvasElement>() {
                        let _ = canvas.release_pointer_capture(event.pointer_id());
                    }
                }
            }
        }
        if replan {
            self.recompute_distribution_plan();
        }
        self.canvas_dirty = true;
        true
    }

    pub(super) fn view_distribution_tools(&self, ctx: &Context<Self>) -> Html {
        let pool = self
            .selected_entity_id()
            .and_then(|id| self.model.timeline().snapshot().entity(id))
            .is_some_and(|e| e.as_blind().is_some());
        html! { <nav class="canvas-toolbox contextual-canvas-tools" aria-label="Distribution tools">
            <button disabled={!pool} title="Select exactly one Pool" onclick={ctx.link().callback(|_|Msg::Distribution(DistributionMsg::Divide))}>{"Pool Division"}</button>
            <button onclick={ctx.link().callback(|_|Msg::Distribution(DistributionMsg::Preview(false)))}>{"Distribute"}</button>
            <button onclick={ctx.link().callback(|_|Msg::Distribution(DistributionMsg::Preview(true)))}>{"Quick Distribute"}</button>
        </nav> }
    }

    pub(super) fn view_distribution_panel(&self, ctx: &Context<Self>) -> Html {
        if self.mode != Mode::Distribution {
            return Html::default();
        }
        let Some(preview) = &self.distribution.preview else {
            return html! { <section class="distribution-panel"><h3>{"Advanced Distribution"}</h3><p>{"Select Pools and Blocks on the board. Shift-click or marquee adds entities without moving them."}</p><button onclick={ctx.link().callback(|_|Msg::Distribution(DistributionMsg::Preview(false)))}>{"Preview selection"}</button></section> };
        };
        let plan = self.distribution_plan();
        let gate = self.distribution_gate();
        let snapshot = self.model.timeline().snapshot();
        html! { <section class="distribution-panel">
            <h3>{"Distribution preview"}</h3>
            <p>{if preview.quick {"Scope: all Pools and Block layers. Equal weights; nothing applied yet."} else {"Scope: selected Pools and Block layers. Keep selecting on the board to change the scope."}}</p>
            <p>{"Colors stay unchanged. Disabled-color layers are excluded. Locked finite layers reserve capacity; only unlocked selected layers are allocated."}</p>
            <h4>{"Source Pool groups"}</h4>
            {for preview.selected.iter().filter_map(|id|snapshot.entity(id)).filter_map(|entity|entity.as_blind().map(|b|(entity,b))).map(|(entity,blind)| {
                let selected = preview.request.pools.iter().find(|p|p.entity_id == *entity.id());
                let names = std::iter::once((0,"Default".to_owned())).chain(blind.distribution_groups().iter().map(|g|(g.id,g.name.clone())));
                html!{<fieldset key={format!("pool-{}",entity.id())}><legend>{format!("Pool {}",entity.id())}</legend>{for names.map(|(group_id,name)| {
                    let id = entity.id().clone(); let hover_id=id.clone(); let focus_id=id.clone();
                    let checked=selected.is_some_and(|p|p.group_ids.contains(&group_id));
                    let totals=preview.group_counts.get(entity.id()).map_or(Ok((0,0)), |counts| counts.as_ref().map(|c|c.get(&group_id).copied().unwrap_or_default()).map_err(Clone::clone));
                    html!{<label key={format!("{}-{group_id}",id)} class="distribution-group-row" onmouseenter={ctx.link().callback(move |_|Msg::Distribution(DistributionMsg::Hover(Some((hover_id.clone(),group_id)))))} onmouseleave={ctx.link().callback(|_|Msg::Distribution(DistributionMsg::Hover(None)))} onfocusin={ctx.link().callback(move |_|Msg::Distribution(DistributionMsg::Hover(Some((focus_id.clone(),group_id)))))} onfocusout={ctx.link().callback(|_|Msg::Distribution(DistributionMsg::Hover(None)))}>
                        <input type="checkbox" checked={checked} onchange={ctx.link().callback(move |e:Event|Msg::Distribution(DistributionMsg::Group(id.clone(),group_id,e.target_unchecked_into::<HtmlInputElement>().checked())))} />
                        <span>{name}<small>{match totals {Ok((p,e))=>format!("{p} painted / {e} effective / {} excluded",p.saturating_sub(e)),Err(e)=>e.to_string()}}</small></span>
                    </label>}
                })}</fieldset>}
            })}
            <h4>{"Target Block layers / integer weights"}</h4>
            <div class="distribution-actions">{for preview.request.layers.iter().filter_map(|l|snapshot.entity(&l.entity_id).and_then(|e|match e.kind(){PlaceableEntityKind::Block(b)=>b.collect_layers().get(l.layer_index),_=>None})).filter(|l|!l.is_locked()).map(|l|l.color_index()).collect::<BTreeSet<_>>().into_iter().map(|color|html!{<button key={format!("equalize-{color}")} onclick={ctx.link().callback(move |_|Msg::Distribution(DistributionMsg::Equalize(color)))}>{format!("Equalize color {color}")}</button>})}</div>
            {for preview.selected.iter().filter_map(|id|snapshot.entity(id)).filter_map(|entity| match entity.kind() {PlaceableEntityKind::Block(b)=>Some((entity,b)),_=>None}).map(|(entity,block)|html!{
                <fieldset key={format!("block-{}",entity.id())}><legend>{format!("Block {}",entity.id())}</legend>{for block.collect_layers().iter().enumerate().map(|(index,layer)|{
                    let id=entity.id().clone(); let weight_id=id.clone();
                    let selection=preview.request.layers.iter().find(|l|l.entity_id == id && l.layer_index == index);
                    html!{<div key={format!("{}-{index}",entity.id())} class="distribution-layer-row"><label><input type="checkbox" checked={selection.is_some()} disabled={layer.color_index() == DISABLED_COLLECT_COLOR_INDEX} onchange={ctx.link().callback(move |e:Event|Msg::Distribution(DistributionMsg::Layer(id.clone(),index,e.target_unchecked_into::<HtmlInputElement>().checked())))} /><span style={format!("color:{}",block_color(layer.color_index()))}>{if layer.color_index() == DISABLED_COLLECT_COLOR_INDEX {format!("Layer {} · Disabled",index+1)} else {format!("Layer {} · color {}",index+1,layer.color_index())}}</span></label><small>{format!("{}{}",capacity_label(layer.capacity()),if layer.is_locked(){" · locked"}else{""})}</small><input aria-label={format!("Weight for Block {} layer {}",entity.id(),index+1)} type="number" min="0" max={MAX_DISTRIBUTION_WEIGHT.to_string()} step="1" disabled={layer.is_locked() || selection.is_none()} value={selection.map_or(1,|l|l.weight).to_string()} onchange={ctx.link().callback(move |e:Event|Msg::Distribution(DistributionMsg::Weight(weight_id.clone(),index,e.target_unchecked_into::<HtmlInputElement>().value())))} /></div>}
                })}</fieldset>
            })}
            {match &plan {
                Err(error)=>html!{<p class="distribution-error" role="alert">{error}</p>},
                Ok(plan)=>html!{<><h4>{"After border padding"}</h4>{for plan.colors.iter().map(|color|{
                    let color_index=color.color_index;
                    let allocations=plan.layers.iter().filter(|allocation|snapshot.entity(&allocation.entity_id).and_then(|e|match e.kind(){PlaceableEntityKind::Block(b)=>b.collect_layers().get(allocation.layer_index),_=>None}).is_some_and(|l|l.color_index()==color_index)).collect::<Vec<_>>();
                    html!{<section key={color_index.to_string()} class="distribution-color"><header><strong>{format!("Color {color_index}")}</strong></header><small>{format!("{} painted · {} effective · {} excluded",color.painted,color.effective,color.painted.saturating_sub(color.effective))}</small><div class="distribution-bar" aria-label={format!("Color {color_index} capacity allocation")}>{for allocations.iter().enumerate().map(|(i,l)|{let percent=if color.effective==0 {0.0}else{f64::from(l.capacity)/color.effective as f64*100.0}; html!{<span key={format!("{}-{}",l.entity_id,l.layer_index)} style={format!("width:{percent}%;background:{};opacity:{}",block_color(color_index),if i%2==0 {1.0}else{0.6})} title={format!("{} / layer {}: {} ({percent:.1}%){}",l.entity_id,l.layer_index+1,l.capacity,if l.locked{" locked"}else{""})}></span>}})}</div>{for allocations.iter().map(|l|{let percent=if color.effective==0 {0.0}else{f64::from(l.capacity)/color.effective as f64*100.0};html!{<small key={format!("allocation-{}-{}",l.entity_id,l.layer_index)}>{format!("{} / layer {}: {} → {} ({percent:.1}%){}",l.entity_id,l.layer_index+1,capacity_label(l.before),l.capacity,if l.locked{" · locked"}else{""})}</small>}})}{if color.unmatched>0 {html!{<p class="distribution-warning">{format!("Unmatched color {color_index}: {} pixels have no receiving layer.",color.unmatched)}</p>}}else{Html::default()}}</section>}
                })}</>},
            }}
            {gate.as_ref().map(|e|html!{<p class="distribution-warning">{e}</p>}).unwrap_or_default()}
            <div class="distribution-actions"><button onclick={ctx.link().callback(|_|Msg::Distribution(DistributionMsg::Refresh))}>{"Refresh"}</button><button onclick={ctx.link().callback(|_|Msg::Distribution(DistributionMsg::Cancel))}>{"Reset / Cancel"}</button><button disabled={gate.is_some() || plan.is_err()} onclick={ctx.link().callback(|_|Msg::Distribution(DistributionMsg::Apply))}>{"Apply"}</button></div>
        </section> }
    }

    pub(super) fn view_pool_division(&self, ctx: &Context<Self>) -> Html {
        let Some(d) = &self.distribution.division else {
            return Html::default();
        };
        let stale = d.target != self.target
            || self.model.timeline().snapshot().entity(d.expected.id()) != Some(&d.expected);
        let invalid = d
            .expected
            .with_pool_distribution_groups(d.groups.clone())
            .err()
            .map(|e| e.to_string());
        let gate = self.distribution_gate();
        html! {<div class="studio-modal-backdrop distribution-backdrop"><section role="dialog" aria-modal="true" aria-label="Pool Division" class="pool-division-modal">
            <header><div><h2>{"Pool Division"}</h2><p>{format!("Pool {} · draw a pixel region, then assign it to a group",d.expected.id())}</p></div><button aria-label="Cancel Pool Division" onclick={ctx.link().callback(|_|Msg::Distribution(DistributionMsg::CloseDivision))}>{"Cancel"}</button></header>
            <div class="division-layout"><div class="division-preview"><div class="distribution-actions"><button class={classes!((!d.freehand).then_some("active"))} onclick={ctx.link().callback(|_|Msg::Distribution(DistributionMsg::DivisionTool(false)))}>{"Rectangle"}</button><button class={classes!(d.freehand.then_some("active"))} onclick={ctx.link().callback(|_|Msg::Distribution(DistributionMsg::DivisionTool(true)))}>{"Freehand"}</button><button onclick={ctx.link().callback(|_|Msg::Distribution(DistributionMsg::ClearRegion))}>{"Clear selection"}</button></div><canvas ref={d.canvas.clone()} width="640" height="640" aria-label="Isolated Pool pixel region selection" onpointerdown={ctx.link().callback(|e|Msg::Distribution(DistributionMsg::RegionDown(e)))} onpointermove={ctx.link().callback(|e|Msg::Distribution(DistributionMsg::RegionMove(e)))} onpointerup={ctx.link().callback(|e|Msg::Distribution(DistributionMsg::RegionUp(e)))} onpointercancel={ctx.link().callback(|e|Msg::Distribution(DistributionMsg::RegionCancel(e)))} /><p>{format!("{} pixels selected (including empty pixels)",d.selection.len())}</p></div>
            <aside><h3>{"Named groups"}</h3><p>{"Groups are permanent, non-overlapping pixel masks. Assignment transfers pixels out of other groups. Default covers everything not in a named group, including empty future paint."}</p><label class="distribution-group-row"><input type="radio" name="division-group" checked={d.active==0} onchange={ctx.link().callback(|_|Msg::Distribution(DistributionMsg::ActiveGroup(0)))} />{"Default (implicit 0)"}</label>
            {for d.groups.iter().map(|g|{let id=g.id;html!{<div key={id.to_string()} class="division-group"><input type="radio" name="division-group" aria-label={format!("Select {}",g.name)} checked={d.active==id} onchange={ctx.link().callback(move |_|Msg::Distribution(DistributionMsg::ActiveGroup(id)))} /><input aria-label="Group name" maxlength={MAX_POOL_DISTRIBUTION_GROUP_NAME_CHARS.to_string()} value={g.name.clone()} onchange={ctx.link().callback(move |e:Event|Msg::Distribution(DistributionMsg::RenameGroup(id,e.target_unchecked_into::<HtmlInputElement>().value())))} /><small>{format!("{} px",g.pixels.len())}</small><button aria-label={format!("Delete {}",g.name)} onclick={ctx.link().callback(move |_|Msg::Distribution(DistributionMsg::DeleteGroup(id)))}>{"Delete"}</button></div>}})}
            <label>{"New group name"}<input maxlength={MAX_POOL_DISTRIBUTION_GROUP_NAME_CHARS.to_string()} value={d.name.clone()} oninput={ctx.link().callback(|e:InputEvent|Msg::Distribution(DistributionMsg::NewName(e.target_unchecked_into::<HtmlInputElement>().value())))} /></label><button disabled={d.name.trim().is_empty() || d.groups.len() >= MAX_POOL_DISTRIBUTION_GROUPS} onclick={ctx.link().callback(|_|Msg::Distribution(DistributionMsg::AddGroup))}>{"Add group & assign selection"}</button><button disabled={d.selection.is_empty()} onclick={ctx.link().callback(|_|Msg::Distribution(DistributionMsg::Assign))}>{"Assign selection to selected group"}</button>
            </aside></div><footer><p>{"Save Groups is one undoable change. Cancel leaves the Pool unchanged."}</p>{if stale {html!{<p class="distribution-error" role="alert">{"Pool changed. Cancel and reopen to refresh; your draft cannot overwrite it."}</p>}}else{Html::default()}}{invalid.as_ref().or(gate.as_ref()).map(|e|html!{<p class="distribution-error">{e}</p>}).unwrap_or_default()}<div class="distribution-actions"><button onclick={ctx.link().callback(|_|Msg::Distribution(DistributionMsg::CloseDivision))}>{"Cancel"}</button><button disabled={stale || invalid.is_some() || gate.is_some()} onclick={ctx.link().callback(|_|Msg::Distribution(DistributionMsg::SaveGroups))}>{"Save Groups"}</button></div></footer>
        </section></div>}
    }
    pub(super) fn draw_division_canvas(&self) {
        let Some(d) = &self.distribution.division else {
            return;
        };
        let Some(canvas) = d.canvas.cast::<HtmlCanvasElement>() else {
            return;
        };
        let Some(context) = canvas
            .get_context("2d")
            .ok()
            .flatten()
            .and_then(|c| c.dyn_into::<CanvasRenderingContext2d>().ok())
        else {
            return;
        };
        context.set_fill_style_str("#18212b");
        context.fill_rect(0.0, 0.0, 640.0, 640.0);
        context.save();
        let _ = context.translate(20.0, 20.0);
        let _ = context.scale(d.scale(), d.scale());
        let selection = d.selected_region();
        for (pixel, (x, y, w, h), color) in pool_pixels(&d.expected) {
            let member = group_contains(&d.groups, d.active, pixel);
            context.set_global_alpha(if member { 1.0 } else { 0.3 });
            context.set_fill_style_str(if color == 0 {
                "#64717d"
            } else {
                blind_color(color)
            });
            context.fill_rect(x, y, w + 0.1, h + 0.1);
            if selection.contains(&pixel) {
                context.set_global_alpha(0.65);
                context.set_fill_style_str("#ffe178");
                context.fill_rect(x, y, w, h);
            }
        }
        context.set_global_alpha(1.0);
        if let Some(gesture) = &d.gesture {
            context.set_stroke_style_str("#ffffff");
            context.set_line_width(1.5 / d.scale());
            context.begin_path();
            let first = gesture.points[0];
            if d.freehand {
                context.move_to(first.0, first.1);
                for &(x, y) in &gesture.points {
                    context.line_to(x, y);
                }
                context.close_path();
                context.stroke();
            } else {
                let end = *gesture.points.last().unwrap_or(&first);
                context.stroke_rect(first.0, first.1, end.0 - first.0, end.1 - first.1);
            }
        }
        context.restore();
    }
    pub(super) fn draw_distribution_highlight(&self, context: &CanvasRenderingContext2d) {
        let Some((id, group)) = &self.distribution.hover else {
            return;
        };
        let Some(entity) = self.model.timeline().snapshot().entity(id) else {
            return;
        };
        let Some(blind) = entity.as_blind() else {
            return;
        };
        let origin = entity.origin();
        context.save();
        for (pixel, (x, y, w, h), color) in pool_pixels(entity) {
            if !group_contains(blind.distribution_groups(), *group, pixel) {
                continue;
            }
            context.set_fill_style_str(if color == 0 {
                "#81958c"
            } else {
                blind_color(color)
            });
            context.fill_rect(
                BOARD_ORIGIN + f64::from(origin.x) * CELL_SIZE + x,
                BOARD_ORIGIN + f64::from(origin.y) * CELL_SIZE + y,
                w + 0.1,
                h + 0.1,
            );
        }
        context.restore();
    }
}
fn capacity_label(capacity: CollectCapacity) -> String {
    match capacity {
        CollectCapacity::Unlimited => "Unlimited".into(),
        CollectCapacity::Finite(value) => value.to_string(),
    }
}

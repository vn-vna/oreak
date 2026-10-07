//! Image Studio owns preview-only state; authoritative writes use the normal pending RPC path.
use super::*;
use crate::api::{ImagePixels, PreparedImage, PreparedImageEntry};
use oreak_core::{
    DitherMode, ImagePlacement, ImageProjector, ImageSampling, ImageTransparency, IndexedImage,
    PaletteMapping, PaletteSettings, SourceColorCluster, convert_rgba, extract_color_clusters,
};
use oreak_protocol::ApplyImageRequest;

#[derive(Default)]
pub(super) struct ImageStudioState {
    generation: u64,
    epoch: u64,
    catalog_generation: u64,
    prepared: Vec<PreparedImageEntry>,
    converter: Option<Converter>,
    pub(super) apply: Option<ApplySession>,
}
struct Converter {
    token: u64,
    source_id: String,
    name: String,
    settings: PaletteSettings,
    pixels: Option<ImagePixels>,
    source_clusters: Vec<SourceColorCluster>,
    dragging_source: Option<[u8; 3]>,
    preview: Option<IndexedImage>,
    preview_url: String,
    original_url: String,
    zoom: f64,
    busy: bool,
    error: Option<String>,
}
pub(super) struct ApplySession {
    token: u64,
    target: ProjectLevelTarget,
    previous: PreviousWorkspace,
    prepared: Option<PreparedImage>,
    overlay_canvas: Option<HtmlCanvasElement>,
    placement: ImagePlacement,
    targets: Vec<PlaceableEntity>,
    projections: BTreeMap<EntityId, PlaceableEntity>,
    projector: Option<ImageProjector>,
    projector_key: Option<(String, ImageSampling, u16)>,
    adjust: bool,
    lock_aspect: bool,
    opacity: f64,
    busy: bool,
    error: Option<String>,
    pending_command: Option<String>,
    gesture: Option<ImageGesture>,
}
struct PreviousWorkspace {
    mode: Mode,
    tool: WorkspaceTool,
    tab: LeftTab,
    layout: PanelLayout,
    right_layout: PanelLayout,
    isolation: Option<EntityId>,
    selection: Option<Selection>,
}
#[derive(Clone)]
struct ImageGesture {
    pointer: i32,
    start: (f64, f64),
    current: (f64, f64),
    placement: ImagePlacement,
    handle: Option<(bool, bool)>,
    additive: bool,
}
#[derive(Clone, Copy)]
pub(crate) enum PlacementField {
    X,
    Y,
    Width,
    Height,
    Pixelation,
    Resolution,
    Opacity,
}
#[derive(Clone, Copy)]
pub(crate) enum ConverterField {
    Name,
    Alpha,
    Background,
    Zoom,
}
pub(crate) enum ImageMsg {
    Convert(String),
    CancelConversion,
    Color(u8),
    BeginColorDrag([u8; 3]),
    EndColorDrag,
    DropColor(u8),
    MapColor([u8; 3], Option<u8>),
    ResetMappings,
    Dither(bool),
    ConverterField(ConverterField, String),
    SavePrepared,
    Pixels {
        project: String,
        token: u64,
        result: Result<ImagePixels, ApiError>,
    },
    Saved {
        project: String,
        epoch: u64,
        token: u64,
        result: Result<PreparedImage, ApiError>,
    },
    Catalog {
        project: String,
        token: u64,
        result: Result<Vec<PreparedImageEntry>, ApiError>,
    },
    Start(String),
    Choose(String),
    Cancel,
    Adjust(bool),
    LockAspect(bool),
    Placement(PlacementField, String),
    Fit(bool),
    Reset,
    Sampling(bool),
    Transparency(bool),
    Prepared {
        target: ProjectLevelTarget,
        token: u64,
        result: Result<PreparedImage, ApiError>,
    },
    Apply,
    Applied {
        target: ProjectLevelTarget,
        token: u64,
        attempt: u64,
        command_id: String,
        result: Result<ApplyCommandResponse, String>,
    },
}

impl Converter {
    fn refresh(&mut self) {
        let Some(pixels) = &self.pixels else {
            return;
        };
        match convert_rgba(pixels.width, pixels.height, &pixels.rgba, &self.settings) {
            Ok(image) => {
                self.preview_url = indexed_url(&image);
                self.preview = Some(image);
                self.error = None;
            }
            Err(error) => {
                self.preview = None;
                self.preview_url.clear();
                self.error = Some(error.to_string());
            }
        }
    }
}
impl ApplySession {
    fn refresh(&mut self) {
        self.projections.clear();
        self.error = None;
        // Match the authoritative command budget before allocating Area tables or
        // cloning/projecting target pixels. Input snapshots have a separate budget.
        if self.targets.len() > 64 {
            self.error = Some("Select at most 64 Pools per Apply Image command.".into());
            return;
        }
        let mut input_pixels = 0usize;
        let mut output_pixels = 0usize;
        for target in &self.targets {
            let Some(blind) = target.as_blind() else {
                self.error = Some("Only Pools can receive an image.".into());
                return;
            };
            let cells = target.shape().occupied_cells().count();
            let input_resolution = usize::from(blind.pixels_per_cell());
            let output_resolution =
                usize::from(self.placement.resolution.unwrap_or(blind.pixels_per_cell()));
            input_pixels += cells * input_resolution * input_resolution;
            output_pixels += cells * output_resolution * output_resolution;
            if input_pixels > 1_000_000 || output_pixels > 1_000_000 {
                self.error = Some("Apply Image is limited to 1,000,000 source and output Pool pixels. Select fewer Pools or lower the output resolution.".into());
                return;
            }
        }
        let Some(prepared) = &self.prepared else {
            return;
        };
        let key = (
            prepared.entry.id.clone(),
            self.placement.sampling,
            self.placement.pixelation,
        );
        if self.projector_key.as_ref() != Some(&key) {
            match ImageProjector::new(
                &prepared.image,
                &prepared.entry.settings,
                self.placement.sampling,
                self.placement.pixelation,
            ) {
                Ok(projector) => {
                    self.projector = Some(projector);
                    self.projector_key = Some(key);
                }
                Err(error) => {
                    self.projector = None;
                    self.projector_key = None;
                    self.error = Some(error.to_string());
                    return;
                }
            }
        }
        let Some(projector) = &self.projector else {
            return;
        };
        for target in &self.targets {
            match projector.project(&self.placement, target) {
                Ok(entity) => {
                    self.projections.insert(entity.id().clone(), entity);
                }
                Err(error) => {
                    self.error = Some(error.to_string());
                    self.projections.clear();
                    break;
                }
            }
        }
    }
    fn fit(&mut self, cover: bool) {
        let Some(prepared) = &self.prepared else {
            return;
        };
        let Some((x, y, w, h)) = target_bounds(&self.targets) else {
            return;
        };
        let ratio = f64::from(prepared.image.width) / f64::from(prepared.image.height);
        let width = if cover {
            w.max(h * ratio)
        } else {
            w.min(h * ratio)
        };
        self.placement.x = x + (w - width) / 2.0;
        self.placement.y = y + (h - width / ratio) / 2.0;
        self.placement.width = width;
        self.placement.height = width / ratio;
        self.refresh();
    }
}
fn default_placement() -> ImagePlacement {
    ImagePlacement {
        x: 0.0,
        y: 0.0,
        width: 4.0,
        height: 4.0,
        sampling: ImageSampling::Nearest,
        pixelation: 1,
        resolution: None,
        transparency: ImageTransparency::Preserve,
    }
}
fn target_bounds(targets: &[PlaceableEntity]) -> Option<(f64, f64, f64, f64)> {
    let points = targets
        .iter()
        .flat_map(|e| shape_world_points(e.origin(), e.shape()).unwrap_or_default())
        .collect::<Vec<_>>();
    let x = points.iter().map(|p| p.x).min()?;
    let y = points.iter().map(|p| p.y).min()?;
    Some((
        f64::from(x),
        f64::from(y),
        f64::from(points.iter().map(|p| p.x).max()? - x + 1),
        f64::from(points.iter().map(|p| p.y).max()? - y + 1),
    ))
}
fn rgba_url(pixels: &ImagePixels) -> String {
    let make = || -> Result<String, JsValue> {
        let document = web_sys::window().unwrap().document().unwrap();
        let canvas = document
            .create_element("canvas")?
            .dyn_into::<HtmlCanvasElement>()?;
        canvas.set_width(pixels.width);
        canvas.set_height(pixels.height);
        let context = canvas
            .get_context("2d")?
            .unwrap()
            .dyn_into::<CanvasRenderingContext2d>()?;
        let data = web_sys::ImageData::new_with_u8_clamped_array_and_sh(
            wasm_bindgen::Clamped(&pixels.rgba),
            pixels.width,
            pixels.height,
        )?;
        context.put_image_data(&data, 0.0, 0.0)?;
        canvas.to_data_url()
    };
    make().unwrap_or_default()
}
fn indexed_canvas(image: &IndexedImage) -> Option<HtmlCanvasElement> {
    let document = web_sys::window()?.document()?;
    let canvas = document
        .create_element("canvas")
        .ok()?
        .dyn_into::<HtmlCanvasElement>()
        .ok()?;
    canvas.set_width(image.width);
    canvas.set_height(image.height);
    let context = canvas
        .get_context("2d")
        .ok()??
        .dyn_into::<CanvasRenderingContext2d>()
        .ok()?;
    let rgba = image
        .pixels
        .iter()
        .flat_map(|&color| {
            if color == 0 {
                [0, 0, 0, 0]
            } else {
                let rgb = oreak_core::PALETTE_RGB[usize::from(color - 1)];
                [rgb[0], rgb[1], rgb[2], 255]
            }
        })
        .collect::<Vec<_>>();
    let data = web_sys::ImageData::new_with_u8_clamped_array_and_sh(
        wasm_bindgen::Clamped(&rgba),
        image.width,
        image.height,
    )
    .ok()?;
    context.put_image_data(&data, 0.0, 0.0).ok()?;
    Some(canvas)
}
fn indexed_url(image: &IndexedImage) -> String {
    indexed_canvas(image)
        .and_then(|canvas| canvas.to_data_url().ok())
        .unwrap_or_default()
}

fn source_rgb_css(rgb: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}

fn palette_drag_callback(ctx: &Context<App>, source_rgb: [u8; 3]) -> Callback<DragEvent> {
    ctx.link().callback(move |event: DragEvent| {
        if let Some(data_transfer) = event.data_transfer() {
            data_transfer.set_effect_allowed("move");
            let _ = data_transfer.set_data(
                "application/x-oreak-source-rgb",
                &format!("{},{},{}", source_rgb[0], source_rgb[1], source_rgb[2]),
            );
        }
        Msg::ImageStudio(ImageMsg::BeginColorDrag(source_rgb))
    })
}

fn palette_mapping_chip(
    ctx: &Context<App>,
    source_rgb: [u8; 3],
    pixel_count: String,
    target_color: Option<u8>,
    enabled_colors: &[u8],
    busy: bool,
) -> Html {
    let select_rgb = source_rgb;
    let remove_rgb = source_rgb;
    html! {
        <article
            key={format!("{}-{}-{}", source_rgb[0], source_rgb[1], source_rgb[2])}
            class={classes!("palette-source-chip", target_color.is_some().then_some("mapped"))}
            draggable={(!busy).to_string()}
            ondragstart={palette_drag_callback(ctx, source_rgb)}
            ondragend={ctx.link().callback(|_| Msg::ImageStudio(ImageMsg::EndColorDrag))}
        >
            <span class="palette-source-swatch" style={format!("--source-color:{}", source_rgb_css(source_rgb))}></span>
            <span class="palette-source-copy"><code>{source_rgb_css(source_rgb)}</code><small>{format!("{pixel_count} px")}</small></span>
            <select
                aria-label={format!("Target color for source {}", source_rgb_css(source_rgb))}
                disabled={busy}
                onchange={ctx.link().callback(move |event: Event| {
                    let value = event.target_unchecked_into::<HtmlSelectElement>().value();
                    Msg::ImageStudio(ImageMsg::MapColor(
                        select_rgb,
                        value.parse::<u8>().ok().filter(|color| *color != 0),
                    ))
                })}
            >
                <option value="0" selected={target_color.is_none()}>{if target_color.is_some() { "Remove mapping" } else { "Choose target…" }}</option>
                {for enabled_colors.iter().map(|color| html! { <option value={color.to_string()} selected={Some(*color) == target_color}>{format!("Color {color}")}</option> })}
            </select>
            {target_color.map(|_| html! {
                <button
                    type="button"
                    class="palette-mapping-remove"
                    disabled={busy}
                    aria-label={format!("Remove mapping for source {}", source_rgb_css(source_rgb))}
                    onclick={ctx.link().callback(move |_| Msg::ImageStudio(ImageMsg::MapColor(remove_rgb, None)))}
                >{"Remove"}</button>
            }).unwrap_or_default()}
        </article>
    }
}

impl App {
    pub(super) fn load_prepared_catalog(&mut self, ctx: &Context<Self>) {
        self.image_studio.catalog_generation += 1;
        let token = self.image_studio.catalog_generation;
        let project = self.target.project_id.to_string();
        let link = ctx.link().clone();
        wasm_bindgen_futures::spawn_local(async move {
            let result = RestClient.list_prepared_images(&project).await;
            link.send_message(Msg::ImageStudio(ImageMsg::Catalog {
                project,
                token,
                result,
            }));
        });
    }
    pub(super) fn image_scope(&self) -> (u64, u64) {
        (self.image_studio.epoch, self.image_studio.generation)
    }
    pub(super) fn image_request_interrupted(&mut self) {
        if let Some(a) = self
            .image_studio
            .apply
            .as_mut()
            .filter(|a| a.pending_command.is_some())
        {
            a.busy = false;
            a.pending_command = None;
            a.error = Some("Connection changed during Apply. Verify the level, then reselect Pools before retrying.".into());
        }
    }
    pub(super) fn invalidate_image_studio(&mut self) {
        self.image_studio.epoch += 1;
        self.cancel_apply_image();
        self.image_studio.generation += 1;
        self.image_studio.catalog_generation += 1;
        self.image_studio.converter = None;
        self.image_studio.prepared.clear();
    }
    pub(super) fn cancel_conversion(&mut self) {
        self.image_studio.generation += 1;
        self.image_studio.converter = None;
    }
    pub(super) fn cancel_apply_image(&mut self) -> bool {
        let Some(session) = self.image_studio.apply.take() else {
            return false;
        };
        self.mode = session.previous.mode;
        self.workspace_tool = session.previous.tool;
        self.left_tab = session.previous.tab;
        self.left_panel_layout = session.previous.layout;
        self.right_panel_layout = session.previous.right_layout;
        self.isolated_blind = session.previous.isolation;
        self.selection = session.previous.selection;
        self.canvas_dirty = true;
        true
    }
    fn begin_converter(&mut self, ctx: &Context<Self>, id: String) -> bool {
        if self.image_studio.apply.as_ref().is_some_and(|a| a.busy) {
            return false;
        }
        let Some(source) = self.image_catalog.iter().find(|e| e.id == id) else {
            return false;
        };
        self.image_studio.generation += 1;
        let token = self.image_studio.generation;
        let settings = self
            .image_studio
            .apply
            .as_ref()
            .and_then(|a| a.prepared.as_ref())
            .filter(|p| p.entry.source_image_id == id)
            .map(|p| p.entry.settings.clone())
            .unwrap_or(PaletteSettings {
                enabled_colors: (1..=10).collect(),
                dithering: DitherMode::None,
                alpha_threshold: 128,
                background: None,
                mappings: Vec::new(),
            });
        self.image_studio.converter = Some(Converter {
            token,
            source_id: id.clone(),
            name: format!("{} · prepared", source.name),
            settings,
            pixels: None,
            source_clusters: Vec::new(),
            dragging_source: None,
            preview: None,
            preview_url: String::new(),
            original_url: String::new(),
            zoom: 1.0,
            busy: true,
            error: None,
        });
        self.studio_modal = Some(StudioModal::Image);
        let project = self.target.project_id.to_string();
        let link = ctx.link().clone();
        wasm_bindgen_futures::spawn_local(async move {
            let result = RestClient.image_pixels(&project, &id).await;
            link.send_message(Msg::ImageStudio(ImageMsg::Pixels {
                project,
                token,
                result,
            }));
        });
        true
    }
    fn request_prepared(&mut self, ctx: &Context<Self>, id: String) {
        self.image_studio.generation += 1;
        let token = self.image_studio.generation;
        let Some(session) = self.image_studio.apply.as_mut() else {
            return;
        };
        session.token = token;
        session.busy = true;
        session.error = None;
        // Do not permit the previous immutable image to be applied while its replacement loads.
        session.prepared = None;
        session.overlay_canvas = None;
        session.projections.clear();
        let target = self.target.clone();
        let link = ctx.link().clone();
        wasm_bindgen_futures::spawn_local(async move {
            let result = RestClient
                .prepared_image(target.project_id.as_str(), &id)
                .await;
            link.send_message(Msg::ImageStudio(ImageMsg::Prepared {
                target,
                token,
                result,
            }));
        });
    }
    pub(super) fn image_studio_message(&mut self, ctx: &Context<Self>, msg: ImageMsg) -> bool {
        match msg {
            ImageMsg::Convert(id) => return self.begin_converter(ctx, id),
            ImageMsg::CancelConversion => self.cancel_conversion(),
            ImageMsg::Catalog {
                project,
                token,
                result,
            } => {
                if project != self.target.project_id.as_str()
                    || token != self.image_studio.catalog_generation
                {
                    return false;
                }
                match result {
                    Ok(entries) => self.image_studio.prepared = entries,
                    Err(e) => self.image_import_error = Some(api_error_message(&e)),
                }
            }
            ImageMsg::Pixels {
                project,
                token,
                result,
            } => {
                if project != self.target.project_id.as_str() {
                    return false;
                }
                let Some(c) = self
                    .image_studio
                    .converter
                    .as_mut()
                    .filter(|c| c.token == token)
                else {
                    return false;
                };
                c.busy = false;
                match result {
                    Ok(pixels) => {
                        c.original_url = rgba_url(&pixels);
                        let clusters =
                            extract_color_clusters(pixels.width, pixels.height, &pixels.rgba, 16);
                        c.pixels = Some(pixels);
                        c.refresh();
                        match clusters {
                            Ok(clusters) => c.source_clusters = clusters,
                            Err(error) => {
                                c.source_clusters.clear();
                                c.error = Some(error.to_string());
                            }
                        }
                    }
                    Err(e) => c.error = Some(api_error_message(&e)),
                }
            }
            ImageMsg::Color(color) => {
                let Some(c) = self.image_studio.converter.as_mut().filter(|c| !c.busy) else {
                    return false;
                };
                if c.settings.enabled_colors.contains(&color) {
                    c.settings.enabled_colors.retain(|v| *v != color);
                    c.settings
                        .mappings
                        .retain(|mapping| mapping.target_color != color);
                } else {
                    c.settings.enabled_colors.push(color);
                    c.settings.enabled_colors.sort();
                }
                c.refresh();
            }
            ImageMsg::BeginColorDrag(source_rgb) => {
                let Some(c) = self.image_studio.converter.as_mut().filter(|c| !c.busy) else {
                    return false;
                };
                c.dragging_source = Some(source_rgb);
            }
            ImageMsg::EndColorDrag => {
                let Some(c) = self.image_studio.converter.as_mut() else {
                    return false;
                };
                c.dragging_source = None;
            }
            ImageMsg::DropColor(target_color) => {
                let Some(c) = self.image_studio.converter.as_mut().filter(|c| !c.busy) else {
                    return false;
                };
                let Some(source_rgb) = c.dragging_source.take() else {
                    return false;
                };
                if !c.settings.enabled_colors.contains(&target_color) {
                    return false;
                }
                c.settings
                    .mappings
                    .retain(|mapping| mapping.source_rgb != source_rgb);
                c.settings.mappings.push(PaletteMapping {
                    source_rgb,
                    target_color,
                });
                c.refresh();
            }
            ImageMsg::MapColor(source_rgb, target_color) => {
                let Some(c) = self.image_studio.converter.as_mut().filter(|c| !c.busy) else {
                    return false;
                };
                c.settings
                    .mappings
                    .retain(|mapping| mapping.source_rgb != source_rgb);
                if let Some(target_color) =
                    target_color.filter(|color| c.settings.enabled_colors.contains(color))
                {
                    c.settings.mappings.push(PaletteMapping {
                        source_rgb,
                        target_color,
                    });
                }
                c.refresh();
            }
            ImageMsg::ResetMappings => {
                let Some(c) = self.image_studio.converter.as_mut().filter(|c| !c.busy) else {
                    return false;
                };
                if c.settings.mappings.is_empty() {
                    return false;
                }
                c.settings.mappings.clear();
                c.refresh();
            }
            ImageMsg::Dither(enabled) => {
                let Some(c) = self.image_studio.converter.as_mut().filter(|c| !c.busy) else {
                    return false;
                };
                c.settings.dithering = if enabled {
                    DitherMode::FloydSteinberg
                } else {
                    DitherMode::None
                };
                c.refresh();
            }
            ImageMsg::ConverterField(field, value) => {
                let Some(c) = self.image_studio.converter.as_mut().filter(|c| !c.busy) else {
                    return false;
                };
                match field {
                    ConverterField::Name => {
                        c.name = value;
                        return true;
                    }
                    ConverterField::Alpha => {
                        if let Ok(v) = value.parse::<u8>() {
                            c.settings.alpha_threshold = v;
                        }
                    }
                    ConverterField::Background => {
                        c.settings.background =
                            value.parse::<u8>().ok().filter(|v| (1..=10).contains(v))
                    }
                    ConverterField::Zoom => {
                        if let Ok(v) = value.parse::<f64>() {
                            c.zoom = v.clamp(0.125, 8.0);
                        }
                        return true;
                    }
                }
                c.refresh();
            }
            ImageMsg::SavePrepared => {
                let Some(c) = self
                    .image_studio
                    .converter
                    .as_mut()
                    .filter(|c| !c.busy && c.preview.is_some() && !c.name.trim().is_empty())
                else {
                    return false;
                };
                if !self.can_edit_timeline {
                    return false;
                }
                c.busy = true;
                let (token, id, name, settings) = (
                    c.token,
                    c.source_id.clone(),
                    c.name.clone(),
                    c.settings.clone(),
                );
                let project = self.target.project_id.to_string();
                let epoch = self.image_studio.epoch;
                let link = ctx.link().clone();
                wasm_bindgen_futures::spawn_local(async move {
                    let result = RestClient
                        .prepare_image(&project, &id, &name, &settings)
                        .await;
                    link.send_message(Msg::ImageStudio(ImageMsg::Saved {
                        project,
                        epoch,
                        token,
                        result,
                    }));
                });
            }
            ImageMsg::Saved {
                project,
                epoch,
                token,
                result,
            } => {
                if project != self.target.project_id.as_str() || epoch != self.image_studio.epoch {
                    return false;
                }
                let owns_converter = self
                    .image_studio
                    .converter
                    .as_ref()
                    .is_some_and(|c| c.token == token);
                match result {
                    Ok(prepared) => {
                        // An authoritative save may finish after Cancel. Reconcile the
                        // catalog, but never reopen or replace a newer converter/session.
                        self.image_studio
                            .prepared
                            .retain(|entry| entry.id != prepared.entry.id);
                        self.image_studio.prepared.push(prepared.entry.clone());
                        // Supersede stale list requests, but fetch their baseline too:
                        // the first catalog load may not have completed before this save.
                        self.load_prepared_catalog(ctx);
                        if owns_converter {
                            self.cancel_conversion();
                            if let Some(a) = &mut self.image_studio.apply {
                                a.overlay_canvas = indexed_canvas(&prepared.image);
                                a.prepared = Some(prepared);
                                a.refresh();
                                self.studio_modal = None;
                            }
                        }
                        self.push_toast(
                            "Prepared image saved. Original unchanged.".into(),
                            "success",
                        );
                    }
                    Err(e) => {
                        if let Some(c) = self
                            .image_studio
                            .converter
                            .as_mut()
                            .filter(|c| c.token == token)
                        {
                            c.busy = false;
                            c.error = Some(api_error_message(&e));
                        } else {
                            self.push_toast(api_error_message(&e), "warning");
                        }
                    }
                }
            }
            ImageMsg::Start(id) => {
                if self.image_studio.apply.is_some()
                    || !self.pending_commands.is_empty()
                    || !self.can_edit_timeline
                {
                    return false;
                }
                let previous = PreviousWorkspace {
                    mode: self.mode,
                    tool: self.workspace_tool,
                    tab: self.left_tab,
                    layout: self.left_panel_layout,
                    right_layout: self.right_panel_layout,
                    isolation: self.isolated_blind.clone(),
                    selection: self.selection.clone(),
                };
                let targets = self
                    .selected_entity_ids()
                    .iter()
                    .filter_map(|id| self.model.timeline().snapshot().entity(id))
                    .filter(|e| matches!(e.kind(), PlaceableEntityKind::Blind(_)))
                    .cloned()
                    .collect();
                self.image_studio.apply = Some(ApplySession {
                    token: 0,
                    target: self.target.clone(),
                    previous,
                    prepared: None,
                    overlay_canvas: None,
                    placement: default_placement(),
                    targets,
                    projections: BTreeMap::new(),
                    projector: None,
                    projector_key: None,
                    adjust: false,
                    lock_aspect: true,
                    opacity: 0.25,
                    busy: false,
                    error: None,
                    pending_command: None,
                    gesture: None,
                });
                self.entity_drag = None;
                self.blind_gesture = None;
                self.placement_drag = None;
                self.marquee_gesture = None;
                self.map_resize_gesture = None;
                self.drag_kind = None;
                self.key_locker_assignment = None;
                self.isolated_blind = None;
                self.studio_modal = None;
                self.left_tab = LeftTab::ApplyImage;
                self.left_panel_layout = PanelLayout::Docked;
                self.sync_image_selection();
                self.request_prepared(ctx, id);
            }
            ImageMsg::Choose(id) => {
                if self.image_studio.apply.as_ref().is_some_and(|a| !a.busy) {
                    self.request_prepared(ctx, id);
                }
            }
            ImageMsg::Prepared {
                target,
                token,
                result,
            } => {
                if target != self.target {
                    return false;
                }
                let Some(a) = self
                    .image_studio
                    .apply
                    .as_mut()
                    .filter(|a| a.token == token && a.target == target)
                else {
                    return false;
                };
                a.busy = false;
                match result {
                    Ok(p) => {
                        a.placement.height = a.placement.width * f64::from(p.image.height)
                            / f64::from(p.image.width);
                        a.overlay_canvas = indexed_canvas(&p.image);
                        a.prepared = Some(p);
                        a.fit(false);
                        a.refresh();
                    }
                    Err(e) => a.error = Some(api_error_message(&e)),
                }
            }
            ImageMsg::Cancel => {
                self.cancel_apply_image();
                self.cancel_conversion();
                self.studio_modal = None;
            }
            ImageMsg::Adjust(adjust) => {
                if let Some(a) = self.image_studio.apply.as_mut().filter(|a| !a.busy) {
                    a.adjust = adjust;
                    a.gesture = None;
                }
            }
            ImageMsg::LockAspect(lock) => {
                if let Some(a) = self.image_studio.apply.as_mut().filter(|a| !a.busy) {
                    a.lock_aspect = lock;
                }
            }
            ImageMsg::Fit(cover) => {
                if let Some(a) = self.image_studio.apply.as_mut().filter(|a| !a.busy) {
                    a.fit(cover);
                }
            }
            ImageMsg::Reset => {
                if let Some(a) = self.image_studio.apply.as_mut().filter(|a| !a.busy) {
                    a.placement = default_placement();
                    if let Some(p) = &a.prepared {
                        a.placement.height = a.placement.width * f64::from(p.image.height)
                            / f64::from(p.image.width);
                    }
                    a.refresh();
                }
            }
            ImageMsg::Sampling(area) => {
                if let Some(a) = self.image_studio.apply.as_mut().filter(|a| !a.busy) {
                    a.placement.sampling = if area {
                        ImageSampling::Area
                    } else {
                        ImageSampling::Nearest
                    };
                    a.refresh();
                }
            }
            ImageMsg::Transparency(erase) => {
                if let Some(a) = self.image_studio.apply.as_mut().filter(|a| !a.busy) {
                    a.placement.transparency = if erase {
                        ImageTransparency::Erase
                    } else {
                        ImageTransparency::Preserve
                    };
                    a.refresh();
                }
            }
            ImageMsg::Placement(field, value) => {
                let Some(a) = self.image_studio.apply.as_mut().filter(|a| !a.busy) else {
                    return false;
                };
                let Ok(v) = value.parse::<f64>() else {
                    return false;
                };
                if !v.is_finite() {
                    return false;
                }
                let ratio = a.placement.width / a.placement.height;
                match field {
                    PlacementField::X => a.placement.x = v,
                    PlacementField::Y => a.placement.y = v,
                    PlacementField::Width => {
                        a.placement.width = v.max(0.01);
                        if a.lock_aspect {
                            a.placement.height = a.placement.width / ratio;
                        }
                    }
                    PlacementField::Height => {
                        a.placement.height = v.max(0.01);
                        if a.lock_aspect {
                            a.placement.width = a.placement.height * ratio;
                        }
                    }
                    PlacementField::Pixelation => {
                        a.placement.pixelation = v.clamp(1.0, 1024.0) as u16
                    }
                    PlacementField::Resolution => {
                        a.placement.resolution = if v == 0.0 {
                            None
                        } else {
                            Some(v.clamp(1.0, 32.0) as u8)
                        }
                    }
                    PlacementField::Opacity => a.opacity = v.clamp(0.0, 1.0),
                }
                a.refresh();
            }
            ImageMsg::Apply => return self.submit_image(ctx),
            ImageMsg::Applied {
                target,
                token,
                attempt,
                command_id,
                result,
            } => {
                if target != self.target || attempt != self.connection_attempt {
                    return false;
                }
                let owns = self.image_studio.apply.as_ref().is_some_and(|a| {
                    a.token == token && a.pending_command.as_ref() == Some(&command_id)
                });
                let error = result.as_ref().err().cloned();
                self.handle_apply_response(ctx, command_id, result);
                if owns {
                    if let Some(error) = error {
                        if let Some(a) = &mut self.image_studio.apply {
                            a.busy = false;
                            a.pending_command = None;
                            a.error = Some(error);
                        }
                    } else {
                        self.cancel_apply_image();
                    }
                }
            }
        }
        self.canvas_dirty = true;
        true
    }
    fn sync_image_selection(&mut self) {
        if let Some(a) = &self.image_studio.apply {
            self.selection = Some(Selection::Entities(
                a.targets.iter().map(|e| e.id().clone()).collect(),
            ));
        }
    }
    fn image_apply_error(&self) -> Option<String> {
        let a = self.image_studio.apply.as_ref()?;
        if a.busy {
            return Some(if a.pending_command.is_some() {
                "Applying to the server… Cancel closes this session; it cannot retract an already sent command.".into()
            } else {
                "Image request pending…".into()
            });
        }
        if !self.can_edit_timeline || !matches!(self.rpc_state, RpcState::Online) {
            return Some("Connect with edit permission to apply.".into());
        }
        if let Some(error) = &a.error {
            return Some(error.clone());
        }
        if a.prepared.is_none() {
            return Some("Choose a prepared image.".into());
        }
        if a.targets.is_empty() {
            return Some("Select one or more Pools.".into());
        }
        if a.targets.iter().any(|e| {
            self.model.timeline().snapshot().entity(e.id()) != Some(e)
                || self.pending_entities.contains_key(e.id())
        }) {
            return Some("A selected Pool changed. Reselect it to refresh its snapshot.".into());
        }
        if a.projections.len() != a.targets.len() {
            return Some("Preview is not ready.".into());
        }
        None
    }
    fn submit_image(&mut self, ctx: &Context<Self>) -> bool {
        if self.image_apply_error().is_some() {
            return false;
        }
        let Some(rpc) = self.rpc.clone() else {
            return false;
        };
        let metadata = self.model.prepare_image_metadata(now_ms());
        let command_id = metadata.id.to_string();
        let a = self.image_studio.apply.as_mut().unwrap();
        let request = ApplyImageRequest {
            target: self.target.clone(),
            metadata,
            prepared_image_id: a.prepared.as_ref().unwrap().entry.id.clone(),
            placement: a.placement.clone(),
            targets: a.targets.clone(),
        };
        for entity in a.projections.values() {
            self.pending_entities.insert(
                entity.id().clone(),
                PendingEntity {
                    command_id: command_id.clone(),
                    preview: Some(entity.clone()),
                },
            );
        }
        self.pending_commands.insert(command_id.clone());
        a.busy = true;
        a.pending_command = Some(command_id.clone());
        a.gesture = None;
        let token = a.token;
        let target = self.target.clone();
        let attempt = self.connection_attempt;
        let link = ctx.link().clone();
        wasm_bindgen_futures::spawn_local(async move {
            let result = rpc.apply_image(request).await;
            link.send_message(Msg::ImageStudio(ImageMsg::Applied {
                target,
                token,
                attempt,
                command_id,
                result,
            }));
        });
        self.canvas_dirty = true;
        true
    }
    pub(super) fn image_projection(&self, entity: &PlaceableEntity) -> Option<PlaceableEntity> {
        self.image_studio
            .apply
            .as_ref()?
            .projections
            .get(entity.id())
            .cloned()
    }
    fn image_world_position(&self, event: &PointerEvent) -> Option<(f64, f64)> {
        let (x, y) = self.canvas_position(event)?;
        Some((
            (x - self.viewport.offset_x) / (CELL_SIZE * self.viewport.scale),
            (y - self.viewport.offset_y) / (CELL_SIZE * self.viewport.scale),
        ))
    }
    pub(super) fn image_pointer_down(&mut self, event: PointerEvent) -> bool {
        event.prevent_default();
        if let Some(root) = self.root_ref.cast::<HtmlElement>() {
            let _ = root.focus();
        }
        let Some(point) = self.image_world_position(&event) else {
            return false;
        };
        let Some(a) = self.image_studio.apply.as_mut().filter(|a| !a.busy) else {
            return false;
        };
        if event.button() != 0 {
            return false;
        }
        let p = &a.placement;
        let tolerance = 10.0 / (CELL_SIZE * self.viewport.scale);
        let handle = if a.adjust {
            [(false, false), (true, false), (false, true), (true, true)]
                .into_iter()
                .find(|(right, bottom)| {
                    (point.0 - (p.x + if *right { p.width } else { 0.0 })).abs() < tolerance
                        && (point.1 - (p.y + if *bottom { p.height } else { 0.0 })).abs()
                            < tolerance
                })
        } else {
            None
        };
        if a.adjust
            && handle.is_none()
            && !(point.0 >= p.x
                && point.0 <= p.x + p.width
                && point.1 >= p.y
                && point.1 <= p.y + p.height)
        {
            return false;
        }
        a.gesture = Some(ImageGesture {
            pointer: event.pointer_id(),
            start: point,
            current: point,
            placement: p.clone(),
            handle,
            additive: event.shift_key(),
        });
        self.capture_pointer(event.pointer_id());
        self.canvas_dirty = true;
        true
    }
    pub(super) fn image_pointer_move(&mut self, event: PointerEvent) -> bool {
        let Some(point) = self.image_world_position(&event) else {
            return false;
        };
        let Some(a) = self.image_studio.apply.as_mut().filter(|a| !a.busy) else {
            return false;
        };
        let Some(g) = a
            .gesture
            .as_mut()
            .filter(|g| g.pointer == event.pointer_id())
        else {
            return false;
        };
        g.current = point;
        if a.adjust {
            let (dx, dy) = (point.0 - g.start.0, point.1 - g.start.1);
            a.placement = crate::image_geometry::drag_placement(
                &g.placement,
                (dx, dy),
                g.handle,
                a.lock_aspect,
            );
            a.refresh();
        }
        self.canvas_dirty = true;
        true
    }
    pub(super) fn image_pointer_up(&mut self, event: PointerEvent, cancel: bool) -> bool {
        let Some(a) = self.image_studio.apply.as_mut() else {
            return false;
        };
        let Some(g) = a.gesture.take() else {
            return false;
        };
        if g.pointer != event.pointer_id() {
            a.gesture = Some(g);
            return false;
        }
        if cancel {
            a.placement = g.placement;
            a.refresh();
        } else if !a.adjust {
            let click = (g.current.0 - g.start.0)
                .abs()
                .max((g.current.1 - g.start.1).abs())
                * CELL_SIZE
                * self.viewport.scale
                < 4.0;
            let snapshot = self.model.timeline().snapshot();
            let hits = snapshot
                .entities()
                .iter()
                .filter(|e| matches!(e.kind(), PlaceableEntityKind::Blind(_)))
                .filter(|e| {
                    shape_world_points(e.origin(), e.shape())
                        .unwrap_or_default()
                        .iter()
                        .any(|p| {
                            let (x, y) = (f64::from(p.x), f64::from(p.y));
                            if click {
                                g.start.0 >= x
                                    && g.start.0 < x + 1.0
                                    && g.start.1 >= y
                                    && g.start.1 < y + 1.0
                            } else {
                                x + 1.0 > g.start.0.min(g.current.0)
                                    && x < g.start.0.max(g.current.0)
                                    && y + 1.0 > g.start.1.min(g.current.1)
                                    && y < g.start.1.max(g.current.1)
                            }
                        })
                })
                .cloned()
                .collect::<Vec<_>>();
            if !g.additive {
                a.targets.clear();
            }
            for hit in hits {
                if g.additive && click && a.targets.iter().any(|e| e.id() == hit.id()) {
                    a.targets.retain(|e| e.id() != hit.id());
                } else {
                    a.targets.retain(|e| e.id() != hit.id());
                    a.targets.push(hit);
                }
            }
            a.refresh();
        }
        self.release_pointer(event.pointer_id());
        self.sync_image_selection();
        self.canvas_dirty = true;
        true
    }
    pub(super) fn draw_image_overlay(&self, context: &CanvasRenderingContext2d) {
        let Some(a) = &self.image_studio.apply else {
            return;
        };
        let p = &a.placement;
        let (x, y, w, h) = (
            BOARD_ORIGIN + p.x * CELL_SIZE,
            BOARD_ORIGIN + p.y * CELL_SIZE,
            p.width * CELL_SIZE,
            p.height * CELL_SIZE,
        );
        context.save();
        if let Some(canvas) = &a.overlay_canvas {
            context.set_global_alpha(a.opacity);
            context.set_image_smoothing_enabled(false);
            let _ = context.draw_image_with_html_canvas_element_and_dw_and_dh(canvas, x, y, w, h);
        }
        context.set_global_alpha(1.0);
        context.set_stroke_style_str("#5de4c7");
        context.set_line_width(2.0 / self.viewport.scale);
        context.stroke_rect(x, y, w, h);
        if a.adjust {
            context.set_fill_style_str("#5de4c7");
            let r = 5.0 / self.viewport.scale;
            for (hx, hy) in [(x, y), (x + w, y), (x, y + h), (x + w, y + h)] {
                context.fill_rect(hx - r, hy - r, r * 2.0, r * 2.0);
            }
        }
        if !a.adjust {
            if let Some(g) = &a.gesture {
                context.set_global_alpha(0.15);
                context.set_fill_style_str("#5de4c7");
                let x = BOARD_ORIGIN + g.start.0.min(g.current.0) * CELL_SIZE;
                let y = BOARD_ORIGIN + g.start.1.min(g.current.1) * CELL_SIZE;
                let w = (g.start.0 - g.current.0).abs() * CELL_SIZE;
                let h = (g.start.1 - g.current.1).abs() * CELL_SIZE;
                context.fill_rect(x, y, w, h);
                context.set_global_alpha(1.0);
                context.stroke_rect(x, y, w, h);
            }
        }
        context.restore();
    }
}

impl App {
    pub(super) fn view_image_converter(&self, ctx: &Context<Self>) -> Option<Html> {
        let c = self.image_studio.converter.as_ref()?;
        let dimensions = c
            .pixels
            .as_ref()
            .map(|p| format!("{} × {} pixels", p.width, p.height))
            .unwrap_or_else(|| "Loading original pixels…".into());
        let size = c
            .pixels
            .as_ref()
            .map(|p| {
                format!(
                    "width:{}px;height:{}px",
                    f64::from(p.width) * c.zoom,
                    f64::from(p.height) * c.zoom
                )
            })
            .unwrap_or_default();
        Some(html! {
            <div class="image-converter">
                <header><div><h2>{"Palette converter"}</h2><p>{dimensions}{" · exact-size comparison"}</p></div></header>
                <div class="image-converter-body">
                    <div class="image-compare-wrap">
                        <label class="image-zoom">{"Comparison zoom"}<select onchange={ctx.link().callback(|e:Event|Msg::ImageStudio(ImageMsg::ConverterField(ConverterField::Zoom,e.target_unchecked_into::<HtmlSelectElement>().value())))}><option value="0.125" selected={c.zoom == 0.125}>{"12.5%"}</option><option value="0.25" selected={c.zoom == 0.25}>{"25%"}</option><option value="0.5" selected={c.zoom == 0.5}>{"50%"}</option><option value="1" selected={c.zoom == 1.0}>{"100%"}</option><option value="2" selected={c.zoom == 2.0}>{"200%"}</option><option value="4" selected={c.zoom == 4.0}>{"400%"}</option><option value="8" selected={c.zoom == 8.0}>{"800%"}</option></select></label>
                        <div class="image-compare-scroll"><div class="image-compare">
                            <figure><figcaption>{"ORIGINAL · server-decoded RGBA"}</figcaption><div class="image-checker">{if c.original_url.is_empty() { html!{<p>{"Loading…"}</p>} } else {html!{<img src={c.original_url.clone()} style={size.clone()} alt="Original image at comparison zoom" />}}}</div></figure>
                            <figure><figcaption>{"CONVERTED · fixed palette"}</figcaption><div class="image-checker">{if c.preview_url.is_empty() { html!{<p>{"No valid preview"}</p>} } else {html!{<img src={c.preview_url.clone()} style={size} alt="Converted image at the same dimensions and zoom" />}}}</div></figure>
                        </div></div>
                    </div>
                    <fieldset class="image-converter-controls" disabled={c.busy}>
                        <label>{"Prepared image name"}<input maxlength="120" value={c.name.clone()} oninput={ctx.link().callback(|e:InputEvent|Msg::ImageStudio(ImageMsg::ConverterField(ConverterField::Name,e.target_unchecked_into::<HtmlInputElement>().value())))} /></label>
                        <span>{"Enabled colors"}</span><div class="image-color-grid">{for (1..=10).map(|color|{let enabled=c.settings.enabled_colors.contains(&color);html!{<button class={classes!(enabled.then_some("active"))} style={format!("--brush-color:{}",blind_color(color))} aria-label={format!("Enable color {color}")} aria-pressed={enabled.to_string()} onclick={ctx.link().callback(move |_|Msg::ImageStudio(ImageMsg::Color(color)))}><span></span>{color}</button>}})}</div>
                        <label class="image-checkbox"><input type="checkbox" checked={c.settings.dithering == DitherMode::FloydSteinberg} onchange={ctx.link().callback(|e:Event|Msg::ImageStudio(ImageMsg::Dither(e.target_unchecked_into::<HtmlInputElement>().checked())))} />{"Floyd–Steinberg dithering"}</label>
                        <label>{"Alpha threshold (0–255)"}<input type="number" min="0" max="255" value={c.settings.alpha_threshold.to_string()} onchange={ctx.link().callback(|e:Event|Msg::ImageStudio(ImageMsg::ConverterField(ConverterField::Alpha,e.target_unchecked_into::<HtmlInputElement>().value())))} /></label>
                        <label>{"Background"}<select onchange={ctx.link().callback(|e:Event|Msg::ImageStudio(ImageMsg::ConverterField(ConverterField::Background,e.target_unchecked_into::<HtmlSelectElement>().value())))}><option value="0" selected={c.settings.background.is_none()}>{"Transparent"}</option>{for c.settings.enabled_colors.iter().map(|color|html!{<option value={color.to_string()} selected={c.settings.background == Some(*color)}>{format!("Color {color}")}</option>})}</select></label>
                        <details class="palette-mapping-details">
                            <summary>{"Advanced palette mapping"}<code>{format!("{} mapped", c.settings.mappings.len())}</code></summary>
                            <div class="palette-mapping-body">
                                <button type="button" class="palette-mapping-reset" disabled={c.busy || c.settings.mappings.is_empty()} onclick={ctx.link().callback(|_| Msg::ImageStudio(ImageMsg::ResetMappings))}>{"Reset mappings"}</button>
                                <section class="palette-unassigned">
                                    <h3>{"Unassigned source colors"}</h3>
                                    <p>{"Drag a dominant source color into a game color, or choose a target with the select."}</p>
                                    <div class="palette-chip-list">
                                        {for c.source_clusters.iter().filter(|cluster| !c.settings.mappings.iter().any(|mapping| mapping.source_rgb == cluster.rgb)).map(|cluster| {
                                            palette_mapping_chip(ctx, cluster.rgb, cluster.pixel_count.to_string(), None, &c.settings.enabled_colors, c.busy)
                                        })}
                                    </div>
                                </section>
                                <div class="palette-drop-groups">
                                    {for c.settings.enabled_colors.iter().map(|color| {
                                        let target_color = *color;
                                        let has_mappings = c.settings.mappings.iter().any(|mapping| mapping.target_color == target_color);
                                        html! {
                                            <section
                                                key={target_color}
                                                class="palette-drop-group"
                                                ondragover={Callback::from(move |event: DragEvent| {
                                                    event.prevent_default();
                                                    if let Some(data_transfer) = event.data_transfer() {
                                                        data_transfer.set_drop_effect("move");
                                                    }
                                                })}
                                                ondrop={ctx.link().callback(move |event: DragEvent| {
                                                    event.prevent_default();
                                                    Msg::ImageStudio(ImageMsg::DropColor(target_color))
                                                })}
                                            >
                                                <header><span class="palette-target-swatch" style={format!("--target-color:{}", blind_color(target_color))}></span><strong>{format!("Game color {target_color}")}</strong></header>
                                                <div class="palette-chip-list">
                                                    {if has_mappings {
                                                        html! { <>{for c.settings.mappings.iter().filter(|mapping| mapping.target_color == target_color).map(|mapping| {
                                                            let count = c.source_clusters.iter().find(|cluster| cluster.rgb == mapping.source_rgb).map(|cluster| cluster.pixel_count.to_string()).unwrap_or_else(|| "—".into());
                                                            palette_mapping_chip(ctx, mapping.source_rgb, count, Some(target_color), &c.settings.enabled_colors, c.busy)
                                                        })}</> }
                                                    } else {
                                                        html! { <p class="palette-drop-empty">{"Drop source color here"}</p> }
                                                    }}
                                                </div>
                                            </section>
                                        }
                                    })}
                                </div>
                            </div>
                        </details>
                        <p class="image-studio-note">{"Preview is local. Saving recomputes from the original on the server and creates a new immutable prepared image."}</p>
                    </fieldset>
                </div>
                {c.error.as_ref().map(|e|html!{<p role="alert" class="image-import-error">{e}</p>}).unwrap_or_default()}
                <footer><p>{"Cancel keeps the uploaded original in the project library. No Pool changes are made."}</p><div class="converter-actions"><button onclick={ctx.link().callback(|_|Msg::ImageStudio(ImageMsg::CancelConversion))}>{"Cancel conversion"}</button><button class="converter-save" disabled={c.busy || c.preview.is_none() || c.name.trim().is_empty() || !self.can_edit_timeline} onclick={ctx.link().callback(|_|Msg::ImageStudio(ImageMsg::SavePrepared))}>{if c.busy {"Working…"} else {"Save prepared image"}}</button></div></footer>
            </div>
        })
    }
    pub(super) fn view_prepared_images(&self, ctx: &Context<Self>) -> Html {
        html! { <section class="prepared-image-library"><div class="studio-section-heading"><span>{"PREPARED IMAGES"}</span><code>{self.image_studio.prepared.len()}</code></div>
            {if self.image_studio.prepared.is_empty() {html!{<p class="image-studio-note">{"Convert an original to make a palette-ready image. Each saved preparation is immutable."}</p>}} else {html!{<div class="image-catalog">{for self.image_studio.prepared.iter().map(|entry|{
                let id=entry.id.clone(); let source=entry.source_image_id.clone();
                let thumbnail=format!("/api/projects/{}/prepared-images/{}/thumbnail",self.target.project_id,entry.id);
                html!{<article class="image-catalog-card"><img src={thumbnail} alt={entry.name.clone()} loading="lazy" decoding="async" /><div><strong>{entry.name.clone()}</strong><code>{format!("{} × {} · palette v{}",entry.width,entry.height,entry.palette_version)}</code><small>{format!("{} colors · {:?}",entry.settings.enabled_colors.len(),entry.settings.dithering)}</small><div class="image-card-actions"><button disabled={!self.can_edit_timeline || self.image_studio.apply.is_some() || !self.pending_commands.is_empty()} onclick={ctx.link().callback(move |_|Msg::ImageStudio(ImageMsg::Start(id.clone())))}>{"Apply Image"}</button><button onclick={ctx.link().callback(move |_|Msg::ImageStudio(ImageMsg::Convert(source.clone())))}>{"New conversion"}</button></div></div></article>}
            })}</div>}}}
        </section>}
    }
    pub(super) fn view_apply_image_panel(&self, ctx: &Context<Self>) -> Html {
        let Some(a) = &self.image_studio.apply else {
            return Html::default();
        };
        let selected = a
            .prepared
            .as_ref()
            .map(|p| p.entry.id.clone())
            .unwrap_or_default();
        html! { <div class="apply-image-panel">
            <div class="section-title"><span>{"APPLY IMAGE"}</span><code>{format!("{} POOLS",a.targets.len())}</code></div>
            <p class="image-studio-note">{"Select Pool: click, Shift-click to toggle, or drag a marquee. Only Pools are eligible. Nothing changes until Apply."}</p>
            <fieldset disabled={a.busy}>
                <label>{"Prepared image"}<select onchange={ctx.link().callback(|e:Event|Msg::ImageStudio(ImageMsg::Choose(e.target_unchecked_into::<HtmlSelectElement>().value())))}><option value="" disabled=true selected={selected.is_empty()}>{"Choose prepared image"}</option>{for self.image_studio.prepared.iter().map(|p|html!{<option value={p.id.clone()} selected={selected == p.id}>{p.name.clone()}</option>})}</select></label>
                {a.prepared.as_ref().map(|p|{let source=p.entry.source_image_id.clone();html!{<button onclick={ctx.link().callback(move |_|Msg::ImageStudio(ImageMsg::Convert(source.clone())))}>{"Edit palette as new preparation…"}</button>}}).unwrap_or_default()}
                <div class="image-placement-grid">{for [(PlacementField::X,"X",a.placement.x),(PlacementField::Y,"Y",a.placement.y),(PlacementField::Width,"Width",a.placement.width),(PlacementField::Height,"Height",a.placement.height)].into_iter().map(|(field,label,value)|html!{<label>{label}<input type="number" step="0.1" value={format!("{value:.3}")} onchange={ctx.link().callback(move |e:Event|Msg::ImageStudio(ImageMsg::Placement(field,e.target_unchecked_into::<HtmlInputElement>().value())))} /></label>})}</div>
                <label class="image-checkbox"><input type="checkbox" checked={a.lock_aspect} onchange={ctx.link().callback(|e:Event|Msg::ImageStudio(ImageMsg::LockAspect(e.target_unchecked_into::<HtmlInputElement>().checked())))} />{"Lock aspect ratio"}</label>
                <div class="image-fit-actions"><button disabled={a.targets.is_empty()} onclick={ctx.link().callback(|_|Msg::ImageStudio(ImageMsg::Fit(false)))}>{"Fit"}</button><button disabled={a.targets.is_empty()} onclick={ctx.link().callback(|_|Msg::ImageStudio(ImageMsg::Fit(true)))}>{"Cover"}</button><button onclick={ctx.link().callback(|_|Msg::ImageStudio(ImageMsg::Reset))}>{"Reset"}</button></div>
                <label>{"Sampling"}<select onchange={ctx.link().callback(|e:Event|Msg::ImageStudio(ImageMsg::Sampling(e.target_unchecked_into::<HtmlSelectElement>().value()=="area")))}><option value="nearest" selected={a.placement.sampling == ImageSampling::Nearest}>{"Nearest neighbor"}</option><option value="area" selected={a.placement.sampling == ImageSampling::Area}>{"Area average"}</option></select></label>
                <label>{"Pixelation block"}<input type="number" min="1" max="1024" value={a.placement.pixelation.to_string()} onchange={ctx.link().callback(|e:Event|Msg::ImageStudio(ImageMsg::Placement(PlacementField::Pixelation,e.target_unchecked_into::<HtmlInputElement>().value())))} /></label>
                <label>{"Pool resolution"}<select onchange={ctx.link().callback(|e:Event|Msg::ImageStudio(ImageMsg::Placement(PlacementField::Resolution,e.target_unchecked_into::<HtmlSelectElement>().value())))}><option value="0" selected={a.placement.resolution.is_none()}>{"Keep each Pool's resolution"}</option>{for (1..=32).map(|v|html!{<option value={v.to_string()} selected={a.placement.resolution == Some(v)}>{format!("Set {v} px / cell")}</option>})}</select></label>
                <label>{"Transparent pixels"}<select onchange={ctx.link().callback(|e:Event|Msg::ImageStudio(ImageMsg::Transparency(e.target_unchecked_into::<HtmlSelectElement>().value()=="erase")))}><option value="preserve" selected={a.placement.transparency == ImageTransparency::Preserve}>{"Preserve existing pixels"}</option><option value="erase" selected={a.placement.transparency == ImageTransparency::Erase}>{"Erase existing pixels"}</option></select></label>
                <label>{"Overlay opacity (preview only)"}<input type="range" min="0" max="1" step="0.05" value={a.opacity.to_string()} oninput={ctx.link().callback(|e:InputEvent|Msg::ImageStudio(ImageMsg::Placement(PlacementField::Opacity,e.target_unchecked_into::<HtmlInputElement>().value())))} /></label>
            </fieldset>
            {self.image_apply_error().map(|e|html!{<p role="status" class="image-import-error">{e}</p>}).unwrap_or_else(||html!{<p class="image-studio-note">{"Preview matches the projected Pool pixels. Apply creates one undoable command."}</p>})}
        </div> }
    }
    pub(super) fn view_apply_image_dock(&self, ctx: &Context<Self>) -> Html {
        let Some(a) = &self.image_studio.apply else {
            return Html::default();
        };
        html! {<div class="canvas-toolbox contextual-canvas-tools apply-image-dock"><button class={classes!((!a.adjust).then_some("active"))} disabled={a.busy} onclick={ctx.link().callback(|_|Msg::ImageStudio(ImageMsg::Adjust(false)))}>{"Select Pool"}</button><button class={classes!(a.adjust.then_some("active"))} disabled={a.busy} onclick={ctx.link().callback(|_|Msg::ImageStudio(ImageMsg::Adjust(true)))}>{"Adjust Image"}</button><button onclick={ctx.link().callback(|_|Msg::ImageStudio(ImageMsg::Cancel))}>{"Cancel"}</button><button class="tool-button-colored" style="--tool-button-bg:var(--accent);--tool-button-color:var(--surface-0);--tool-button-hover-bg:var(--accent-strong);--tool-button-hover-color:var(--surface-0)" disabled={self.image_apply_error().is_some()} onclick={ctx.link().callback(|_|Msg::ImageStudio(ImageMsg::Apply))}>{"Apply"}</button></div>}
    }
}

use std::{cell::RefCell, path::Path, rc::Rc};

use thaum_renderer_domain::{
    title_hotspot, vivid_flash_is_lit, Cell, CellGraphic, CellGroup, CellGroupIntakeBehavior,
    CellPoint, CellWeight, GizmoBar, GizmoClickOutcome, GizmoKind, GizmoState, Hotspot, Module,
    ModulePointerButton, ModulePointerEvent, ModuleRect, PanelChrome, PersistedModuleUiState,
    Scrollbar, ScrollbarSpan, ScrollState, SidedGraphic, SpriteGraphic, UiColorRole, UiPalette,
    WorldPoint,
};

use crate::tool_state::{HandState, PaintHand, PaintTarget, ToolState};

const GLYPH_SECTIONS_TEXT: &str = thaum_renderer_domain::MONOTHAUM_ATLAS_V3_SECTIONS_TEXT;
const SUPPORTED_SPRITE_PATHS: &[&str] = &["proofs/channel-bands.png", "proofs/grass.png"];
const RECENT_CAPACITY: usize = 10;
const RECENT_PITCH: i32 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
struct GlyphSection {
    title: String,
    glyphs: Vec<char>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SpriteOption {
    label: String,
    path: String,
}

/// One sided declaration option: a declaration file under the asset root's
/// `sided/` folder, parsed once at boot. The declaration content paints onto
/// cells; the file stays the authoring source of truth.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidedOption {
    pub label: String,
    pub path: String,
    pub graphic: SidedGraphic,
}

/// Loads every parseable `sided/*.json` declaration under the asset root.
/// Broken files are skipped rather than blocking the picker; the declaration
/// format rejects them loudly at authoring time.
pub fn load_sided_options(asset_root: &Path) -> Vec<SidedOption> {
    let folder = asset_root.join("sided");
    let Ok(entries) = std::fs::read_dir(&folder) else {
        return Vec::new();
    };
    let mut options = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Ok(graphic) = thaum_renderer_domain::load_sided_declaration(&path) else {
            continue;
        };
        let file = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let label = file.strip_suffix(".json").unwrap_or(&file).to_string();
        options.push(SidedOption {
            label,
            path: format!("sided/{file}"),
            graphic,
        });
    }
    options.sort_by(|a, b| a.label.cmp(&b.label));
    options
}

/// One content row of the picker, top to bottom. Materialized into cells
/// only for the rows currently visible inside the module bounds so content
/// larger than the panel crops and scrolls instead of forcing panel size.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ContentRow {
    SectionTitle(String),
    /// The painter's clear character. It displays as a dot but equips a real
    /// space glyph, which every paint tool resolves through `write_cell`.
    Clear,
    Glyphs(Vec<char>),
    Sprite(usize),
    Sided(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LayoutHit {
    Clear {
        x0: i32,
        x1: i32,
        y: i32,
    },
    Glyph {
        x: i32,
        y: i32,
        graphic: CellGraphic,
    },
    Sprite {
        x0: i32,
        x1: i32,
        y: i32,
        graphic: CellGraphic,
    },
    Sided {
        x0: i32,
        x1: i32,
        y: i32,
        graphic: CellGraphic,
    },
    Recent {
        x0: i32,
        x1: i32,
        y: i32,
        graphic: CellGraphic,
    },
}

fn parse_supported_glyph_sections(text: &str) -> Vec<GlyphSection> {
    let mut sections = Vec::new();
    let mut pending_title: Option<String> = None;

    for line in text.lines() {
        if line.is_empty() {
            continue;
        }

        // A section title may sit on its own line (`symbols:`) or be glued to
        // its glyph line (`borders:━┃…`) — the renderer's sprite-section
        // parser accepts both, and this picker must stay in lockstep with it
        // or whole sections silently disappear from the glyph list. Only an
        // ASCII-identifier head counts as a title, so glyph lines that merely
        // contain ':' (like the dots section's `.,:;…`) still parse as
        // glyphs of the current section.
        if let Some((head, tail)) = line.split_once(':') {
            let is_title =
                !head.is_empty() && head.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            if is_title {
                if tail.is_empty() {
                    pending_title = Some(head.trim().to_string());
                } else {
                    sections.push(GlyphSection {
                        title: head.trim().to_string(),
                        glyphs: tail.chars().collect(),
                    });
                }
                continue;
            }
        }

        let Some(title) = pending_title.take() else {
            continue;
        };
        sections.push(GlyphSection {
            title,
            glyphs: line.chars().collect(),
        });
    }

    sections
}

fn supported_glyph_sections() -> Vec<GlyphSection> {
    parse_supported_glyph_sections(GLYPH_SECTIONS_TEXT)
}

fn supported_sprites() -> Vec<SpriteOption> {
    SUPPORTED_SPRITE_PATHS
        .iter()
        .map(|path| SpriteOption {
            path: (*path).to_string(),
            label: path
                .rsplit('/')
                .next()
                .unwrap_or(path)
                .trim_end_matches(".png")
                .to_string(),
        })
        .collect()
}

fn format_section_title(title: &str) -> String {
    title.to_ascii_uppercase()
}

fn display_glyph(glyph: char) -> char {
    if glyph == ' ' {
        '·'
    } else {
        glyph
    }
}

fn assignment_role(tool_state: &ToolState, graphic: &CellGraphic) -> UiColorRole {
    let is_left = tool_state.left_hand.graphic == *graphic;
    let is_right = tool_state.right_hand.graphic == *graphic;
    if is_left && is_right {
        UiColorRole::Vivid
    } else if is_left {
        UiColorRole::LeftHand
    } else if is_right {
        UiColorRole::RightHand
    } else {
        UiColorRole::Medium
    }
}

fn assignment_indicator(tool_state: &ToolState, graphic: &CellGraphic) -> char {
    let is_left = tool_state.left_hand.graphic == *graphic;
    let is_right = tool_state.right_hand.graphic == *graphic;
    if is_left && is_right {
        '◆'
    } else if is_left {
        'L'
    } else if is_right {
        'R'
    } else {
        ' '
    }
}

fn preview_hand<'a>(tool_state: &'a ToolState, graphic: &CellGraphic) -> Option<&'a HandState> {
    let is_left = tool_state.left_hand.graphic == *graphic;
    let is_right = tool_state.right_hand.graphic == *graphic;
    match (is_left, is_right) {
        (true, false) => Some(&tool_state.left_hand),
        (false, true) => Some(&tool_state.right_hand),
        (true, true) => Some(&tool_state.left_hand),
        (false, false) => None,
    }
}

fn preview_color(
    tool_state: &ToolState,
    graphic: &CellGraphic,
    palette: &UiPalette,
) -> thaum_renderer_domain::CellColor {
    preview_hand(tool_state, graphic)
        .map(|hand| hand.color.to_cell_color())
        .unwrap_or_else(|| palette.get(assignment_role(tool_state, graphic)))
}

/// Picker preview weights per J's tooling-weight rule: the module previews
/// the weight(s) the hands are actually tooling, never a static default.
/// - exactly one hand targeted on Image: that hand's weight, static
/// - both hands on Image, or neither: alternate left→right weights on the
///   same vivid-flash clock the selection shader blinks with, so a user
///   sees both candidate weights while the palette is up
pub fn preview_weights(tool_state: &ToolState) -> (CellWeight, Option<CellWeight>) {
    let hand_weight = |hand: &HandState| CellWeight::from_index_clamped(hand.weight_index as i32);
    match (
        tool_state.left_hand.target == PaintTarget::Image,
        tool_state.right_hand.target == PaintTarget::Image,
    ) {
        (true, false) => (hand_weight(&tool_state.left_hand), None),
        (false, true) => (hand_weight(&tool_state.right_hand), None),
        (true, true) | (false, false) => (
            hand_weight(&tool_state.left_hand),
            Some(hand_weight(&tool_state.right_hand)),
        ),
    }
}

pub struct GraphicPickerModule {
    id: String,
    rect: ModuleRect,
    tool_state: Rc<RefCell<ToolState>>,
    glyph_sections: Vec<GlyphSection>,
    sprites: Vec<SpriteOption>,
    sided: Vec<SidedOption>,
    palette: UiPalette,
    gizmos: GizmoBar,
    gizmo_state: GizmoState,
    hidden: bool,
    /// Row-scroll state: offset over the category rows, pinned RECENT block
    /// at the screen top shrinks the scrollable middle.
    scroll: ScrollState,
    /// Most-recent-first recall list, observed at draw time.
    recent: RefCell<Vec<CellGraphic>>,
    /// Shared frame-breath clock: the event loop writes the renderer's
    /// breath each frame so weight alternation blinks in step with the
    /// selection shader's vivid-flash cadence.
    breath: Rc<std::cell::Cell<i32>>,
    /// Vertical scrollbar over the scrolling category rows, overlaid on the
    /// rightmost content column; drag state lives here, geometry re-synced
    /// on every pointer/wheel event so panel resizes keep it honest.
    scrollbar: Scrollbar,
}

impl GraphicPickerModule {
    pub fn new(
        id: impl Into<String>,
        rect: ModuleRect,
        tool_state: Rc<RefCell<ToolState>>,
    ) -> Self {
        let mut picker = Self {
            id: id.into(),
            rect,
            tool_state,
            glyph_sections: supported_glyph_sections(),
            sprites: supported_sprites(),
            sided: Vec::new(),
            palette: UiPalette::default(),
            gizmos: GizmoBar::standard(),
            gizmo_state: GizmoState::new(),
            hidden: false,
            scroll: ScrollState::new(),
            recent: RefCell::new(Vec::new()),
            breath: Rc::new(std::cell::Cell::new(0)),
            scrollbar: Scrollbar::new(ScrollbarSpan::Vertical {
                x: 0,
                top: 0,
                bottom: 0,
            }),
        };
        picker.scrollbar.set_span(picker.scrollbar_span());
        picker
    }

    /// Shares the frame-breath clock with this picker. The event loop sets
    /// the renderer's breath each frame before module draw.
    pub fn with_breath_clock(mut self, breath: Rc<std::cell::Cell<i32>>) -> Self {
        self.breath = breath;
        self
    }

    /// The weight every preview cell shows this frame: the hands' tooling
    /// weight (see `preview_weights`), alternating on the vivid-flash phase
    /// when both candidate weights apply.
    fn preview_weight(&self, tool_state: &ToolState) -> CellWeight {
        let (lit_weight, alt_weight) = preview_weights(tool_state);
        if vivid_flash_is_lit(self.breath.get()) {
            lit_weight
        } else {
            alt_weight.unwrap_or(lit_weight)
        }
    }

    pub fn with_palette(mut self, palette: UiPalette) -> Self {
        self.palette = palette;
        self
    }

    /// Sets the sided declaration options (loaded from the asset root by the
    /// booting program). Empty by default so tests run without assets.
    pub fn with_sided_options(mut self, sided: Vec<SidedOption>) -> Self {
        self.sided = sided;
        self
    }

    fn build_scroll_rows(&self) -> Vec<ContentRow> {
        let (content_width, _) = PanelChrome::content_size(self.rect);
        let mut columns = content_width.max(1) as usize;
        // J: when the bar is present the rightmost content column belongs
        // to it, so the character rows wrap one column short — no
        // character sits under the bar and none becomes unreachable
        // without resizing the module.
        if self.scrollbar_visible() && columns > 1 {
            columns -= 1;
        }
        self.build_rows_with_columns(columns)
    }

    /// Whether the vertical scrollbar shows this frame: the row list built
    /// at the full content width overflows the viewport. The single
    /// decision point `build_scroll_rows` and the pinned-block draw share,
    /// so row wrapping and the recent row always agree with the bar.
    fn scrollbar_visible(&self) -> bool {
        let (content_width, _) = PanelChrome::content_size(self.rect);
        self.build_rows_with_columns(content_width.max(1) as usize).len()
            > self.available_scroll_rows()
    }

    fn build_rows_with_columns(&self, columns: usize) -> Vec<ContentRow> {
        let mut rows = vec![
            ContentRow::SectionTitle("character".to_string()),
            ContentRow::Clear,
        ];
        for section in &self.glyph_sections {
            rows.push(ContentRow::SectionTitle(section.title.clone()));
            for glyph_row in section.glyphs.chunks(columns) {
                rows.push(ContentRow::Glyphs(glyph_row.to_vec()));
            }
        }
        // Source-of-truth from J: sprites are a standard category like the
        // ascii sections, not a separated pinned block.
        rows.push(ContentRow::SectionTitle("sprites".to_string()));
        for index in 0..self.sprites.len() {
            rows.push(ContentRow::Sprite(index));
        }
        rows.push(ContentRow::SectionTitle("sided".to_string()));
        for index in 0..self.sided.len() {
            rows.push(ContentRow::Sided(index));
        }
        rows
    }

    /// Middle rows available for scrolling once the pinned RECENT block
    /// takes its share of the viewport.
    fn available_scroll_rows(&self) -> usize {
        let (_, content_height) = PanelChrome::content_size(self.rect);
        ScrollState::available_rows(
            content_height.max(0) as usize,
            self.recent_block_height().max(0) as usize,
            0,
        )
    }

    /// Largest valid row offset over the whole category list.
    fn max_scroll_rows(&self) -> usize {
        ScrollState::max_offset(self.build_scroll_rows().len(), self.available_scroll_rows())
    }

    /// Pinned block at the screen-top end of the module: RECENT header plus
    /// one row of recent graphics. Stays put while the list scrolls under.
    fn recent_block_height(&self) -> i32 {
        2
    }

    /// Where the vertical scrollbar lives this frame: the rightmost content
    /// column, spanning the scrollable middle between the pinned RECENT
    /// block (screen-top, larger y) and the bottom content edge.
    fn scrollbar_span(&self) -> ScrollbarSpan {
        let (content_x, content_y) = PanelChrome::content_origin();
        let (content_width, content_height) = PanelChrome::content_size(self.rect);
        let top = content_y + content_height - self.recent_block_height() - 1;
        let bottom = (top - self.available_scroll_rows() as i32 + 1).max(content_y);
        ScrollbarSpan::Vertical {
            x: content_x + content_width - 1,
            top,
            bottom,
        }
    }

    /// Press on the bar: thumb grab or track page-jump. Returns whether the
    /// bar consumed the press.
    fn scrollbar_press(&mut self, local_x: i32, local_y: i32) -> bool {
        let content = self.build_scroll_rows().len();
        let viewport = self.available_scroll_rows();
        let max = ScrollState::max_offset(content, viewport);
        let mut offset = self.scroll.offset();
        let consumed = self.scrollbar.on_down(local_x, local_y, content, viewport, &mut offset);
        self.scroll.set_offset(offset, max);
        consumed
    }

    /// Pointer move during a live bar drag: re-map the grab point.
    fn scrollbar_drag(&mut self, local_x: i32, local_y: i32) -> bool {
        let content = self.build_scroll_rows().len();
        let viewport = self.available_scroll_rows();
        let max = ScrollState::max_offset(content, viewport);
        let mut offset = self.scroll.offset();
        let moved = self.scrollbar.on_drag(local_x, local_y, content, viewport, &mut offset);
        self.scroll.set_offset(offset, max);
        moved
    }

    /// Most-recent-first recall list of the last assigned graphics. Fed by
    /// draw-time observation of both hands so every assignment path lands
    /// here, deduped, capped at `RECENT_CAPACITY`.
    fn note_recent(&self, graphic: &CellGraphic) {
        let mut recent = self.recent.borrow_mut();
        recent.retain(|existing| existing != graphic);
        recent.insert(0, graphic.clone());
        recent.truncate(RECENT_CAPACITY);
    }

    fn observe_hand_recents(&self, state: &ToolState) {
        for graphic in [&state.left_hand.graphic, &state.right_hand.graphic] {
            self.note_recent(graphic);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_pinned_recent_block(
        &self,
        content_x: i32,
        content_y: i32,
        content_width: i32,
        content_height_param: i32,
        cells: &mut Vec<Cell>,
        hits: &mut Vec<LayoutHit>,
    ) {
        let max_x = content_x + content_width - 1;
        let draw_text = |text: &str, y: i32, role: UiColorRole, cells: &mut Vec<Cell>| {
            for (index, glyph) in text.chars().enumerate() {
                let x = content_x + index as i32;
                if x > max_x {
                    break;
                }
                cells.push(Cell {
                    position: CellPoint { x, y, z: 0 },
                    graphic: CellGraphic::Glyph(glyph),
                    color: self.palette.get(role),
                    weight: CellWeight::from_index_clamped(2),
                    ..Cell::default()
                });
            }
        };
        let header_y = content_y + content_height_param - 1;
        let row_y = header_y - 1;
        draw_text("RECENT", header_y, UiColorRole::Bright, cells);
        let recent = self.recent.borrow();
        for (index, graphic) in recent.iter().enumerate() {
            let x = content_x + (index as i32) * RECENT_PITCH;
            if x > max_x {
                break;
            }
            let state = self.tool_state.borrow();
            let _role = assignment_role(&state, graphic);
            cells.push(Cell {
                position: CellPoint { x, y: row_y, z: 0 },
                graphic: match graphic {
                    CellGraphic::Glyph(' ') => CellGraphic::Glyph('·'),
                    other => other.clone(),
                },
                color: preview_color(&state, graphic, &self.palette),
                weight: self.preview_weight(&state),
                ..Cell::default()
            });
            hits.push(LayoutHit::Recent {
                x0: x,
                x1: (x + RECENT_PITCH - 1).min(max_x),
                y: row_y,
                graphic: graphic.clone(),
            });
        }
    }

    fn build_layout(&self) -> (Vec<Cell>, Vec<LayoutHit>) {
        let (content_x, content_y) = PanelChrome::content_origin();
        let (content_width, content_height) = PanelChrome::content_size(self.rect);
        let mut cells: Vec<Cell> = if self.gizmo_state.is_seamless() {
            Vec::new()
        } else {
            self.gizmo_state
                .decorate_panel_chrome(
                    PanelChrome::new(self.rect, &self.palette)
                        .with_title("GRAPHICS")
                        .with_title_start_x(self.gizmos.title_start_x()),
                    &self.palette,
                )
                .cells()
        };
        if self.gizmo_state.should_draw_gizmo_bar() {
            cells.extend(
                self.gizmos
                    .cells(self.rect, &self.gizmo_state, &self.palette),
            );
        }

        let state = self.tool_state.borrow();
        self.observe_hand_recents(&state);
        let mut hits = Vec::new();
        let max_x = content_x + content_width - 1;

        let draw_text = |text: &str, x: i32, y: i32, role: UiColorRole, cells: &mut Vec<Cell>| {
            for (index, glyph) in text.chars().enumerate() {
                let x = x + index as i32;
                if x > max_x {
                    break;
                }
                cells.push(Cell {
                    position: CellPoint { x, y, z: 0 },
                    graphic: CellGraphic::Glyph(glyph),
                    color: self.palette.get(role),
                    weight: CellWeight::from_index_clamped(2),
                    ..Cell::default()
                });
            }
        };

        self.draw_pinned_recent_block(
            content_x,
            content_y,
            content_width,
            content_height,
            &mut cells,
            &mut hits,
        );

        // Flat2d panels render larger local y higher on screen, so the
        // screen-top of the content is the largest-y row: walk row_index 0
        // (screen-topmost scroll row) downward from just under RECENT.
        let available = self.available_scroll_rows();
        let rows = self.build_scroll_rows();
        let scroll = self.scroll.offset().min(self.max_scroll_rows());
        let first_y = content_y + content_height - self.recent_block_height() - 1;
        for (row_index, row) in rows.iter().skip(scroll).take(available).enumerate() {
            let y = first_y - row_index as i32;
            match row {
                ContentRow::SectionTitle(title) => draw_text(
                    &format_section_title(title),
                    content_x,
                    y,
                    UiColorRole::Vivid,
                    &mut cells,
                ),
                ContentRow::Clear => {
                    let graphic = CellGraphic::Glyph(' ');
                    let role = assignment_role(&state, &graphic);
                    cells.push(Cell {
                        position: CellPoint {
                            x: content_x,
                            y,
                            z: 0,
                        },
                        graphic: CellGraphic::Glyph('·'),
                        color: preview_color(&state, &graphic, &self.palette),
                        weight: self.preview_weight(&state),
                        ..Cell::default()
                    });
                    draw_text("CLEAR", content_x + 2, y, role, &mut cells);
                    hits.push(LayoutHit::Clear {
                        x0: content_x,
                        x1: max_x,
                        y,
                    });
                }
                ContentRow::Glyphs(glyph_row) => {
                    for (column, glyph) in glyph_row.iter().enumerate() {
                        let x = content_x + column as i32;
                        if x > max_x {
                            break;
                        }
                        let graphic = CellGraphic::Glyph(*glyph);
                        cells.push(Cell {
                            position: CellPoint { x, y, z: 0 },
                            graphic: CellGraphic::Glyph(display_glyph(*glyph)),
                            color: preview_color(&state, &graphic, &self.palette),
                            weight: self.preview_weight(&state),
                            ..Cell::default()
                        });
                        hits.push(LayoutHit::Glyph { x, y, graphic });
                    }
                }
                ContentRow::Sprite(index) => {
                    let sprite = &self.sprites[*index];
                    let graphic = CellGraphic::Sprite(SpriteGraphic::new(&sprite.path));
                    let indicator = assignment_indicator(&state, &graphic);
                    let role = assignment_role(&state, &graphic);
                    cells.push(Cell {
                        position: CellPoint {
                            x: content_x,
                            y,
                            z: 0,
                        },
                        graphic: CellGraphic::Glyph(indicator),
                        color: self.palette.get(role),
                        weight: self.preview_weight(&state),
                        ..Cell::default()
                    });
                    cells.push(Cell {
                        position: CellPoint {
                            x: content_x + 2,
                            y,
                            z: 0,
                        },
                        graphic: graphic.clone(),
                        color: preview_color(&state, &graphic, &self.palette),
                        weight: self.preview_weight(&state),
                        ..Cell::default()
                    });
                    draw_text(&sprite.label, content_x + 4, y, role, &mut cells);
                    hits.push(LayoutHit::Sprite {
                        x0: content_x,
                        x1: max_x,
                        y,
                        graphic,
                    });
                }
                ContentRow::Sided(index) => {
                    let option = &self.sided[*index];
                    let graphic = CellGraphic::Sided(option.graphic.clone());
                    let indicator = assignment_indicator(&state, &graphic);
                    let role = assignment_role(&state, &graphic);
                    cells.push(Cell {
                        position: CellPoint {
                            x: content_x,
                            y,
                            z: 0,
                        },
                        graphic: CellGraphic::Glyph(indicator),
                        color: self.palette.get(role),
                        weight: self.preview_weight(&state),
                        ..Cell::default()
                    });
                    // The preview cell carries the Sided graphic itself: boot
                    // staging resolves the visible side, so the picker preview
                    // shows the declaration's front look with zero extra code.
                    cells.push(Cell {
                        position: CellPoint {
                            x: content_x + 2,
                            y,
                            z: 0,
                        },
                        graphic: graphic.clone(),
                        color: preview_color(&state, &graphic, &self.palette),
                        weight: self.preview_weight(&state),
                        ..Cell::default()
                    });
                    draw_text(&option.label, content_x + 4, y, role, &mut cells);
                    hits.push(LayoutHit::Sided {
                        x0: content_x,
                        x1: max_x,
                        y,
                        graphic,
                    });
                }
            }
        }

        // Vertical scrollbar over the scrollable middle, drawn last so the
        // thumb/track overwrite any row cell they sit on. Only renders when
        // the rows overflow the viewport.
        let content_rows = rows.len();
        let available = self.available_scroll_rows();
        cells.extend(self.scrollbar.cells_with_span(
            self.scrollbar_span(),
            content_rows,
            available,
            self.scroll.offset(),
            &self.palette,
            None,
            self.scrollbar.is_active(),
        ));

        (cells, hits)
    }

    fn glyph_hit_option(&self, needle: char) -> Option<(i32, i32)> {
        let (_, hits) = self.build_layout();
        hits.into_iter().find_map(|hit| match hit {
            LayoutHit::Glyph {
                x,
                y,
                graphic: CellGraphic::Glyph(glyph),
            } if glyph == needle => Some((self.rect.x0 + x, self.rect.y0 + y)),
            _ => None,
        })
    }

    fn hit_graphic_at(&self, x: i32, y: i32) -> Option<CellGraphic> {
        let (_, hits) = self.build_layout();
        hits.into_iter().find_map(|hit| match hit {
            LayoutHit::Clear { x0, x1, y: hit_y } => {
                (y - self.rect.y0 == hit_y && x - self.rect.x0 >= x0 && x - self.rect.x0 <= x1)
                    .then_some(CellGraphic::Glyph(' '))
            }
            LayoutHit::Glyph {
                x: hit_x,
                y: hit_y,
                graphic,
            } => (x - self.rect.x0 == hit_x && y - self.rect.y0 == hit_y).then_some(graphic),
            LayoutHit::Sprite {
                x0,
                x1,
                y: hit_y,
                graphic,
            }
            | LayoutHit::Sided {
                x0,
                x1,
                y: hit_y,
                graphic,
            }
            | LayoutHit::Recent {
                x0,
                x1,
                y: hit_y,
                graphic,
            } => (y - self.rect.y0 == hit_y && x - self.rect.x0 >= x0 && x - self.rect.x0 <= x1)
                .then_some(graphic),
        })
    }
}

impl Module for GraphicPickerModule {
    fn id(&self) -> &str {
        &self.id
    }

    fn rect(&self) -> ModuleRect {
        self.rect
    }

    /// Tooltip hotspots: the module's gizmo bar plus one hotspot per
    /// clickable glyph/sprite cell in the current layout, so every graphic
    /// explains itself through the shared tooltip implementation.
    fn hotspots(&self) -> Vec<Hotspot> {
        let (_, hits) = self.build_layout();
        let mut custom = vec![title_hotspot(
            self.rect,
            self.gizmos.title_start_x(),
            "graphics module",
            "picks the glyph or sprite for your given hand. left click for left hand, right click for right hand.",
        )];
        custom.extend(hits.into_iter().map(|hit| match hit {
            LayoutHit::Clear { x0, x1, y } => Hotspot::new(
                ModuleRect {
                    x0: self.rect.x0 + x0,
                    y0: self.rect.y0 + y,
                    x1: self.rect.x0 + x1,
                    y1: self.rect.y0 + y,
                },
                "clear character",
                "equip the clear character on a hand; brush, fill, lasso, and future paint tools then clear cells",
            ),
            LayoutHit::Glyph { x, y, graphic } => {
                let title = match graphic {
                    CellGraphic::Glyph(glyph) => format!("glyph {glyph}"),
                    _ => "glyph".to_string(),
                };
                // J 2026-09-10: the glyph's description is its unicode
                // character key — clean and techy.
                let key = match &graphic {
                    CellGraphic::Glyph(glyph) => format!("U+{:04X}", *glyph as u32),
                    _ => "glyph".to_string(),
                };
                Hotspot::new(
                    ModuleRect {
                        x0: self.rect.x0 + x,
                        y0: self.rect.y0 + y,
                        x1: self.rect.x0 + x,
                        y1: self.rect.y0 + y,
                    },
                    title,
                    format!("{key} — click to equip on a hand: left-click left, right-click right"),
                )
            }
            LayoutHit::Sprite { x0, x1, y, .. }
            | LayoutHit::Sided { x0, x1, y, .. }
            | LayoutHit::Recent { x0, x1, y, .. } => {
                Hotspot::new(
                    ModuleRect {
                        x0: self.rect.x0 + x0,
                        y0: self.rect.y0 + y,
                        x1: self.rect.x0 + x1,
                        y1: self.rect.y0 + y,
                    },
                    "sprite",
                    "click to equip on a hand: left-click left, right-click right",
                )
            }
        }));
        self.gizmos.hotspots_with(self.rect, custom)
    }

    fn draw(&self) -> CellGroup {
        let origin = WorldPoint {
            x: self.rect.x0,
            y: self.rect.y0,
            z: 0,
        };
        let (cells, _) = self.build_layout();
        CellGroup::from_cells(origin, cells).with_intake_behavior(CellGroupIntakeBehavior::Flat2d)
    }

    fn on_wheel(&mut self, _x: i32, _y: i32, _delta_x: f32, delta_y: f32) -> bool {
        // Re-sync the bar geometry before any interaction so panel resizes
        // keep the span honest.
        self.scrollbar.set_span(self.scrollbar_span());
        self.scroll.wheel(delta_y, self.max_scroll_rows());
        true
    }

    fn on_pointer_event(&mut self, event: ModulePointerEvent) {
        self.scrollbar.set_span(self.scrollbar_span());
        match event {
            ModulePointerEvent::Click { x, y, button } => {
                if let Some(outcome) = self.gizmo_state.handle_click(&self.gizmos, self.rect, x, y)
                {
                    if outcome == GizmoClickOutcome::Gizmo(GizmoKind::Close) {
                        self.hidden = true;
                    }
                    return;
                }

                // A left press on the scrollbar is bar interaction (thumb
                // grab or track page-jump), never a glyph assignment.
                if button == ModulePointerButton::Left
                    && self.scrollbar_press(x - self.rect.x0, y - self.rect.y0)
                {
                    return;
                }

                let hand = match button {
                    ModulePointerButton::Left => PaintHand::Left,
                    ModulePointerButton::Right => PaintHand::Right,
                    ModulePointerButton::Middle => return,
                };
                let Some(graphic) = self.hit_graphic_at(x, y) else {
                    return;
                };
                self.tool_state
                    .borrow_mut()
                    .set_graphic_for_hand(hand, graphic);
            }
            ModulePointerEvent::Move { x, y } => {
                self.scrollbar.note_pointer(x - self.rect.x0, y - self.rect.y0);
                // A live bar drag owns the pointer: follow the thumb, skip
                // the gizmo drag path entirely.
                if self.scrollbar.is_dragging() {
                    self.scrollbar_drag(x - self.rect.x0, y - self.rect.y0);
                    return;
                }
                self.gizmo_state.note_pointer(&self.gizmos, self.rect, x, y);
                if let Some(next_rect) = self.gizmo_state.drag_rect(x, y) {
                    self.rect = next_rect;
                }
            }
            ModulePointerEvent::Up { .. } => {
                self.scrollbar.on_up();
                self.gizmo_state.end_drag();
            }
            ModulePointerEvent::Enter => self.gizmo_state.set_hovered(true),
            ModulePointerEvent::Leave => {
                self.scrollbar.clear_pointer();
                self.gizmo_state.set_hovered(false);
            }
            ModulePointerEvent::Down { .. } => {}
        }
    }

    fn wants_pointer_capture(&self) -> bool {
        self.gizmo_state.wants_pointer_capture() || self.scrollbar.is_dragging()
    }

    fn is_hidden(&self) -> bool {
        self.hidden
    }

    fn set_hidden(&mut self, hidden: bool) {
        self.hidden = hidden;
    }

    fn persisted_ui_state(&self) -> Option<PersistedModuleUiState> {
        Some(PersistedModuleUiState::new(
            self.id(),
            self.rect,
            self.gizmo_state.is_seamless(),
            self.hidden,
        ))
    }

    fn apply_persisted_ui_state(&mut self, state: &PersistedModuleUiState) {
        self.rect = state.rect.to_runtime();
        self.gizmo_state.set_seamless(state.is_seamless);
        self.hidden = state.is_hidden;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> ModuleRect {
        ModuleRect {
            x0: 0,
            y0: 0,
            x1: 31,
            y1: 20,
        }
    }

    fn tool_state() -> Rc<RefCell<ToolState>> {
        Rc::new(RefCell::new(ToolState::default()))
    }

    fn glyph_hit(module: &GraphicPickerModule, needle: char) -> (i32, i32) {
        module
            .glyph_hit_option(needle)
            .unwrap_or_else(|| panic!("glyph hit should exist: {needle}"))
    }

    fn clear_hit(module: &GraphicPickerModule) -> (i32, i32) {
        let (_, hits) = module.build_layout();
        hits.into_iter()
            .find_map(|hit| match hit {
                LayoutHit::Clear { x0, y, .. } => Some((module.rect.x0 + x0, module.rect.y0 + y)),
                _ => None,
            })
            .expect("clear character hit should exist")
    }

    fn scroll_to_top(module: &mut GraphicPickerModule) {
        for _ in 0..200 {
            module.on_wheel(0, 0, 0.0, 1.0);
        }
    }

    fn scroll_to_bottom(module: &mut GraphicPickerModule) {
        for _ in 0..200 {
            module.on_wheel(0, 0, 0.0, -1.0);
        }
    }

    fn sprite_hit(module: &GraphicPickerModule, needle: &str) -> (i32, i32) {
        let (_, hits) = module.build_layout();
        hits.into_iter()
            .find_map(|hit| match hit {
                LayoutHit::Sprite {
                    x0,
                    y,
                    graphic: CellGraphic::Sprite(sprite),
                    ..
                } if sprite.atlas_relative_path().to_string_lossy() == needle => {
                    Some((module.rect.x0 + x0 + 2, module.rect.y0 + y))
                }
                _ => None,
            })
            .expect("sprite hit should exist")
    }

    #[test]
    fn supported_glyph_sections_preserve_the_leading_ascii_space() {
        let sections = parse_supported_glyph_sections("ascii:\n ABC\n");
        assert_eq!(sections[0].title, "ascii");
        assert_eq!(sections[0].glyphs, vec![' ', 'A', 'B', 'C']);
    }

    #[test]
    fn glyph_sections_parse_glued_title_lines_like_the_renderer() {
        // The shipped sections.txt glues some titles to their glyph lines
        // (`borders:━┃…`); the renderer's sprite parser accepts both shapes,
        // so the picker must too — otherwise whole sections vanish.
        let sections = parse_supported_glyph_sections("ascii:\n ABC\n\nborders:━┃\n");
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[1].title, "borders");
        assert_eq!(sections[1].glyphs, vec!['━', '┃']);
    }

    #[test]
    fn glyph_lines_containing_colons_still_parse_as_glyphs() {
        // The dots section contains ':' as a glyph (`.,:;…`); a colon inside
        // the glyph line must not be mistaken for a new section title.
        let sections = parse_supported_glyph_sections("dots:\n.,:;·\n");
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].title, "dots");
        assert_eq!(sections[0].glyphs, vec!['.', ',', ':', ';', '·']);
    }

    #[test]
    fn shipped_glyph_sections_include_every_atlas_batch_including_borders_and_ornaments() {
        let sections = supported_glyph_sections();
        let titles: Vec<&str> = sections
            .iter()
            .map(|section| section.title.as_str())
            .collect();
        for expected in [
            "ascii",
            "faces",
            "symbols",
            "dots",
            "dashes",
            "arrows",
            "borders",
            "blocks",
            "latinext",
            "ornaments",
            "games",
        ] {
            assert!(titles.contains(&expected), "missing section: {expected}");
        }
        // The atlas symbols sheet has one more column than a pre-fix
        // sections.txt declared: the generator's `°` between `≈` and `✝`.
        // Without it every later symbols glyph painted the wrong sprite.
        let symbols = sections
            .iter()
            .find(|section| section.title == "symbols")
            .expect("symbols section");
        let degree = symbols
            .glyphs
            .iter()
            .position(|glyph| *glyph == '°')
            .expect("degree sign present in symbols");
        assert_eq!(symbols.glyphs[degree - 1], '≈');
        assert_eq!(symbols.glyphs[degree + 1], '✝');
    }

    #[test]
    fn left_clicking_a_glyph_assigns_the_left_hand_graphic() {
        let state = tool_state();
        let mut module = GraphicPickerModule::new("graphics", rect(), state.clone());
        let (x, y) = glyph_hit(&module, 'A');

        module.on_pointer_event(ModulePointerEvent::Click {
            x,
            y,
            button: ModulePointerButton::Left,
        });

        assert_eq!(state.borrow().left_hand.graphic, CellGraphic::Glyph('A'));
        assert_eq!(state.borrow().active_hand, PaintHand::Left);
    }

    #[test]
    fn clear_character_is_visible_and_assigns_a_real_blank_glyph() {
        let state = tool_state();
        let mut module = GraphicPickerModule::new("graphics", rect(), state.clone());
        let (x, y) = clear_hit(&module);

        module.on_pointer_event(ModulePointerEvent::Click {
            x,
            y,
            button: ModulePointerButton::Right,
        });

        assert_eq!(state.borrow().right_hand.graphic, CellGraphic::Glyph(' '));
        assert_eq!(state.borrow().active_hand, PaintHand::Right);
    }

    #[test]
    fn right_clicking_a_sprite_assigns_the_right_hand_graphic() {
        let state = tool_state();
        let mut module = GraphicPickerModule::new("graphics", rect(), state.clone());
        // Sprites are a standard category at the end of the scroll list.
        scroll_to_bottom(&mut module);
        let (x, y) = sprite_hit(&module, "proofs/grass.png");

        module.on_pointer_event(ModulePointerEvent::Click {
            x,
            y,
            button: ModulePointerButton::Right,
        });

        assert_eq!(
            state.borrow().right_hand.graphic,
            CellGraphic::Sprite(SpriteGraphic::new("proofs/grass.png"))
        );
        assert_eq!(state.borrow().active_hand, PaintHand::Right);
    }

    #[test]
    fn assigned_glyph_previews_the_hand_color() {
        let state = tool_state();
        {
            let mut state = state.borrow_mut();
            state.left_hand.graphic = CellGraphic::Glyph('A');
            state.left_hand.color = crate::PaintColor::flat_rgb(12, 34, 56);
        }
        let module = GraphicPickerModule::new("graphics", rect(), state);
        let (hit_x, hit_y) = glyph_hit(&module, 'A');

        let (cells, _) = module.build_layout();
        let preview = cells
            .into_iter()
            .find(|cell| {
                cell.position
                    == CellPoint {
                        x: hit_x - module.rect.x0,
                        y: hit_y - module.rect.y0,
                        z: 0,
                    }
                    && cell.graphic == CellGraphic::Glyph('A')
            })
            .expect("assigned glyph preview should exist");

        assert_eq!(
            preview.color,
            thaum_renderer_domain::CellColor::Flat([12.0 / 255.0, 34.0 / 255.0, 56.0 / 255.0, 1.0])
        );
    }

    /// Weight preview cell at a glyph hit, per the module-local clock value.
    fn glyph_preview_weight(module: &GraphicPickerModule, needle: char) -> CellWeight {
        let (hit_x, hit_y) = glyph_hit(module, needle);
        let (cells, _) = module.build_layout();
        cells
            .into_iter()
            .find(|cell| {
                cell.position
                    == CellPoint {
                        x: hit_x - module.rect.x0,
                        y: hit_y - module.rect.y0,
                        z: 0,
                    }
                    && cell.graphic == CellGraphic::Glyph(display_glyph(needle))
            })
            .expect("glyph preview should exist")
            .weight
    }

    #[test]
    fn the_single_image_hand_weights_every_preview() {
        let state = tool_state();
        {
            let mut state = state.borrow_mut();
            state.left_hand.target = PaintTarget::Image;
            state.left_hand.weight_index = 3;
            state.right_hand.target = PaintTarget::Selection;
            state.right_hand.weight_index = 1;
        }
        let module = GraphicPickerModule::new("graphics", rect(), state);

        // Right targets selection: the left hand's weight shows, unblinking.
        assert_eq!(
            glyph_preview_weight(&module, 'A'),
            CellWeight::from_index_clamped(3)
        );
    }

    #[test]
    fn both_image_hands_alternate_weights_on_the_flash_clock() {
        let state = tool_state();
        {
            let mut state = state.borrow_mut();
            state.left_hand.target = PaintTarget::Image;
            state.left_hand.weight_index = 3;
            state.right_hand.target = PaintTarget::Image;
            state.right_hand.weight_index = 1;
        }
        let clock = Rc::new(std::cell::Cell::new(0));
        let module =
            GraphicPickerModule::new("graphics", rect(), state).with_breath_clock(clock.clone());

        // Off phase shows the right hand's weight, lit phase the left's.
        clock.set(0);
        assert_eq!(
            glyph_preview_weight(&module, 'A'),
            CellWeight::from_index_clamped(1)
        );
        clock.set(1);
        assert_eq!(
            glyph_preview_weight(&module, 'A'),
            CellWeight::from_index_clamped(3)
        );
    }

    #[test]
    fn neither_image_hand_alternates_weights_too() {
        let state = tool_state();
        {
            let mut state = state.borrow_mut();
            state.left_hand.target = PaintTarget::Selection;
            state.left_hand.weight_index = 0;
            state.right_hand.target = PaintTarget::Selection;
            state.right_hand.weight_index = 2;
        }
        let clock = Rc::new(std::cell::Cell::new(0));
        let module =
            GraphicPickerModule::new("graphics", rect(), state).with_breath_clock(clock.clone());

        clock.set(0);
        assert_eq!(
            glyph_preview_weight(&module, 'A'),
            CellWeight::from_index_clamped(2)
        );
        clock.set(1);
        assert_eq!(
            glyph_preview_weight(&module, 'A'),
            CellWeight::from_index_clamped(0)
        );
    }

    #[test]
    fn overflowing_rows_draw_a_vertical_scrollbar() {
        let state = tool_state();
        let module = GraphicPickerModule::new("graphics", rect(), state);
        let span = module.scrollbar_span();
        let (cells, _) = module.build_layout();
        // The bar draws on its column: thumb at the screen-top end
        // (offset 0) — the occupied top cap keeps its own glyph, weight
        // boosted (J), so it reads ┳ weight 2 instead of the filled ┃.
        assert!(cells.iter().any(|cell| {
            cell.position.x == span_x(&span)
                && cell.position.y == span_y_top(&span)
                && cell.graphic == CellGraphic::Glyph('┳')
        }));
        assert!(cells.iter().any(|cell| {
            cell.position.x == span_x(&span)
                && cell.position.y == span_y_bottom(&span)
                && cell.graphic == CellGraphic::Glyph('┻')
        }));
    }

    #[test]
    fn thumb_drag_walks_the_category_rows_to_both_edges() {
        let state = tool_state();
        let mut module = GraphicPickerModule::new("graphics", rect(), state);
        let span = module.scrollbar_span();
        let max = module.max_scroll_rows();
        assert!(max > 0, "default picker content overflows its viewport");

        // Press the thumb's top-end edge, drag the pointer to the bar's
        // bottom end: offset lands at the max.
        assert!(module.scrollbar_press(span_x(&span), span_y_top(&span)));
        assert!(module.scrollbar_drag(span_x(&span), span_y_bottom(&span)));
        assert_eq!(module.scroll.offset(), max);

        // Drag back to the top end: offset returns to the top.
        assert!(module.scrollbar_drag(span_x(&span), span_y_top(&span)));
        assert_eq!(module.scroll.offset(), 0);
    }

    fn span_x(span: &ScrollbarSpan) -> i32 {
        match span {
            ScrollbarSpan::Vertical { x, .. } => *x,
            ScrollbarSpan::Horizontal { .. } => unreachable!(),
        }
    }

    fn span_y_top(span: &ScrollbarSpan) -> i32 {
        match span {
            ScrollbarSpan::Vertical { top, .. } => *top,
            ScrollbarSpan::Horizontal { .. } => unreachable!(),
        }
    }

    fn span_y_bottom(span: &ScrollbarSpan) -> i32 {
        match span {
            ScrollbarSpan::Vertical { bottom, .. } => *bottom,
            ScrollbarSpan::Horizontal { .. } => unreachable!(),
        }
    }

    #[test]
    fn default_view_shows_the_first_glyph_sections() {
        let state = tool_state();
        let module = GraphicPickerModule::new("graphics", rect(), state);
        assert_eq!(module.scroll.offset(), 0);
        assert!(module.glyph_hit_option('A').is_some());
    }

    #[test]
    fn scrolling_down_reveals_later_sections_and_crops_early_ones() {
        let state = tool_state();
        let mut module = GraphicPickerModule::new("graphics", rect(), state);

        scroll_to_bottom(&mut module);
        assert_eq!(module.scroll.offset(), module.max_scroll_rows());
        assert!(module.max_scroll_rows() > 0);
        // the top of the content (ascii) is cropped at max scroll
        assert_eq!(module.glyph_hit_option('A'), None);

        scroll_to_top(&mut module);
        assert!(module.glyph_hit_option('A').is_some());
    }

    #[test]
    fn sprite_rows_stay_pinned_while_scrolling() {
        let state = tool_state();
        let mut module = GraphicPickerModule::new("graphics", rect(), state.clone());

        scroll_to_bottom(&mut module);
        let (x, y) = sprite_hit(&module, "proofs/grass.png");

        module.on_pointer_event(ModulePointerEvent::Click {
            x,
            y,
            button: ModulePointerButton::Right,
        });
        assert_eq!(
            state.borrow().right_hand.graphic,
            CellGraphic::Sprite(SpriteGraphic::new("proofs/grass.png"))
        );
    }

    #[test]
    fn scrolling_clamps_at_both_content_edges() {
        let state = tool_state();
        let mut module = GraphicPickerModule::new("graphics", rect(), state);

        scroll_to_top(&mut module);
        assert_eq!(module.scroll.offset(), 0);

        scroll_to_bottom(&mut module);
        let expected_max = module.max_scroll_rows();
        assert!(expected_max > 0, "content should exceed the panel height");
        assert_eq!(module.scroll.offset(), expected_max);
    }

    #[test]
    fn glyph_rows_reflow_with_module_width() {
        // Regression: rows must chunk at the panel's content WIDTH, so a
        // wider panel fits more glyphs per row and a narrower one fewer.
        let state = tool_state();
        let narrow = GraphicPickerModule::new(
            "graphics",
            ModuleRect {
                x0: 0,
                y0: 0,
                x1: 20,
                y1: 40,
            },
            state.clone(),
        );
        let wide = GraphicPickerModule::new(
            "graphics",
            ModuleRect {
                x0: 0,
                y0: 0,
                x1: 60,
                y1: 40,
            },
            state,
        );
        let first_glyph_row = |module: &GraphicPickerModule| {
            module
                .build_scroll_rows()
                .iter()
                .find_map(|row| match row {
                    ContentRow::Glyphs(glyphs) => Some(glyphs.len()),
                    _ => None,
                })
                .expect("glyph row should exist")
        };
        // narrow: 20-wide rect -> 18 content columns, 17 usable once the
        // visible bar reserves the rightmost column; wide: 60 -> 58, 57
        // with the bar. The ascii section outgrows both, so each first row
        // must fill exactly its own panel's bar-aware content width.
        assert_eq!(first_glyph_row(&narrow), 17);
        assert_eq!(first_glyph_row(&wide), 57);
    }

    #[test]
    fn no_character_draws_under_the_visible_scrollbar() {
        let state = tool_state();
        let module = GraphicPickerModule::new("graphics", rect(), state);
        let span = module.scrollbar_span();
        let (cells, hits) = module.build_layout();
        // The bar is visible, so no glyph cell or glyph hit lands on the
        // bar's column: every character stays reachable without resizing.
        let offenders: Vec<String> = cells
            .iter()
            .filter(|cell| cell.position.x == span_x(&span)
                && cell.position.y <= span_y_top(&span)
                && cell.position.y >= span_y_bottom(&span))
            .filter(|cell| match cell.graphic {
                // Bar glyphs win the column; the chrome's blank clears
                // under the bar are fine — the bar draws over them.
                CellGraphic::Glyph('━' | '┳' | '┻' | '║' | '┃' | ' ') => false,
                _ => true,
            })
            .map(|cell| format!("{:?} {:?}", cell.position, cell.graphic))
            .collect();
        assert!(offenders.is_empty(), "cells under the bar: {offenders:?}");
        assert!(hits.iter().all(|hit| match hit {
            LayoutHit::Glyph { x, .. } => *x != span_x(&span),
            _ => true,
        }));
    }

    #[test]
    fn recent_row_tracks_assignments_most_recent_first() {
        let state = tool_state();
        let mut module = GraphicPickerModule::new("graphics", rect(), state);

        let (x, y) = glyph_hit(&module, 'A');
        module.on_pointer_event(ModulePointerEvent::Click {
            x,
            y,
            button: ModulePointerButton::Left,
        });
        let (x, y) = glyph_hit(&module, 'B');
        module.on_pointer_event(ModulePointerEvent::Click {
            x,
            y,
            button: ModulePointerButton::Left,
        });

        let (cells, _) = module.build_layout();
        let recent_row_y =
            PanelChrome::content_origin().1 + PanelChrome::content_size(rect()).1 - 2;
        let x_of = |needle: char| {
            cells
                .iter()
                .find(|cell| {
                    cell.position.y == recent_row_y && cell.graphic == CellGraphic::Glyph(needle)
                })
                .expect("recent glyph should render")
                .position
                .x
        };
        assert!(x_of('B') < x_of('A'), "B was assigned after A");
    }

    #[test]
    fn clicking_a_recent_slot_reassigns_the_graphic() {
        let state = tool_state();
        let mut module = GraphicPickerModule::new("graphics", rect(), state.clone());

        let (x, y) = glyph_hit(&module, 'A');
        module.on_pointer_event(ModulePointerEvent::Click {
            x,
            y,
            button: ModulePointerButton::Right,
        });
        let (x, y) = glyph_hit(&module, 'B');
        module.on_pointer_event(ModulePointerEvent::Click {
            x,
            y,
            button: ModulePointerButton::Right,
        });
        let (_, hits) = module.build_layout();
        let (slot_x, slot_y) = hits
            .iter()
            .find_map(|hit| match hit {
                LayoutHit::Recent {
                    x0,
                    y,
                    graphic: CellGraphic::Glyph('A'),
                    ..
                } => Some((module.rect.x0 + x0, module.rect.y0 + *y)),
                _ => None,
            })
            .expect("recent A slot should exist");

        module.on_pointer_event(ModulePointerEvent::Click {
            x: slot_x,
            y: slot_y,
            button: ModulePointerButton::Right,
        });
        assert_eq!(state.borrow().right_hand.graphic, CellGraphic::Glyph('A'));
    }

    #[test]
    fn clicking_the_move_gizmo_starts_requesting_pointer_capture() {
        let state = tool_state();
        let mut module = GraphicPickerModule::new("graphics", rect(), state);

        module.on_pointer_event(ModulePointerEvent::Click {
            x: 1,
            y: 19,
            button: ModulePointerButton::Left,
        });

        assert!(module.wants_pointer_capture());
    }
}

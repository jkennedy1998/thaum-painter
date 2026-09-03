use std::{cell::RefCell, rc::Rc};

use thaum_renderer_domain::{
    Cell, CellGraphic, CellGroup, CellGroupIntakeBehavior, CellPoint, CellWeight, GizmoBar,
    GizmoClickOutcome, GizmoKind, GizmoState, Module, ModulePointerButton, ModulePointerEvent,
    ModuleRect, PanelChrome, PersistedModuleUiState, SpriteGraphic, UiColorRole, UiPalette,
    WorldPoint,
};

use crate::tool_state::{HandState, PaintHand, ToolState};

const GLYPH_SECTIONS_TEXT: &str = include_str!("/home/j/Repos/thaum-renderer/orchestration/renderer-assets/cell-sprites/monothaum-atlas-v3/sections.txt");
const SUPPORTED_SPRITE_PATHS: &[&str] = &["proofs/channel-bands.png", "proofs/grass.png"];

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

#[derive(Debug, Clone, PartialEq, Eq)]
enum LayoutHit {
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
}

fn parse_supported_glyph_sections(text: &str) -> Vec<GlyphSection> {
    let mut sections = Vec::new();
    let mut pending_title: Option<String> = None;

    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }

        if let Some(title) = line.strip_suffix(':') {
            pending_title = Some(title.to_string());
            continue;
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

fn preview_weight(tool_state: &ToolState, graphic: &CellGraphic) -> CellWeight {
    let left = (tool_state.left_hand.graphic == *graphic).then_some(tool_state.left_hand.weight_index);
    let right = (tool_state.right_hand.graphic == *graphic).then_some(tool_state.right_hand.weight_index);
    CellWeight::from_index_clamped(left.into_iter().chain(right).max().unwrap_or(2) as i32)
}

pub struct GraphicPickerModule {
    id: String,
    rect: ModuleRect,
    tool_state: Rc<RefCell<ToolState>>,
    glyph_sections: Vec<GlyphSection>,
    sprites: Vec<SpriteOption>,
    palette: UiPalette,
    gizmos: GizmoBar,
    gizmo_state: GizmoState,
    hidden: bool,
}

impl GraphicPickerModule {
    pub fn new(
        id: impl Into<String>,
        rect: ModuleRect,
        tool_state: Rc<RefCell<ToolState>>,
    ) -> Self {
        Self {
            id: id.into(),
            rect,
            tool_state,
            glyph_sections: supported_glyph_sections(),
            sprites: supported_sprites(),
            palette: UiPalette::default(),
            gizmos: GizmoBar::standard(),
            gizmo_state: GizmoState::new(),
            hidden: false,
        }
    }

    pub fn with_palette(mut self, palette: UiPalette) -> Self {
        self.palette = palette;
        self
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
        let mut hits = Vec::new();
        let mut cursor_y = content_y + content_height - 1;
        let min_y = content_y;
        let max_x = content_x + content_width - 1;

        let draw_text = |text: &str, x: i32, y: i32, role: UiColorRole, cells: &mut Vec<Cell>| {
            if y < min_y {
                return;
            }
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

        let mut reserve_row = || {
            if cursor_y < min_y {
                None
            } else {
                let y = cursor_y;
                cursor_y -= 1;
                Some(y)
            }
        };

        if let Some(y) = reserve_row() {
            draw_text("SPRITES", content_x, y, UiColorRole::Bright, &mut cells);
        }
        for sprite in &self.sprites {
            let Some(y) = reserve_row() else {
                break;
            };
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
                weight: preview_weight(&state, &graphic),
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
                weight: preview_weight(&state, &graphic),
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

        if !self.sprites.is_empty() {
            let _ = reserve_row();
        }

        if let Some(y) = reserve_row() {
            draw_text("GLYPHS", content_x, y, UiColorRole::Bright, &mut cells);
        }

        let columns = content_width.max(1) as usize;
        for section in &self.glyph_sections {
            let Some(header_y) = reserve_row() else {
                break;
            };
            draw_text(
                &format_section_title(&section.title),
                content_x,
                header_y,
                UiColorRole::Vivid,
                &mut cells,
            );

            for glyph_row in section.glyphs.chunks(columns) {
                let Some(y) = reserve_row() else {
                    break;
                };
                for (column, glyph) in glyph_row.iter().enumerate() {
                    let x = content_x + column as i32;
                    let graphic = CellGraphic::Glyph(*glyph);
                    cells.push(Cell {
                        position: CellPoint { x, y, z: 0 },
                        graphic: CellGraphic::Glyph(display_glyph(*glyph)),
                        color: preview_color(&state, &graphic, &self.palette),
                        weight: preview_weight(&state, &graphic),
                        ..Cell::default()
                    });
                    hits.push(LayoutHit::Glyph { x, y, graphic });
                }
            }
        }

        (cells, hits)
    }

    fn hit_graphic_at(&self, x: i32, y: i32) -> Option<CellGraphic> {
        let (_, hits) = self.build_layout();
        hits.into_iter().find_map(|hit| match hit {
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

    fn draw(&self) -> CellGroup {
        let origin = WorldPoint {
            x: self.rect.x0,
            y: self.rect.y0,
            z: 0,
        };
        let (cells, _) = self.build_layout();
        CellGroup::from_cells(origin, cells).with_intake_behavior(CellGroupIntakeBehavior::Flat2d)
    }

    fn on_pointer_event(&mut self, event: ModulePointerEvent) {
        match event {
            ModulePointerEvent::Click { x, y, button } => {
                if let Some(outcome) = self.gizmo_state.handle_click(&self.gizmos, self.rect, x, y)
                {
                    if outcome == GizmoClickOutcome::Gizmo(GizmoKind::Close) {
                        self.hidden = true;
                    }
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
                self.gizmo_state.note_pointer(&self.gizmos, self.rect, x, y);
                if let Some(next_rect) = self.gizmo_state.drag_rect(x, y) {
                    self.rect = next_rect;
                }
            }
            ModulePointerEvent::Up { .. } => self.gizmo_state.end_drag(),
            ModulePointerEvent::Enter => self.gizmo_state.set_hovered(true),
            ModulePointerEvent::Leave => self.gizmo_state.set_hovered(false),
            ModulePointerEvent::Down { .. } => {}
        }
    }

    fn wants_pointer_capture(&self) -> bool {
        self.gizmo_state.wants_pointer_capture()
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
        let (_, hits) = module.build_layout();
        hits.into_iter()
            .find_map(|hit| match hit {
                LayoutHit::Glyph {
                    x,
                    y,
                    graphic: CellGraphic::Glyph(glyph),
                } if glyph == needle => Some((module.rect.x0 + x, module.rect.y0 + y)),
                _ => None,
            })
            .expect("glyph hit should exist")
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
    fn right_clicking_a_sprite_assigns_the_right_hand_graphic() {
        let state = tool_state();
        let mut module = GraphicPickerModule::new("graphics", rect(), state.clone());
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
    fn assigned_glyph_previews_the_hand_color_and_weight() {
        let state = tool_state();
        {
            let mut state = state.borrow_mut();
            state.left_hand.graphic = CellGraphic::Glyph('A');
            state.left_hand.color = crate::PaintColor::flat_rgb(12, 34, 56);
            state.left_hand.weight_index = 3;
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
        assert_eq!(preview.weight, CellWeight::from_index_clamped(3));
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

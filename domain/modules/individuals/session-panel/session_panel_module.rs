//! Session panel: the user surface for LAN multiplayer (the session-ui
//! plan's view + intent module). One standard `Module` over a shared
//! `SessionPanelState`: standard gizmo bar (move / close / resize /
//! seamless) on `PanelChrome` chrome, recallable from the command bar via
//! the registry's hidden flag — the same shape as every other painter
//! panel (material-picker pattern). No chip: status lives in the panel's
//! own synced status row, never in a fake always-attached button.
//!
//! Ownership rule: the module never touches `SessionNet`/`SessionHost`/
//! `SessionClient`/sockets. It renders only what `SessionPanelState::sync`
//! was told — every displayed state is real synced state, never invented —
//! and emits at most one `SessionPanelAction` per frame via
//! `take_pending_action` for the orchestration layer to apply onto the one
//! `Option<SessionNet>`. Swap the transport, the panel doesn't know; swap
//! the panel, the transport doesn't know.
//!
//! Text entry rides the existing key-capture seam (`Module::on_key_capture`,
//! the controls-panel rebind pattern): clicking a field focuses it, focused
//! fields consume keys until Enter commits or Escape blurs. There is no
//! generic Module-trait focus/IME hook yet — the same deferred cross-repo
//! seam the layers-panel rename wants.

use std::cell::RefCell;
use std::rc::Rc;

use thaum_renderer_domain::{
    Cell, CellColor, CellGraphic, CellGroup, CellGroupIntakeBehavior, CellPoint, GizmoBar,
    GizmoClickOutcome, GizmoKind, GizmoState, Hotspot, Module, ModulePointerButton,
    ModulePointerEvent, ModuleRect, PanelChrome, PersistedModuleUiState, UiColorRole, UiPalette,
    WorldPoint,
};

/// One roster row as synced from the live session. The host is roster entry
/// zero by the `SessionNet::roster` contract; `is_you` marks the local user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRosterRow {
    pub user_id: String,
    pub display_name: String,
    pub is_host: bool,
    pub is_you: bool,
    /// The user's presence color, 8-bit RGB (matches `SessionUser`).
    pub color: [u8; 3],
}

/// A user action requested through the panel, for the orchestration layer to
/// apply onto the real `SessionNet` and reflect back through `sync`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionPanelAction {
    /// Host a session on the current document.
    HostRequested,
    /// Join the host at a typed `ip:port` (default port applied by the
    /// orchestration layer through `join_address`).
    JoinRequested { address: String },
    /// Copy the invite addresses to the OS clipboard (orchestration-owned).
    CopyInvite,
    /// Leave / end the session (host leaving ends it for everyone).
    LeaveRequested,
    /// Commit the edited display name.
    SetDisplayName(String),
}

/// Which panel field owns the keyboard right now. Module-local truth: the
/// orchestration layer never sees drafts or focus, only committed actions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum FieldFocus {
    #[default]
    None,
    JoinAddress,
    DisplayName,
}

/// Shared state between the module and its orchestration caller. The module
/// only ever reads/writes this struct — no document or socket.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionPanelState {
    /// True while this app participates in a session (either side).
    pub in_session: bool,
    /// True when this app is the hosting side.
    pub is_host: bool,
    /// Live link health as the frame loop last saw it.
    pub connected: bool,
    pub reconnecting: bool,
    pub session_ended: bool,
    /// Other users in the session (roster minus self, host excluded).
    pub peer_count: usize,
    /// `SessionNet::invite_addresses()` verbatim; empty when not hosting.
    pub invite_addresses: Vec<String>,
    /// The full roster, host first (the `SessionNet::roster` contract).
    pub roster: Vec<SessionRosterRow>,
    /// The local user's current display name.
    pub self_display_name: String,
    /// The last events (joined / left / dropped / denied / copied), oldest
    /// first. The panel renders the newest three; nothing it was not told.
    pub events: Vec<String>,
    pending_action: Option<SessionPanelAction>,
}

/// The event log shown is the newest three lines.
const EVENT_LINES: usize = 3;

impl SessionPanelState {
    /// Pushes one event line, capped at the log-lite limit.
    pub fn push_event(&mut self, event: impl Into<String>) {
        self.events.push(event.into());
        if self.events.len() > EVENT_LINES {
            let excess = self.events.len() - EVENT_LINES;
            self.events.drain(0..excess);
        }
    }

    /// The one-line session status, derived only from synced fields.
    pub fn status_label(&self) -> String {
        if self.session_ended {
            return "ENDED".to_string();
        }
        if self.reconnecting {
            return "RECONNECTING...".to_string();
        }
        if self.in_session && self.is_host {
            return match self.invite_addresses.first() {
                Some(address) => format!("HOSTING {address}"),
                None => "HOSTING".to_string(),
            };
        }
        if self.in_session {
            return format!("CONNECTED ({})", self.peer_count);
        }
        "OFFLINE".to_string()
    }

    /// Takes the pending action, if any. At most one action queues per frame.
    pub fn take_pending_action(&mut self) -> Option<SessionPanelAction> {
        self.pending_action.take()
    }

    /// Refreshes the displayed truth from the live session. Called once per
    /// frame by the orchestration layer; events are the panel's own memory
    /// and are never touched here.
    #[allow(clippy::too_many_arguments)] // one arg per synced truth field
    pub fn sync(
        &mut self,
        in_session: bool,
        is_host: bool,
        connected: bool,
        reconnecting: bool,
        session_ended: bool,
        peer_count: usize,
        invite_addresses: Vec<String>,
        roster: Vec<SessionRosterRow>,
        self_display_name: String,
    ) {
        self.in_session = in_session;
        self.is_host = is_host;
        self.connected = connected;
        self.reconnecting = reconnecting;
        self.session_ended = session_ended;
        self.peer_count = peer_count;
        self.invite_addresses = invite_addresses;
        self.roster = roster;
        self.self_display_name = self_display_name;
    }

    fn queue_action(&mut self, action: SessionPanelAction) {
        self.pending_action = Some(action);
    }
}

fn text_color_role() -> UiColorRole {
    UiColorRole::Medium
}

fn truncate_to_width(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

fn push_text(cells: &mut Vec<Cell>, x: i32, y: i32, glyph: char, color: CellColor) {
    cells.push(Cell {
        position: CellPoint { x, y, z: 0 },
        graphic: CellGraphic::Glyph(glyph),
        color,
        ..Cell::default()
    });
}

fn push_line(cells: &mut Vec<Cell>, y: i32, line: &str, color: CellColor, width: usize) {
    // Text starts one content column in, matching the roster rows' left pad.
    for (index, glyph) in truncate_to_width(line, width).chars().enumerate() {
        push_text(cells, content_x_of(index as i32 + 1), y, glyph, color);
    }
}

/// The click-open session detail panel in standard module form: gizmo-bar
/// chrome, command-bar recall, roster, invite copy, join field, name edit,
/// and the log-lite event lines.
pub struct SessionPanelModule {
    id: String,
    rect: ModuleRect,
    state: Rc<RefCell<SessionPanelState>>,
    palette: UiPalette,
    gizmos: GizmoBar,
    gizmo_state: GizmoState,
    hidden: bool,
    focus: FieldFocus,
    join_draft: String,
    name_draft: String,
}

/// Content rows (local to `PanelChrome::content_origin`, bottom-up) shared
/// by draw and hit-test.
const ROW_EVENTS_BASE: i32 = 0;
const ROW_NAME: i32 = 4;
const ROW_BUTTONS: i32 = 5;
const ROW_ROSTER_BASE: i32 = 7;
const ROW_JOIN: i32 = 6;
const ROW_HOST: i32 = 8;

impl SessionPanelModule {
    pub fn new(
        id: impl Into<String>,
        rect: ModuleRect,
        state: Rc<RefCell<SessionPanelState>>,
    ) -> Self {
        Self {
            id: id.into(),
            rect,
            state,
            palette: UiPalette::default(),
            gizmos: GizmoBar::standard(),
            gizmo_state: GizmoState::new(),
            hidden: false,
            focus: FieldFocus::None,
            join_draft: String::new(),
            name_draft: String::new(),
        }
    }

    pub fn with_palette(mut self, palette: UiPalette) -> Self {
        self.palette = palette;
        self
    }

    /// Content width in cells inside the chrome.
    fn content_width(&self) -> i32 {
        let (width, _) = PanelChrome::content_size(self.rect);
        width
    }

    /// Content height in cells inside the chrome.
    fn content_height(&self) -> i32 {
        let (_, height) = PanelChrome::content_size(self.rect);
        height
    }

    /// Converts an absolute pointer position into content-local coordinates.
    /// `None` when the point is outside the usable content area (gizmo bar,
    /// borders and title row are chrome, not content).
    fn content_local(&self, x: i32, y: i32) -> Option<(i32, i32)> {
        let (origin_x, origin_y) = PanelChrome::content_origin();
        let local_x = x - self.rect.x0 - origin_x;
        let local_y = y - self.rect.y0 - origin_y;
        (local_x >= 0 && local_y >= 0 && local_y < self.content_height())
            .then_some((local_x, local_y))
    }

    /// Roster rows the panel can show; the rest scroll off (v1 keeps the
    /// panel small — more users than fits is past LAN-v1 honesty anyway).
    fn visible_roster_rows(&self) -> usize {
        let fit = (self.content_height() - 1 - ROW_ROSTER_BASE).max(0) as usize;
        self.state.borrow().roster.len().min(fit)
    }

    fn roster_row_at(&self, count: usize, row: i32) -> Option<usize> {
        let index = (row - ROW_ROSTER_BASE) as usize;
        (index < count).then_some(index)
    }

    /// Denial/event text passthrough: the panel shows what actually happened,
    /// verbatim from the orchestration layer.
    fn event_text(&self, events: &[String], row: i32) -> Option<(String, CellColor)> {
        let index = (row - ROW_EVENTS_BASE) as usize;
        // Newest at the lowest row, older stacked above.
        let event = events.get(events.len().checked_sub(1 + index)?)?;
        Some((
            truncate_to_width(event, self.content_width() as usize),
            self.palette.get(text_color_role()),
        ))
    }

    fn blur_fields(&mut self) {
        self.focus = FieldFocus::None;
        self.join_draft.clear();
        self.name_draft.clear();
    }
}

impl Module for SessionPanelModule {
    fn id(&self) -> &str {
        &self.id
    }

    fn rect(&self) -> ModuleRect {
        self.rect
    }

    /// Tooltip hotspots: the module's gizmo bar, so every gizmo-enabled
    /// panel grows tooltips from one shared implementation.
    fn hotspots(&self) -> Vec<Hotspot> {
        self.gizmos.hotspots(self.rect)
    }

    fn draw(&self) -> CellGroup {
        if self.hidden {
            return CellGroup::new(WorldPoint {
                x: self.rect.x0,
                y: self.rect.y0,
                z: 0,
            })
            .with_intake_behavior(CellGroupIntakeBehavior::Flat2d);
        }

        let state = self.state.borrow();
        let mut cells: Vec<Cell> = if self.gizmo_state.is_seamless() {
            Vec::new()
        } else {
            self.gizmo_state
                .decorate_panel_chrome(
                    PanelChrome::new(self.rect, &self.palette)
                        .with_title("SESSION")
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

        let text = self.palette.get(text_color_role());
        let bright = self.palette.get(UiColorRole::Bright);
        let vivid = self.palette.get(UiColorRole::Vivid);
        let dim = self.palette.get(UiColorRole::Dimmest);
        let width = self.content_width() as usize;
        let (_, content_y) = PanelChrome::content_origin();

        // Status row at the top of the content area: the synced truth, never
        // invented.
        push_line(
            &mut cells,
            content_y + self.content_height() - 1,
            &state.status_label(),
            bright,
            width,
        );

        if state.in_session {
            // Roster rows: entry zero is the host (crowned), you marked.
            let fit = (self.content_height() - 1 - ROW_ROSTER_BASE).max(0) as usize;
            for (index, member) in state.roster.iter().take(fit).enumerate() {
                let row_y = content_y + ROW_ROSTER_BASE + index as i32;
                let color = CellColor::Flat([
                    member.color[0] as f32 / 255.0,
                    member.color[1] as f32 / 255.0,
                    member.color[2] as f32 / 255.0,
                    1.0,
                ]);
                push_text(&mut cells, content_x_of(1), row_y, '●', color);
                let mut x = 3;
                if member.is_host {
                    push_text(&mut cells, content_x_of(x), row_y, '♛', bright);
                    x += 1;
                }
                let mut label = member.display_name.clone();
                if member.is_you {
                    label.push_str(" (you)");
                }
                for glyph in truncate_to_width(&label, width.saturating_sub(x as usize)).chars() {
                    push_text(&mut cells, content_x_of(x), row_y, glyph, text);
                    x += 1;
                }
            }

            // Buttons row: host copies the invite, everyone can leave.
            if state.is_host {
                push_line(
                    &mut cells,
                    content_y + ROW_BUTTONS,
                    "[COPY INVITE]  [LEAVE]",
                    bright,
                    width,
                );
            } else {
                push_line(
                    &mut cells,
                    content_y + ROW_BUTTONS,
                    "[LEAVE]",
                    bright,
                    width,
                );
            }

            // Name edit row: committed with Enter through the key-capture seam.
            let (name_text, name_color) = if self.focus == FieldFocus::DisplayName {
                (format!("{}_", self.name_draft), vivid)
            } else {
                (state.self_display_name.clone(), text)
            };
            push_line(
                &mut cells,
                content_y + ROW_NAME,
                &format!("NAME> {name_text}"),
                name_color,
                width,
            );
        } else {
            // Offline: one-click host, one-field join, name edit.
            push_line(
                &mut cells,
                content_y + ROW_HOST,
                "> HOST SESSION",
                bright,
                width,
            );
            let (join_text, join_color) = if self.focus == FieldFocus::JoinAddress {
                (format!("{}_", self.join_draft), vivid)
            } else if self.join_draft.is_empty() {
                ("ip:port + ENTER".to_string(), dim)
            } else {
                (self.join_draft.clone(), text)
            };
            push_line(
                &mut cells,
                content_y + ROW_JOIN,
                &format!("JOIN> {join_text}"),
                join_color,
                width,
            );
            let (name_text, name_color) = if self.focus == FieldFocus::DisplayName {
                (format!("{}_", self.name_draft), vivid)
            } else {
                (state.self_display_name.clone(), text)
            };
            push_line(
                &mut cells,
                content_y + ROW_NAME,
                &format!("NAME> {name_text}"),
                name_color,
                width,
            );
        }

        // Event lines: the newest three, newest lowest.
        for row in ROW_EVENTS_BASE..ROW_EVENTS_BASE + EVENT_LINES as i32 {
            if let Some((line, color)) = self.event_text(&state.events, row) {
                push_line(&mut cells, content_y + row, &line, color, width);
            }
        }

        CellGroup::from_cells(
            WorldPoint {
                x: self.rect.x0,
                y: self.rect.y0,
                z: 0,
            },
            cells,
        )
        .with_intake_behavior(CellGroupIntakeBehavior::Flat2d)
    }

    fn on_pointer_event(&mut self, event: ModulePointerEvent) {
        match event {
            ModulePointerEvent::Click { x, y, button } => {
                if self.hidden {
                    return;
                }
                if let Some(outcome) = self.gizmo_state.handle_click(&self.gizmos, self.rect, x, y)
                {
                    if outcome == GizmoClickOutcome::Gizmo(GizmoKind::Close) {
                        self.hidden = true;
                        self.blur_fields();
                    }
                    return;
                }
                if button != ModulePointerButton::Left {
                    return;
                }
                let Some((local_x, local_y)) = self.content_local(x, y) else {
                    return;
                };
                let mut state = self.state.borrow_mut();
                if state.in_session {
                    if local_y == ROW_BUTTONS {
                        if state.is_host && local_x < 14 {
                            state.queue_action(SessionPanelAction::CopyInvite);
                        } else {
                            state.queue_action(SessionPanelAction::LeaveRequested);
                        }
                        self.focus = FieldFocus::None;
                    } else if local_y == ROW_NAME {
                        self.focus = FieldFocus::DisplayName;
                        self.name_draft = state.self_display_name.clone();
                    } else if self
                        .roster_row_at(self.visible_roster_rows(), local_y)
                        .is_none()
                    {
                        self.focus = FieldFocus::None;
                    }
                } else {
                    if local_y == ROW_HOST {
                        state.queue_action(SessionPanelAction::HostRequested);
                        self.focus = FieldFocus::None;
                    } else if local_y == ROW_JOIN {
                        self.focus = FieldFocus::JoinAddress;
                    } else if local_y == ROW_NAME {
                        self.focus = FieldFocus::DisplayName;
                        self.name_draft = state.self_display_name.clone();
                    } else {
                        self.focus = FieldFocus::None;
                    }
                }
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

    /// Focused-field key entry through the registry's key-capture seam.
    /// Consumes keys only while a field is focused; otherwise every key
    /// falls through to normal binding dispatch.
    fn on_key_capture(&mut self, label: &str) -> bool {
        if self.hidden {
            return false;
        }
        match self.focus {
            FieldFocus::None => false,
            FieldFocus::JoinAddress => {
                match label {
                    "ENTER" => {
                        let address = self.join_draft.trim().to_string();
                        self.join_draft.clear();
                        self.focus = FieldFocus::None;
                        if !address.is_empty() {
                            self.state
                                .borrow_mut()
                                .queue_action(SessionPanelAction::JoinRequested { address });
                        }
                    }
                    "ESCAPE" => {
                        self.join_draft.clear();
                        self.focus = FieldFocus::None;
                    }
                    "BACKSPACE" => {
                        self.join_draft.pop();
                    }
                    "SPACE" => {}
                    single if single.chars().count() == 1 => {
                        if self.join_draft.chars().count() < 21 {
                            self.join_draft.push_str(single);
                        }
                    }
                    _ => {}
                }
                true
            }
            FieldFocus::DisplayName => {
                match label {
                    "ENTER" => {
                        let name = self.name_draft.trim().to_string();
                        self.name_draft.clear();
                        self.focus = FieldFocus::None;
                        if !name.is_empty() && name != self.state.borrow().self_display_name {
                            self.state
                                .borrow_mut()
                                .queue_action(SessionPanelAction::SetDisplayName(name));
                        }
                    }
                    "ESCAPE" => {
                        self.name_draft.clear();
                        self.focus = FieldFocus::None;
                    }
                    "BACKSPACE" => {
                        self.name_draft.pop();
                    }
                    "SPACE" => {
                        if self.name_draft.chars().count() < 20 {
                            self.name_draft.push(' ');
                        }
                    }
                    single if single.chars().count() == 1 => {
                        if self.name_draft.chars().count() < 20 {
                            self.name_draft.push_str(single);
                        }
                    }
                    _ => {}
                }
                true
            }
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
        if hidden {
            self.blur_fields();
        }
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

/// Pushes content-column `x` into chrome-local cell space (content columns
/// are offset one cell in from the content origin for the text gutter).
fn content_x_of(content_x: i32) -> i32 {
    PanelChrome::content_origin().0 + content_x
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: i32, y0: i32, x1: i32, y1: i32) -> ModuleRect {
        ModuleRect { x0, y0, x1, y1 }
    }

    fn state() -> Rc<RefCell<SessionPanelState>> {
        Rc::new(RefCell::new(SessionPanelState::default()))
    }

    fn panel(state: Rc<RefCell<SessionPanelState>>) -> SessionPanelModule {
        // h=17, w=24: content 22x14 at content_origin.
        SessionPanelModule::new("panel", rect(0, 0, 24, 17), state)
    }

    fn content_click(module: &mut SessionPanelModule, local_x: i32, local_y: i32) {
        let (origin_x, origin_y) = PanelChrome::content_origin();
        module.on_pointer_event(ModulePointerEvent::Click {
            x: origin_x + local_x,
            y: origin_y + local_y,
            button: ModulePointerButton::Left,
        });
    }

    #[test]
    fn status_label_reflects_only_synced_truth() {
        let state = state();

        assert_eq!(state.borrow().status_label(), "OFFLINE");

        state.borrow_mut().in_session = true;
        state.borrow_mut().is_host = true;
        state.borrow_mut().invite_addresses = vec!["192.168.1.5:4747".into()];
        assert_eq!(state.borrow().status_label(), "HOSTING 192.168.1.5:4747");

        state.borrow_mut().is_host = false;
        state.borrow_mut().peer_count = 3;
        assert_eq!(state.borrow().status_label(), "CONNECTED (3)");

        state.borrow_mut().connected = false;
        state.borrow_mut().reconnecting = true;
        assert_eq!(state.borrow().status_label(), "RECONNECTING...");

        state.borrow_mut().reconnecting = false;
        state.borrow_mut().session_ended = true;
        assert_eq!(state.borrow().status_label(), "ENDED");
    }

    #[test]
    fn hidden_panel_draws_nothing_and_ignores_clicks() {
        let state = state();
        let mut module = panel(state.clone());
        module.set_hidden(true);

        let group = module.draw();
        assert!(group.iter_cells().next().is_none());

        content_click(&mut module, 3, ROW_HOST);
        assert!(state.borrow_mut().take_pending_action().is_none());
        // Keys fall through while hidden.
        assert!(!module.on_key_capture("1"));
    }

    #[test]
    fn draws_standard_gizmo_bar_and_session_title() {
        let state = state();
        let module = panel(state);
        let group = module.draw();

        let height = 17;
        let glyph_at = |x: i32| -> Option<char> {
            group
                .get(CellPoint {
                    x,
                    y: height - 1,
                    z: 0,
                })
                .map(|cell| match cell.graphic {
                    CellGraphic::Glyph(glyph) => glyph,
                    _ => ' ',
                })
        };
        assert_eq!(glyph_at(1), Some('#'));
        assert_eq!(glyph_at(3), Some('X'));
        assert_eq!(glyph_at(5), Some('╋'));
        assert_eq!(glyph_at(7), Some('S'));
    }

    #[test]
    fn close_gizmo_hides_the_panel() {
        let state = state();
        let mut module = panel(state);
        module.on_pointer_event(ModulePointerEvent::Click {
            x: 1 + 2, // close gizmo column
            y: 16,    // top border row
            button: ModulePointerButton::Left,
        });
        assert!(module.is_hidden());
    }

    #[test]
    fn host_button_queues_one_action_per_frame() {
        let state = state();
        let mut module = panel(state.clone());

        // One click, one action; the take clears it.
        content_click(&mut module, 3, ROW_HOST);
        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(SessionPanelAction::HostRequested)
        );
        assert!(state.borrow_mut().take_pending_action().is_none());

        // A second click requeues.
        content_click(&mut module, 3, ROW_HOST);
        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(SessionPanelAction::HostRequested)
        );
    }

    #[test]
    fn join_field_takes_keys_until_enter_commits_the_address() {
        let state = state();
        let mut module = panel(state.clone());
        content_click(&mut module, 3, ROW_JOIN); // focus the join field

        // Unfocused keys fall through before focus; focused keys are consumed.
        for label in [
            "1", "9", "2", ".", "1", "6", "8", ".", "1", ".", "5", ":", "4", "7", "4", "7",
        ] {
            assert!(module.on_key_capture(label));
        }
        assert!(module.on_key_capture("ENTER"));

        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(SessionPanelAction::JoinRequested {
                address: "192.168.1.5:4747".into()
            })
        );
        // Focus cleared: keys fall through again.
        assert!(!module.on_key_capture("A"));
    }

    #[test]
    fn join_field_escape_blurs_without_queueing() {
        let state = state();
        let mut module = panel(state.clone());
        content_click(&mut module, 3, ROW_JOIN);
        assert!(module.on_key_capture("1"));
        assert!(module.on_key_capture("ESCAPE"));

        assert!(state.borrow_mut().take_pending_action().is_none());
        assert!(!module.on_key_capture("2"));
    }

    #[test]
    fn name_commit_queues_rename_only_when_changed() {
        let state = state();
        state.borrow_mut().self_display_name = "J".into();
        let mut module = panel(state.clone());

        content_click(&mut module, 3, ROW_NAME); // focus the name field
                                                 // The draft starts as the current name; typing appends to it.
        for label in ["J", "2"] {
            assert!(module.on_key_capture(label));
        }
        assert!(module.on_key_capture("ENTER"));
        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(SessionPanelAction::SetDisplayName("JJ2".into()))
        );

        // Committing the unchanged name queues nothing.
        content_click(&mut module, 3, ROW_NAME);
        assert!(module.on_key_capture("ENTER"));
        assert!(state.borrow_mut().take_pending_action().is_none());
    }

    #[test]
    fn leave_and_copy_buttons_route_by_side() {
        let state = state();
        state.borrow_mut().in_session = true;

        // Client side: one leave button.
        let mut module = panel(state.clone());
        content_click(&mut module, 3, ROW_BUTTONS);
        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(SessionPanelAction::LeaveRequested)
        );

        // Host side: the left half copies, the right half leaves.
        state.borrow_mut().is_host = true;
        content_click(&mut module, 3, ROW_BUTTONS);
        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(SessionPanelAction::CopyInvite)
        );
        content_click(&mut module, 16, ROW_BUTTONS);
        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(SessionPanelAction::LeaveRequested)
        );
    }

    #[test]
    fn roster_rows_show_crown_and_you_marker_with_presence_colors() {
        let state = state();
        state.borrow_mut().in_session = true;
        state.borrow_mut().roster = vec![
            SessionRosterRow {
                user_id: "host-1".into(),
                display_name: "The Host".into(),
                is_host: true,
                is_you: false,
                color: [255, 0, 0],
            },
            SessionRosterRow {
                user_id: "me-1".into(),
                display_name: "Me".into(),
                is_host: false,
                is_you: true,
                color: [0, 255, 0],
            },
        ];
        let module = panel(state.clone());
        let group = module.draw();

        let (origin_x, origin_y) = PanelChrome::content_origin();
        // Presence dot color rides the real synced color.
        let host_dot = group
            .get(CellPoint {
                x: origin_x + 1,
                y: origin_y + ROW_ROSTER_BASE,
                z: 0,
            })
            .unwrap();
        assert_eq!(host_dot.color, CellColor::Flat([1.0, 0.0, 0.0, 1.0]));
        // Crown on the host row, "(you)" on your row.
        let crown = group.get(CellPoint {
            x: origin_x + 3,
            y: origin_y + ROW_ROSTER_BASE,
            z: 0,
        });
        assert!(crown.is_some());
        let your_row = origin_y + ROW_ROSTER_BASE + 1;
        let yours: String = (1..6)
            .filter_map(|x| {
                group
                    .get(CellPoint {
                        x: origin_x + x,
                        y: your_row,
                        z: 0,
                    })
                    .map(|cell| match cell.graphic {
                        CellGraphic::Glyph(glyph) => glyph.to_string(),
                        _ => String::new(),
                    })
            })
            .collect();
        assert_eq!(yours, "● Me ");
    }

    #[test]
    fn events_render_newest_lowest_and_verbatim() {
        let state = state();
        state.borrow_mut().push_event("joined bob");
        state.borrow_mut().push_event("join denied: session-ended");
        let module = panel(state.clone());
        let group = module.draw();

        let (origin_x, origin_y) = PanelChrome::content_origin();
        let text_at = |y: i32| -> String {
            // Content columns 1..=22; the right border lives at rect.x1.
            (1..23)
                .filter_map(|x| {
                    group
                        .get(CellPoint {
                            x: origin_x + x,
                            y: origin_y + y,
                            z: 0,
                        })
                        .map(|cell| match cell.graphic {
                            CellGraphic::Glyph(glyph) => glyph.to_string(),
                            _ => String::new(),
                        })
                })
                .collect()
        };
        assert_eq!(text_at(ROW_EVENTS_BASE + 1).trim_end(), "joined bob");
        assert_eq!(
            text_at(ROW_EVENTS_BASE).trim_end(),
            "join denied: session-e"
        ); // truncated to the content width, verbatim otherwise
    }

    #[test]
    fn event_log_is_capped_at_three_lines() {
        let mut state = SessionPanelState::default();
        state.push_event("one");
        state.push_event("two");
        state.push_event("three");
        state.push_event("four");
        assert_eq!(state.events, vec!["two", "three", "four"]);
    }

    #[test]
    fn gizmo_move_drag_relocates_the_panel_rect() {
        let state = state();
        let mut module = panel(state);
        // Click the move gizmo (top border row), then drag.
        module.on_pointer_event(ModulePointerEvent::Click {
            x: 1,
            y: 16,
            button: ModulePointerButton::Left,
        });
        module.on_pointer_event(ModulePointerEvent::Move { x: 6, y: 12 });
        module.on_pointer_event(ModulePointerEvent::Up { x: 6, y: 12 });

        assert_eq!(module.rect(), rect(5, -4, 29, 13));
    }

    #[test]
    fn persisted_state_round_trips_rect_seamless_and_hidden() {
        let state = state();
        let mut module = panel(Rc::clone(&state));
        module.set_hidden(true);
        module.gizmo_state.set_seamless(true);

        let persisted = module.persisted_ui_state().unwrap();
        let mut restored = panel(state);
        restored.apply_persisted_ui_state(&persisted);

        assert!(restored.is_hidden());
        assert!(restored.gizmo_state.is_seamless());
        assert_eq!(restored.rect(), rect(0, 0, 24, 17));
    }
}

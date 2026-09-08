//! Session panel: the user surface for LAN multiplayer (the session-ui plan's
//! view + intent module). Two `Module` impls share one `SessionPanelState`:
//! a one-row always-attached chip whose label IS the session status, and the
//! click-open detail panel (roster, invite copy, join field, name edit).
//!
//! Ownership rule: the modules never touch `SessionNet`/`SessionHost`/
//! `SessionClient`/sockets. They render only what `SessionPanelState::sync`
//! was told — every displayed state is real synced state, never invented —
//! and emit at most one `SessionPanelAction` per frame via
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
    Cell, CellColor, CellGraphic, CellGroup, CellGroupIntakeBehavior, CellPoint, Module,
    ModulePointerButton, ModulePointerEvent, ModuleRect, PanelChrome, UiColorRole, UiPalette,
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

/// Shared state between the chip, the panel, and their orchestration caller.
/// The modules only ever read/write this struct — no document or socket.
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
    /// The panel's open flag, toggled by the chip click. The orchestration
    /// layer applies it to the registry each frame (`set_hidden`).
    panel_open: bool,
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

    /// Whether the detail panel should currently be shown.
    pub fn panel_open(&self) -> bool {
        self.panel_open
    }

    /// Chip click: open when closed, close when open.
    pub fn toggle_panel(&mut self) {
        self.panel_open = !self.panel_open;
    }

    /// The chip's one-line status, derived only from synced fields.
    pub fn chip_label(&self) -> String {
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
    /// frame by the orchestration layer; events and the panel-open flag are
    /// the panel's own memory and are never touched here.
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
    for (index, glyph) in truncate_to_width(line, width).chars().enumerate() {
        push_text(cells, index as i32 + 1, y, glyph, color);
    }
}

/// The one-row always-attached session status chip. Click opens the panel.
pub struct SessionChipModule {
    id: String,
    rect: ModuleRect,
    state: Rc<RefCell<SessionPanelState>>,
    palette: UiPalette,
}

impl SessionChipModule {
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
        }
    }

    pub fn with_palette(mut self, palette: UiPalette) -> Self {
        self.palette = palette;
        self
    }
}

impl Module for SessionChipModule {
    fn id(&self) -> &str {
        &self.id
    }

    fn rect(&self) -> ModuleRect {
        self.rect
    }

    fn draw(&self) -> CellGroup {
        let state = self.state.borrow();
        let label = state.chip_label();
        let width = (self.rect.x1 - self.rect.x0) as usize;
        let label = truncate_to_width(&label, width.saturating_sub(2));
        let active = state.in_session && !state.session_ended;
        let dot_glyph = if active { '●' } else { '○' };
        let dot_color = if state.session_ended || state.reconnecting {
            self.palette.get(UiColorRole::Vivid)
        } else if active {
            self.palette.get(UiColorRole::Bright)
        } else {
            self.palette.get(UiColorRole::Dimmest)
        };
        let text_color = self.palette.get(text_color_role());

        let mut cells = Vec::new();
        cells.push(Cell {
            position: CellPoint { x: 0, y: 0, z: 0 },
            graphic: CellGraphic::Glyph(dot_glyph),
            color: dot_color,
            ..Cell::default()
        });
        for (index, glyph) in format!(" {label}").chars().enumerate() {
            cells.push(Cell {
                position: CellPoint {
                    x: index as i32 + 1,
                    y: 0,
                    z: 0,
                },
                graphic: CellGraphic::Glyph(glyph),
                color: text_color,
                ..Cell::default()
            });
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
        let ModulePointerEvent::Click {
            button: ModulePointerButton::Left,
            ..
        } = event
        else {
            return;
        };
        self.state.borrow_mut().toggle_panel();
    }
}

/// The click-open session detail panel: roster, invite copy, join field,
/// name edit, and the log-lite event lines. Draws and hit-tests nothing
/// while closed.
pub struct SessionPanelModule {
    id: String,
    rect: ModuleRect,
    state: Rc<RefCell<SessionPanelState>>,
    palette: UiPalette,
    focus: FieldFocus,
    join_draft: String,
    name_draft: String,
}

/// Roster rows the panel can show; the rest scroll off (v1 keeps the panel
/// small — more users than this is past LAN-v1 honesty anyway).
const MAX_ROSTER_ROWS: usize = 8;

/// Local content rows (bottom-up) shared by draw and hit-test.
const ROW_EVENTS_BASE: i32 = 0;
const ROW_NAME: i32 = 3;
const ROW_BUTTONS: i32 = 4;
const ROW_ROSTER_BASE: i32 = 5;

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
            focus: FieldFocus::None,
            join_draft: String::new(),
            name_draft: String::new(),
        }
    }

    pub fn with_palette(mut self, palette: UiPalette) -> Self {
        self.palette = palette;
        self
    }

    fn content_width(&self) -> i32 {
        self.rect.x1 - self.rect.x0 - 1
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

    fn roster_row_at(&self, count: usize, row: i32) -> Option<usize> {
        let index = (row - ROW_ROSTER_BASE) as usize;
        (index < count).then_some(index)
    }
}

impl Module for SessionPanelModule {
    fn id(&self) -> &str {
        &self.id
    }

    fn rect(&self) -> ModuleRect {
        self.rect
    }

    fn draw(&self) -> CellGroup {
        let state = self.state.borrow();
        if !state.panel_open {
            return CellGroup::new(WorldPoint {
                x: self.rect.x0,
                y: self.rect.y0,
                z: 0,
            })
            .with_intake_behavior(CellGroupIntakeBehavior::Flat2d);
        }

        let chrome = PanelChrome::new(self.rect, &self.palette).with_title("SESSION");
        let mut group = CellGroup::from_cells(
            WorldPoint {
                x: self.rect.x0,
                y: self.rect.y0,
                z: 0,
            },
            chrome.cells(),
        )
        .with_intake_behavior(CellGroupIntakeBehavior::Flat2d);

        let text = self.palette.get(text_color_role());
        let bright = self.palette.get(UiColorRole::Bright);
        let vivid = self.palette.get(UiColorRole::Vivid);
        let dim = self.palette.get(UiColorRole::Dimmest);

        let mut cells = Vec::new();
        let width = self.content_width() as usize;

        if state.in_session {
            // Roster rows: entry zero is the host (crowned), you marked.
            let visible = state.roster.len().min(MAX_ROSTER_ROWS);
            for (index, member) in state.roster.iter().take(visible).enumerate() {
                let row_y = ROW_ROSTER_BASE + index as i32;
                let color = CellColor::Flat([
                    member.color[0] as f32 / 255.0,
                    member.color[1] as f32 / 255.0,
                    member.color[2] as f32 / 255.0,
                    1.0,
                ]);
                push_text(&mut cells, 1, row_y, '●', color);
                let mut x = 3;
                if member.is_host {
                    push_text(&mut cells, x, row_y, '♛', bright);
                    x += 1;
                }
                let mut label = member.display_name.clone();
                if member.is_you {
                    label.push_str(" (you)");
                }
                for glyph in truncate_to_width(&label, width.saturating_sub(x as usize)).chars() {
                    push_text(&mut cells, x, row_y, glyph, text);
                    x += 1;
                }
            }

            // Buttons row: host copies the invite, everyone can leave.
            if state.is_host {
                push_line(
                    &mut cells,
                    ROW_BUTTONS,
                    "[COPY INVITE]  [LEAVE]",
                    bright,
                    width,
                );
            } else {
                push_line(&mut cells, ROW_BUTTONS, "[LEAVE]", bright, width);
            }

            // Name edit row: committed with Enter through the key-capture seam.
            let (name_text, name_color) = if self.focus == FieldFocus::DisplayName {
                (format!("{}_", self.name_draft), vivid)
            } else {
                (state.self_display_name.clone(), text)
            };
            push_line(
                &mut cells,
                ROW_NAME,
                &format!("NAME> {name_text}"),
                name_color,
                width,
            );
        } else {
            // Offline: one-click host, one-field join, name edit.
            push_line(&mut cells, 13, "> HOST SESSION", bright, width);
            let (join_text, join_color) = if self.focus == FieldFocus::JoinAddress {
                (format!("{}_", self.join_draft), vivid)
            } else if self.join_draft.is_empty() {
                ("ip:port + ENTER".to_string(), dim)
            } else {
                (self.join_draft.clone(), text)
            };
            push_line(
                &mut cells,
                11,
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
                8,
                &format!("NAME> {name_text}"),
                name_color,
                width,
            );
        }

        // Event lines: the newest three, newest lowest.
        for row in ROW_EVENTS_BASE..ROW_EVENTS_BASE + EVENT_LINES as i32 {
            if let Some((line, color)) = self.event_text(&state.events, row) {
                push_line(&mut cells, row, &line, color, width);
            }
        }

        group.extend(cells);
        group
    }

    fn on_pointer_event(&mut self, event: ModulePointerEvent) {
        let ModulePointerEvent::Click { x, y, button } = event else {
            return;
        };
        if button != ModulePointerButton::Left {
            return;
        }
        let local_x = x - self.rect.x0;
        let local_y = y - self.rect.y0;
        let mut state = self.state.borrow_mut();
        if !state.panel_open {
            return;
        }
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
                .roster_row_at(state.roster.len().min(MAX_ROSTER_ROWS), local_y)
                .is_none()
            {
                self.focus = FieldFocus::None;
            }
        } else {
            if local_y == 13 {
                state.queue_action(SessionPanelAction::HostRequested);
                self.focus = FieldFocus::None;
            } else if local_y == 11 {
                self.focus = FieldFocus::JoinAddress;
            } else if local_y == 8 {
                self.focus = FieldFocus::DisplayName;
                self.name_draft = state.self_display_name.clone();
            } else {
                self.focus = FieldFocus::None;
            }
        }
    }

    /// Focused-field key entry through the registry's key-capture seam.
    /// Consumes keys only while a field is focused; otherwise every key
    /// falls through to normal binding dispatch.
    fn on_key_capture(&mut self, label: &str) -> bool {
        if !self.state.borrow().panel_open {
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

    fn click(module: &mut dyn Module, x: i32, y: i32) {
        module.on_pointer_event(ModulePointerEvent::Click {
            x,
            y,
            button: ModulePointerButton::Left,
        });
    }

    #[test]
    fn chip_label_reflects_only_synced_truth() {
        let state = state();
        let chip = SessionChipModule::new("chip", rect(0, 0, 30, 0), state.clone());

        assert_eq!(state.borrow().chip_label(), "OFFLINE");

        state.borrow_mut().in_session = true;
        state.borrow_mut().is_host = true;
        state.borrow_mut().invite_addresses = vec!["192.168.1.5:4747".into()];
        assert_eq!(state.borrow().chip_label(), "HOSTING 192.168.1.5:4747");

        state.borrow_mut().is_host = false;
        state.borrow_mut().peer_count = 3;
        assert_eq!(state.borrow().chip_label(), "CONNECTED (3)");

        state.borrow_mut().connected = false;
        state.borrow_mut().reconnecting = true;
        assert_eq!(state.borrow().chip_label(), "RECONNECTING...");

        state.borrow_mut().reconnecting = false;
        state.borrow_mut().session_ended = true;
        assert_eq!(state.borrow().chip_label(), "ENDED");
    }

    #[test]
    fn chip_click_toggles_the_panel_open_flag() {
        let state = state();
        let mut chip = SessionChipModule::new("chip", rect(0, 0, 30, 0), state.clone());

        assert!(!state.borrow().panel_open());
        click(&mut chip, 0, 0);
        assert!(state.borrow().panel_open());
        click(&mut chip, 5, 0);
        assert!(!state.borrow().panel_open());
    }

    #[test]
    fn chip_draws_the_dot_and_label() {
        let state = state();
        let chip = SessionChipModule::new("chip", rect(0, 0, 30, 0), state.clone());
        let group = chip.draw();

        let dot = group.get(CellPoint { x: 0, y: 0, z: 0 }).unwrap();
        assert_eq!(dot.graphic, CellGraphic::Glyph('○'));

        state.borrow_mut().in_session = true;
        let group = chip.draw();
        let dot = group.get(CellPoint { x: 0, y: 0, z: 0 }).unwrap();
        assert_eq!(dot.graphic, CellGraphic::Glyph('●'));
        // "CONNECTED (n)" with zero peers.
        let first = group.get(CellPoint { x: 1, y: 0, z: 0 }).unwrap();
        assert_eq!(first.graphic, CellGraphic::Glyph(' '));
        let c = group.get(CellPoint { x: 2, y: 0, z: 0 }).unwrap();
        assert_eq!(c.graphic, CellGraphic::Glyph('C'));
    }

    #[test]
    fn closed_panel_draws_nothing_and_ignores_clicks() {
        let state = state();
        let mut panel = SessionPanelModule::new("panel", rect(0, 0, 24, 17), state.clone());

        let group = panel.draw();
        assert!(group.iter_cells().next().is_none());

        click(&mut panel, 3, 13);
        assert!(state.borrow_mut().take_pending_action().is_none());
    }

    #[test]
    fn host_button_queues_one_action_per_frame() {
        let state = state();
        state.borrow_mut().toggle_panel(); // open
        let mut panel = SessionPanelModule::new("panel", rect(0, 0, 24, 17), state.clone());

        // One click, one action; the take clears it.
        click(&mut panel, 3, 13);
        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(SessionPanelAction::HostRequested)
        );
        assert!(state.borrow_mut().take_pending_action().is_none());

        // A second click requeues.
        click(&mut panel, 3, 13);
        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(SessionPanelAction::HostRequested)
        );
    }

    #[test]
    fn join_field_takes_keys_until_enter_commits_the_address() {
        let state = state();
        state.borrow_mut().toggle_panel();
        let mut panel = SessionPanelModule::new("panel", rect(0, 0, 24, 17), state.clone());
        click(&mut panel, 3, 11); // focus the join field

        // Unfocused keys fall through before focus; focused keys are consumed.
        for label in [
            "1", "9", "2", ".", "1", "6", "8", ".", "1", ".", "5", ":", "4", "7", "4", "7",
        ] {
            assert!(panel.on_key_capture(label));
        }
        assert!(panel.on_key_capture("ENTER"));

        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(SessionPanelAction::JoinRequested {
                address: "192.168.1.5:4747".into()
            })
        );
        // Focus cleared: keys fall through again.
        assert!(!panel.on_key_capture("A"));
    }

    #[test]
    fn join_field_escape_blurs_without_queueing() {
        let state = state();
        state.borrow_mut().toggle_panel();
        let mut panel = SessionPanelModule::new("panel", rect(0, 0, 24, 17), state.clone());
        click(&mut panel, 3, 11);
        assert!(panel.on_key_capture("1"));
        assert!(panel.on_key_capture("ESCAPE"));

        assert!(state.borrow_mut().take_pending_action().is_none());
        assert!(!panel.on_key_capture("2"));
    }

    #[test]
    fn name_commit_queues_rename_only_when_changed() {
        let state = state();
        state.borrow_mut().toggle_panel();
        state.borrow_mut().self_display_name = "J".into();
        let mut panel = SessionPanelModule::new("panel", rect(0, 0, 24, 17), state.clone());

        click(&mut panel, 3, 8); // focus the name field (offline row)
                                 // The draft starts as the current name; typing appends to it.
        for label in ["J", "2"] {
            assert!(panel.on_key_capture(label));
        }
        assert!(panel.on_key_capture("ENTER"));
        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(SessionPanelAction::SetDisplayName("JJ2".into()))
        );

        // Committing the unchanged name queues nothing.
        click(&mut panel, 3, 8);
        assert!(panel.on_key_capture("ENTER"));
        assert!(state.borrow_mut().take_pending_action().is_none());
    }

    #[test]
    fn leave_and_copy_buttons_route_by_side() {
        let state = state();
        state.borrow_mut().toggle_panel();
        state.borrow_mut().in_session = true;

        // Client side: one leave button.
        let mut panel = SessionPanelModule::new("panel", rect(0, 0, 24, 17), state.clone());
        click(&mut panel, 3, ROW_BUTTONS);
        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(SessionPanelAction::LeaveRequested)
        );

        // Host side: the left half copies, the right half leaves.
        state.borrow_mut().is_host = true;
        click(&mut panel, 3, ROW_BUTTONS);
        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(SessionPanelAction::CopyInvite)
        );
        click(&mut panel, 16, ROW_BUTTONS);
        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(SessionPanelAction::LeaveRequested)
        );
    }

    #[test]
    fn roster_rows_show_crown_and_you_marker_with_presence_colors() {
        let state = state();
        state.borrow_mut().toggle_panel();
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
        let panel = SessionPanelModule::new("panel", rect(0, 0, 24, 17), state.clone());
        let group = panel.draw();

        // Presence dot color rides the real synced color.
        let host_dot = group
            .get(CellPoint {
                x: 1,
                y: ROW_ROSTER_BASE,
                z: 0,
            })
            .unwrap();
        assert_eq!(host_dot.color, CellColor::Flat([1.0, 0.0, 0.0, 1.0]));
        // Crown on the host row, "(you)" on your row.
        let crown = group.get(CellPoint {
            x: 3,
            y: ROW_ROSTER_BASE,
            z: 0,
        });
        assert!(crown.is_some());
        let your_row = ROW_ROSTER_BASE + 1;
        let yours: String = (1..6)
            .filter_map(|x| {
                group
                    .get(CellPoint {
                        x,
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
        state.borrow_mut().toggle_panel();
        state.borrow_mut().push_event("joined bob");
        state.borrow_mut().push_event("join denied: session-ended");
        let panel = SessionPanelModule::new("panel", rect(0, 0, 24, 17), state.clone());
        let group = panel.draw();

        let text_at = |y: i32| -> String {
            (1..24)
                .filter_map(|x| {
                    group
                        .get(CellPoint { x, y, z: 0 })
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
            "join denied: session-en"
        ); // truncated to the panel width, verbatim otherwise
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
}

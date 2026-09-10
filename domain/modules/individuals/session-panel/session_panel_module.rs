//! Session panel: the user surface for multiplayer (the session-ui plan's
//! view + intent module, v2 lane-visible shape). One standard `Module` over
//! a shared `SessionPanelState`: standard gizmo bar (move / close / resize /
//! seamless) on `PanelChrome` chrome, recallable from the command bar via
//! the registry's hidden flag — the same shape as every other painter panel.
//!
//! v2 design (settled with J): no fallback magic, two visible lanes. The
//! offline state shows both join lanes — `join with code` (relay invite)
//! and `join local` (LAN discovery selector) — plus `host session`. The
//! hosting state shows the net invite code (with a relay note when the net
//! lane degraded) and the connected roster; the connected state shows the
//! roster (host crowned) and leave/end. Hosting is never gated on relay
//! health: the relay is one reachability lane, not a precondition.
//!
//! Ownership rule: the module never touches `SessionNet`/sockets/discovery.
//! It renders only what `SessionPanelState::sync` was told — every displayed
//! state is real synced state, never invented — and emits at most one
//! `SessionPanelAction` per frame via `take_pending_action` for the
//! orchestration layer to apply. Swap the transport, the panel doesn't know;
//! swap the panel, the transport doesn't know.
//!
//! Text entry rides the shared `TextEntryField`
//! (`thaum-renderer/domain/modules/shared/text-entry/`) through the
//! registry's key-capture seam, including clipboard-paste intake: the paste
//! button flags a paste request, the orchestration layer reads the OS
//! clipboard and applies it back through `insert_text`.

use std::cell::RefCell;
use std::rc::Rc;

use thaum_renderer_domain::{
    text_entry::TextEntryField, Cell, CellColor, CellGraphic, CellGroup, CellGroupIntakeBehavior,
    CellPoint, GizmoBar, GizmoClickOutcome, GizmoKind, GizmoState, Hotspot, Module,
    ModulePointerButton, ModulePointerEvent, ModuleRect, PanelChrome, PersistedModuleUiState,
    UiColorRole, UiPalette, WorldPoint,
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

/// One LAN-discovered host as synced from the discovery poller. The panel
/// renders the name; the orchestration layer dials the address verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionDiscoveredHost {
    pub name: String,
    pub address: String,
}

/// A user action requested through the panel, for the orchestration layer to
/// apply onto the real `SessionNet` and reflect back through `sync`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionPanelAction {
    /// Host a session on the current document (both lanes; the orchestration
    /// layer reports any degraded lane through `relay_note`).
    HostRequested,
    /// Join by relay invite code typed/pasted into the code field.
    JoinCodeRequested { code: String },
    /// Join the LAN-discovered host at a dialable address.
    JoinLocalRequested { address: String },
    /// Read the OS clipboard and paste it into the code field
    /// (orchestration-owned clipboard timing, panel-owned intent).
    PasteCodeRequested,
    /// Copy the net invite code to the OS clipboard (orchestration-owned).
    CopyCode,
    /// Leave the session (host leaving ends it for everyone; the
    /// orchestration layer decides by side).
    LeaveRequested,
    /// Commit the edited display name (offline state).
    SetDisplayName(String),
}

/// Which panel field owns the keyboard right now. Module-local truth: the
/// orchestration layer never sees drafts or focus, only committed actions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum FieldFocus {
    #[default]
    None,
    Code,
    DisplayName,
}

/// Shared state between the module and its orchestration caller. The module
/// only ever reads/writes this struct — no document, socket, or discovery.
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
    /// The net invite code while hosting (relay lane), verbatim; `None`
    /// when not hosting or hosting LAN-only.
    pub net_code: Option<String>,
    /// Why the net lane is degraded, shown while hosting (e.g. the relay
    /// dial failed and this host is LAN-only). `None` = nothing degraded.
    pub relay_note: Option<String>,
    /// LAN-discovered hosts for the join-local selector, verbatim from the
    /// discovery poller's newest scan.
    pub discovered: Vec<SessionDiscoveredHost>,
    /// The full roster, host first (the `SessionNet::roster` contract).
    pub roster: Vec<SessionRosterRow>,
    /// The local user's current display name.
    pub self_display_name: String,
    /// The last events (joined / left / dropped / denied / copied), oldest
    /// first. The panel renders the newest three; nothing it was not told.
    pub events: Vec<String>,
    /// The shared join-code field: focus, draft, commit, paste intake.
    pub code_field: TextEntryField,
    pending_action: Option<SessionPanelAction>,
}

/// The event log shown is the newest three lines.
const EVENT_LINES: usize = 3;

/// The join code's shape budget: `<room6>-<token10>` is 17 glyphs; a little
/// headroom stays friendly without inviting essays.
const CODE_FIELD_CHARS: usize = 24;

impl SessionPanelState {
    pub fn new() -> Self {
        Self {
            code_field: TextEntryField::new(CODE_FIELD_CHARS, "code + enter"),
            ..Self::default()
        }
    }

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
            return "HOSTING".to_string();
        }
        if self.in_session {
            return format!("CONNECTED ({})", self.peer_count);
        }
        "OFFLINE".to_string()
    }

    /// Takes the pending action, if any. At most one action queues per frame;
    /// a flagged paste request promotes itself here so the shared field's
    /// paste surface and the action queue stay one seam.
    pub fn take_pending_action(&mut self) -> Option<SessionPanelAction> {
        if self.pending_action.is_none() && self.code_field.take_paste_request() {
            self.pending_action = Some(SessionPanelAction::PasteCodeRequested);
        }
        self.pending_action.take()
    }

    /// Refreshes the session truth from the live session. Called once per
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
        net_code: Option<String>,
        roster: Vec<SessionRosterRow>,
        self_display_name: String,
    ) {
        self.in_session = in_session;
        self.is_host = is_host;
        self.connected = connected;
        self.reconnecting = reconnecting;
        self.session_ended = session_ended;
        self.peer_count = peer_count;
        self.net_code = net_code;
        self.roster = roster;
        self.self_display_name = self_display_name;
    }

    /// Refreshes the net-lane health note (why the net lane is degraded, if
    /// it is). Orchestration-owned memory; cleared by passing `None`.
    pub fn sync_relay_note(&mut self, relay_note: Option<String>) {
        self.relay_note = relay_note;
    }

    /// Refreshes the discovery truth from the newest finished scan. An empty
    /// scan is real truth: "none found".
    pub fn sync_discovered(&mut self, discovered: Vec<SessionDiscoveredHost>) {
        self.discovered = discovered;
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
    for (index, glyph) in truncate_to_width(line, width.saturating_sub(1)).chars().enumerate() {
        push_text(cells, content_x_of(index as i32 + 1), y, glyph, color);
    }
}

/// The click-open session detail panel in standard module form: gizmo-bar
/// chrome, command-bar recall, the two visible join lanes offline, roster +
/// invite/end in-session, and the log-lite event lines.
pub struct SessionPanelModule {
    id: String,
    rect: ModuleRect,
    state: Rc<RefCell<SessionPanelState>>,
    palette: UiPalette,
    gizmos: GizmoBar,
    gizmo_state: GizmoState,
    hidden: bool,
    focus: FieldFocus,
    name_draft: String,
    /// The join-local selector's position in the synced discovery list.
    selector_index: usize,
}

/// Content rows (local to `PanelChrome::content_origin`, bottom-up) shared
/// by draw and hit-test.
const ROW_EVENTS_BASE: i32 = 0;
const ROW_HOST_BTN: i32 = 4;
const ROW_SELECTOR: i32 = 6;
const ROW_JOIN_LOCAL_BTN: i32 = 7;
const ROW_CODE_FIELD: i32 = 9;
const ROW_CODE_BTNS: i32 = 10;
const ROW_CODE_TITLE: i32 = 11;
/// In-session rows: roster climbs from here (hosting leaves room for the
/// code + note rows above).
const ROW_ROSTER_BASE: i32 = 6;
const ROW_BUTTONS: i32 = 4;
const ROW_CODE: i32 = 12;
const ROW_RELAY_NOTE: i32 = 11;

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
            name_draft: String::new(),
            selector_index: 0,
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

    /// Absolute screen-space hotspot over one content-row span (inclusive
    /// content-local x0..x1 at content row `row`).
    fn row_hotspot(
        &self,
        row: i32,
        x0: i32,
        x1: i32,
        title: &str,
        description: &str,
    ) -> Hotspot {
        let (origin_x, origin_y) = PanelChrome::content_origin();
        let y = self.rect.y0 + origin_y + row;
        Hotspot::new(
            ModuleRect {
                x0: self.rect.x0 + origin_x + x0,
                y0: y,
                x1: self.rect.x0 + origin_x + x1.max(x0),
                y1: y,
            },
            title,
            description,
        )
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

    /// The selected discovered host, if the synced list has one at the
    /// (clamped) selector position.
    fn selected_host(&self, state: &SessionPanelState) -> Option<SessionDiscoveredHost> {
        if state.discovered.is_empty() {
            return None;
        }
        let index = self.selector_index.min(state.discovered.len() - 1);
        Some(state.discovered[index].clone())
    }

    /// The highest content row the roster may occupy (below the status row;
    /// hosting stops below the code + note rows).
    fn roster_top(&self, state: &SessionPanelState) -> i32 {
        if state.is_host {
            ROW_RELAY_NOTE - 1
        } else {
            self.content_height() - 2
        }
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

}

impl Module for SessionPanelModule {
    fn id(&self) -> &str {
        &self.id
    }

    fn rect(&self) -> ModuleRect {
        self.rect
    }

    /// Tooltip hotspots: the module's gizmo bar plus one hotspot per custom
    /// control, so every button and field explains itself through the shared
    /// tooltip implementation.
    fn hotspots(&self) -> Vec<Hotspot> {
        let state = self.state.borrow();
        let width = self.content_width() - 1;
        let mut custom = Vec::new();
        if state.in_session {
            if state.is_host {
                // CODE> row: clicking copies the invite code.
                custom.push(self.row_hotspot(
                    ROW_CODE,
                    0,
                    width,
                    "invite code",
                    "click to copy the invite code share it with joiners",
                ));
            }
            // Roster rows: entry zero is the host (crowned), you marked.
            let roster_base = if state.is_host {
                ROW_ROSTER_BASE + 2
            } else {
                ROW_ROSTER_BASE
            };
            let fit = (self.roster_top(&state) - roster_base + 1).max(0) as usize;
            for (index, member) in state.roster.iter().take(fit).enumerate() {
                let label = if member.is_you {
                    format!("{} (you)", member.display_name)
                } else {
                    member.display_name.clone()
                };
                custom.push(self.row_hotspot(
                    roster_base + index as i32,
                    0,
                    width,
                    &label,
                    "session member; the crown marks the host",
                ));
            }
            let button = if state.is_host {
                ("end session", "ends the session for everyone and saves the host's document")
            } else {
                ("leave session", "leaves the session this machine keeps its own edits")
            };
            custom.push(self.row_hotspot(ROW_BUTTONS, 0, width, button.0, button.1));
        } else {
            custom.push(self.row_hotspot(
                ROW_HOST_BTN - 1,
                0,
                width,
                "display name",
                "click to edit the name other users see in the session",
            ));
            custom.push(self.row_hotspot(
                ROW_HOST_BTN,
                0,
                width,
                "host session",
                "starts a session others join by LAN or invite code",
            ));
            custom.push(self.row_hotspot(
                ROW_CODE_FIELD,
                0,
                width,
                "join code",
                "type an invite code here",
            ));
            custom.push(self.row_hotspot(
                ROW_CODE_BTNS,
                0,
                10,
                "join by code",
                "commits the typed invite code",
            ));
            custom.push(self.row_hotspot(
                ROW_CODE_BTNS,
                13,
                19,
                "paste",
                "pastes an invite code from the clipboard",
            ));
            custom.push(self.row_hotspot(
                ROW_SELECTOR,
                0,
                width,
                "host selector",
                "arrows pick which discovered host to join",
            ));
            custom.push(self.row_hotspot(
                ROW_JOIN_LOCAL_BTN,
                0,
                width,
                "join local",
                "joins the selected host found on your network",
            ));
        }
        drop(state);
        self.gizmos.hotspots_with(self.rect, custom)
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
            // Hosting shows the net invite code (click copies) plus the net
            // lane note when the lane degraded.
            let roster_base = if state.is_host {
                if let Some(code) = &state.net_code {
                    push_line(
                        &mut cells,
                        content_y + ROW_CODE,
                        &format!("CODE> {code}"),
                        vivid,
                        width,
                    );
                }
                if let Some(note) = &state.relay_note {
                    push_line(
                        &mut cells,
                        content_y + ROW_RELAY_NOTE,
                        &format!("* {note}"),
                        dim,
                        width,
                    );
                }
                ROW_ROSTER_BASE + 2
            } else {
                ROW_ROSTER_BASE
            };

            // Roster rows: entry zero is the host (crowned), you marked.
            let fit = (self.roster_top(&state) - roster_base + 1).max(0) as usize;
            for (index, member) in state.roster.iter().take(fit).enumerate()
            {
                let row_y = content_y + roster_base + index as i32;
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
                for glyph in truncate_to_width(&label, width.saturating_sub(x as usize + 1)).chars()
                {
                    push_text(&mut cells, content_x_of(x), row_y, glyph, text);
                    x += 1;
                }
            }

            // Buttons row: the host ends, a joiner leaves.
            let button = if state.is_host { "[END SESSION]" } else { "[LEAVE SESSION]" };
            push_line(&mut cells, content_y + ROW_BUTTONS, button, bright, width);
        } else {
            // Offline: both lanes visible, host button below.
            push_line(&mut cells, content_y + ROW_CODE_TITLE, "JOIN WITH CODE", text, width);
            let code_color = if state.code_field.is_focused() {
                vivid
            } else if state.code_field.is_empty() {
                dim
            } else {
                text
            };
            push_line(
                &mut cells,
                content_y + ROW_CODE_FIELD,
                &state.code_field.display(),
                code_color,
                width,
            );
            push_line(
                &mut cells,
                content_y + ROW_CODE_BTNS,
                "[JOIN CODE]  [PASTE]",
                bright,
                width,
            );

            push_line(&mut cells, content_y + ROW_JOIN_LOCAL_BTN - 1, "JOIN LOCAL", text, width);
            match self.selected_host(&state) {
                Some(host) => {
                    // `< name >` selector: the arrows are the click zones.
                    let label = truncate_to_width(&host.name, width.saturating_sub(6));
                    push_text(&mut cells, content_x_of(1), content_y + ROW_SELECTOR, '<', bright);
                    let mut x = 3;
                    for glyph in label.chars() {
                        push_text(&mut cells, content_x_of(x), content_y + ROW_SELECTOR, glyph, text);
                        x += 1;
                    }
                    push_text(
                        &mut cells,
                        content_x_of(x + 1),
                        content_y + ROW_SELECTOR,
                        '>',
                        bright,
                    );
                }
                None => {
                    push_line(
                        &mut cells,
                        content_y + ROW_SELECTOR,
                        "none found",
                        dim,
                        width,
                    );
                }
            }
            push_line(
                &mut cells,
                content_y + ROW_JOIN_LOCAL_BTN,
                "[JOIN LOCAL]",
                bright,
                width,
            );

            push_line(
                &mut cells,
                content_y + ROW_HOST_BTN,
                "[HOST SESSION]",
                bright,
                width,
            );

            // Name edit row rides the events' top edge offline (row 3).
            let (name_text, name_color) = if self.focus == FieldFocus::DisplayName {
                (format!("{}_", self.name_draft), vivid)
            } else {
                (state.self_display_name.clone(), text)
            };
            push_line(
                &mut cells,
                content_y + ROW_HOST_BTN - 1,
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
                        self.name_draft.clear();
                        self.focus = FieldFocus::None;
                        self.state.borrow_mut().code_field.blur();
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
                        state.queue_action(SessionPanelAction::LeaveRequested);
                        self.focus = FieldFocus::None;
                    } else if local_y == ROW_CODE && state.is_host && state.net_code.is_some() {
                        state.queue_action(SessionPanelAction::CopyCode);
                        self.focus = FieldFocus::None;
                    } else {
                        let roster_base = if state.is_host {
                            ROW_ROSTER_BASE + 2
                        } else {
                            ROW_ROSTER_BASE
                        };
                        if state
                            .roster
                            .get((local_y - roster_base) as usize)
                            .is_none()
                        {
                            self.focus = FieldFocus::None;
                        }
                    }
                } else if local_y == ROW_CODE_FIELD {
                    self.focus = FieldFocus::Code;
                    state.code_field.focus();
                } else if local_y == ROW_CODE_BTNS {
                    // Left region: [JOIN CODE] commits the draft; right
                    // region: [PASTE] flags the clipboard request.
                    if local_x <= 11 {
                        state.code_field.handle_key_label("ENTER");
                        if let Some(code) = state.code_field.take_commit() {
                            state.queue_action(SessionPanelAction::JoinCodeRequested { code });
                        }
                        self.focus = FieldFocus::None;
                    } else {
                        state.code_field.request_paste();
                    }
                } else if local_y == ROW_SELECTOR {
                    // Arrow zones scroll the discovery list; the name is the
                    // middle.
                    let count = state.discovered.len();
                    if count > 0 && local_x == 1 {
                        self.selector_index = self.selector_index.saturating_sub(1);
                    } else if count > 0 && local_x >= 3 {
                        let max = count - 1;
                        let name_width = self.selected_host(&state).map_or(0, |host| {
                            host.name.chars().count().min(self.content_width() as usize - 6)
                        });
                        if local_x >= 3 + name_width as i32 + 1 {
                            self.selector_index = (self.selector_index + 1).min(max);
                        } else {
                            self.focus = FieldFocus::None;
                        }
                    } else {
                        self.focus = FieldFocus::None;
                    }
                } else if local_y == ROW_JOIN_LOCAL_BTN {
                    if let Some(host) = self.selected_host(&state) {
                        state.queue_action(SessionPanelAction::JoinLocalRequested {
                            address: host.address,
                        });
                    }
                    self.focus = FieldFocus::None;
                } else if local_y == ROW_HOST_BTN - 1 {
                    self.focus = FieldFocus::DisplayName;
                    self.name_draft = state.self_display_name.clone();
                } else if local_y == ROW_HOST_BTN {
                    state.queue_action(SessionPanelAction::HostRequested);
                    self.focus = FieldFocus::None;
                } else {
                    self.focus = FieldFocus::None;
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
        let mut state = self.state.borrow_mut();
        match self.focus {
            FieldFocus::None => false,
            FieldFocus::Code => {
                let consumed = state.code_field.handle_key_label(label);
                if let Some(code) = state.code_field.take_commit() {
                    state.queue_action(SessionPanelAction::JoinCodeRequested { code });
                    self.focus = FieldFocus::None;
                } else if label == "ESCAPE" {
                    self.focus = FieldFocus::None;
                }
                consumed
            }
            FieldFocus::DisplayName => {
                match label {
                    "ENTER" => {
                        let name = self.name_draft.trim().to_string();
                        self.name_draft.clear();
                        self.focus = FieldFocus::None;
                        if !name.is_empty() && name != state.self_display_name {
                            state.queue_action(SessionPanelAction::SetDisplayName(name));
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
            self.name_draft.clear();
            self.focus = FieldFocus::None;
            self.state.borrow_mut().code_field.blur();
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
        Rc::new(RefCell::new(SessionPanelState::new()))
    }

    fn panel(state: Rc<RefCell<SessionPanelState>>) -> SessionPanelModule {
        // w=28, h=17: content 26x14 at content_origin.
        SessionPanelModule::new("panel", rect(0, 0, 28, 17), state)
    }

    fn content_click(module: &mut SessionPanelModule, local_x: i32, local_y: i32) {
        let (origin_x, origin_y) = PanelChrome::content_origin();
        module.on_pointer_event(ModulePointerEvent::Click {
            x: origin_x + local_x,
            y: origin_y + local_y,
            button: ModulePointerButton::Left,
        });
    }

    fn row_text(module: &SessionPanelModule, row: i32) -> String {
        let (origin_x, origin_y) = PanelChrome::content_origin();
        (1..27)
            .filter_map(|x| {
                module
                    .draw()
                    .get(CellPoint {
                        x: origin_x + x,
                        y: origin_y + row,
                        z: 0,
                    })
                    .map(|cell| match cell.graphic {
                        CellGraphic::Glyph(glyph) => glyph.to_string(),
                        _ => String::new(),
                    })
            })
            .collect()
    }

    #[test]
    fn offline_panel_shows_both_lanes_and_host_button() {
        let state = state();
        let module = panel(state);
        assert_eq!(row_text(&module, ROW_CODE_TITLE).trim_end(), "JOIN WITH CODE");
        assert!(row_text(&module, ROW_CODE_FIELD).contains("code + enter"));
        assert_eq!(row_text(&module, ROW_CODE_BTNS).trim_end(), "[JOIN CODE]  [PASTE]");
        assert_eq!(row_text(&module, ROW_SELECTOR).trim_end(), "none found");
        assert_eq!(row_text(&module, ROW_JOIN_LOCAL_BTN).trim_end(), "[JOIN LOCAL]");
        assert_eq!(row_text(&module, ROW_HOST_BTN).trim_end(), "[HOST SESSION]");
    }

    #[test]
    fn selector_renders_synced_hosts_and_arrows_scroll() {
        let state = state();
        state.borrow_mut().sync_discovered(vec![
            SessionDiscoveredHost { name: "Living Room".into(), address: "192.168.1.5:4747".into() },
            SessionDiscoveredHost { name: "Studio".into(), address: "192.168.1.9:4747".into() },
        ]);
        let mut module = panel(state.clone());

        assert!(row_text(&module, ROW_SELECTOR).contains("Living Room"));
        // Click the right arrow: the second host slides in.
        content_click(&mut module, 16, ROW_SELECTOR);
        assert!(row_text(&module, ROW_SELECTOR).contains("Studio"));
        // Left arrow comes back.
        content_click(&mut module, 1, ROW_SELECTOR);
        assert!(row_text(&module, ROW_SELECTOR).contains("Living Room"));
    }

    #[test]
    fn join_local_requires_a_selected_host() {
        let state = state();
        let mut module = panel(state.clone());
        content_click(&mut module, 3, ROW_JOIN_LOCAL_BTN);
        assert!(state.borrow_mut().take_pending_action().is_none());

        state.borrow_mut().sync_discovered(vec![SessionDiscoveredHost {
            name: "Studio".into(),
            address: "192.168.1.9:4747".into(),
        }]);
        content_click(&mut module, 3, ROW_JOIN_LOCAL_BTN);
        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(SessionPanelAction::JoinLocalRequested {
                address: "192.168.1.9:4747".into()
            })
        );
    }

    #[test]
    fn code_field_takes_keys_until_enter_commits_the_code() {
        let state = state();
        let mut module = panel(state.clone());
        content_click(&mut module, 3, ROW_CODE_FIELD); // focus the code field

        for label in ["A", "B", "C"] {
            assert!(module.on_key_capture(label));
        }
        assert!(module.on_key_capture("ENTER"));

        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(SessionPanelAction::JoinCodeRequested { code: "ABC".into() })
        );
        // Focus cleared: keys fall through again.
        assert!(!module.on_key_capture("A"));
    }

    #[test]
    fn join_code_button_commits_the_typed_draft() {
        let state = state();
        let mut module = panel(state.clone());
        content_click(&mut module, 3, ROW_CODE_FIELD);
        for label in ["A", "B", "C"] {
            module.on_key_capture(label);
        }
        // Click [JOIN CODE] (left region of the buttons row).
        content_click(&mut module, 3, ROW_CODE_BTNS);
        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(SessionPanelAction::JoinCodeRequested { code: "ABC".into() })
        );
    }

    #[test]
    fn paste_button_flags_a_paste_request_action() {
        let state = state();
        let mut module = panel(state.clone());
        content_click(&mut module, 16, ROW_CODE_BTNS); // [PASTE] region
        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(SessionPanelAction::PasteCodeRequested)
        );
        assert!(state.borrow_mut().take_pending_action().is_none());
    }

    #[test]
    fn host_button_queues_one_action_per_frame() {
        let state = state();
        let mut module = panel(state.clone());

        content_click(&mut module, 3, ROW_HOST_BTN);
        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(SessionPanelAction::HostRequested)
        );
        assert!(state.borrow_mut().take_pending_action().is_none());

        content_click(&mut module, 3, ROW_HOST_BTN);
        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(SessionPanelAction::HostRequested)
        );
    }

    #[test]
    fn hosting_shows_code_note_roster_and_end_button() {
        let state = state();
        {
            let mut borrowed = state.borrow_mut();
            borrowed.sync(
                true,
                true,
                true,
                false,
                false,
                1,
                Some("abc123-def456".into()),
                vec![
                    SessionRosterRow {
                        user_id: "host-1".into(),
                        display_name: "The Host".into(),
                        is_host: true,
                        is_you: true,
                        color: [255, 0, 0],
                    },
                    SessionRosterRow {
                        user_id: "peer-1".into(),
                        display_name: "Bob".into(),
                        is_host: false,
                        is_you: false,
                        color: [0, 255, 0],
                    },
                ],
                "The Host".to_string(),
            );
            borrowed.sync_relay_note(Some("relay down — lan joins only".into()));
        }
        let mut module = panel(state.clone());

        assert!(row_text(&module, ROW_CODE).contains("CODE> abc123-def456"));
        assert!(row_text(&module, ROW_RELAY_NOTE).contains("relay down"));
        assert!(row_text(&module, ROW_BUTTONS).trim_end().starts_with("[END SESSION]"));

        // Clicking the code copies it.
        content_click(&mut module, 3, ROW_CODE);
        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(SessionPanelAction::CopyCode)
        );
    }

    #[test]
    fn connected_client_sees_roster_crown_and_leave_button() {
        let state = state();
        state.borrow_mut().sync(
            true,
            false,
            true,
            false,
            false,
            1,
            None,
            vec![
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
            ],
            "Me".to_string(),
        );
        let mut module = panel(state.clone());

        assert!(row_text(&module, ROW_BUTTONS).trim_end().starts_with("[LEAVE SESSION]"));
        content_click(&mut module, 3, ROW_BUTTONS);
        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(SessionPanelAction::LeaveRequested)
        );
    }

    #[test]
    fn status_label_reflects_only_synced_truth() {
        let mut state = SessionPanelState::new();

        assert_eq!(state.status_label(), "OFFLINE");

        state.in_session = true;
        state.is_host = true;
        assert_eq!(state.status_label(), "HOSTING");

        state.is_host = false;
        state.peer_count = 3;
        assert_eq!(state.status_label(), "CONNECTED (3)");

        state.connected = false;
        state.reconnecting = true;
        assert_eq!(state.status_label(), "RECONNECTING...");

        state.reconnecting = false;
        state.session_ended = true;
        assert_eq!(state.status_label(), "ENDED");
    }

    #[test]
    fn hidden_panel_draws_nothing_and_ignores_clicks() {
        let state = state();
        let mut module = panel(state.clone());
        module.set_hidden(true);

        let group = module.draw();
        assert!(group.iter_cells().next().is_none());

        content_click(&mut module, 3, ROW_HOST_BTN);
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
    fn name_commit_queues_rename_only_when_changed() {
        let state = state();
        state.borrow_mut().self_display_name = "J".into();
        let mut module = panel(state.clone());

        content_click(&mut module, 3, ROW_HOST_BTN - 1); // focus the name field
        for label in ["J", "2"] {
            assert!(module.on_key_capture(label));
        }
        assert!(module.on_key_capture("ENTER"));
        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(SessionPanelAction::SetDisplayName("JJ2".into()))
        );

        // Committing the unchanged name queues nothing.
        content_click(&mut module, 3, ROW_HOST_BTN - 1);
        assert!(module.on_key_capture("ENTER"));
        assert!(state.borrow_mut().take_pending_action().is_none());
    }

    #[test]
    fn events_render_newest_lowest_and_verbatim() {
        let state = state();
        state.borrow_mut().push_event("joined bob");
        state.borrow_mut().push_event("join denied: session-ended");
        let module = panel(state.clone());

        assert_eq!(row_text(&module, ROW_EVENTS_BASE + 1).trim_end(), "joined bob");
        assert_eq!(
            row_text(&module, ROW_EVENTS_BASE).trim_end(),
            "join denied: session-ende"
        ); // truncated to the content width, verbatim otherwise
    }

    #[test]
    fn event_log_is_capped_at_three_lines() {
        let mut state = SessionPanelState::new();
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
        module.on_pointer_event(ModulePointerEvent::Click {
            x: 1,
            y: 16,
            button: ModulePointerButton::Left,
        });
        module.on_pointer_event(ModulePointerEvent::Move { x: 6, y: 12 });
        module.on_pointer_event(ModulePointerEvent::Up { x: 6, y: 12 });

        assert_eq!(module.rect(), rect(5, -4, 33, 13));
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
        assert_eq!(restored.rect(), rect(0, 0, 28, 17));
    }
}

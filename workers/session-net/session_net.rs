//! Session net: the entrypoint-facing seam over the session host/client pair.
//!
//! The owning painter app holds one `Option<SessionNet>` and treats hosting
//! and joining identically: `publish(record)` for records it just applied
//! locally, `sync(&mut runtime)` to pull everyone else's records in host
//! order. Hosting mode wraps a `SessionHost` + TCP server; joining mode wraps
//! a `SessionClient`. Neither mode owns the runtime — the app's live runtime
//! stays the single source of truth it applies to.
//!
//! Convergence model (Figma's): the host's snapshot captures the document
//! structure as it was when the server started, and the host log is seeded
//! with the host's pre-host action history — canvas content is runtime state
//! rebuilt purely through replay, so the seed is what lets joiners see
//! everything painted before hosting began. Joiners rebuild by fresh
//! snapshot + full-log replay. Structure edits that still bypass the record
//! log (`document.json` path) are not replayed — that gap closes with the
//! structure-edit record variants in `domain/file/storage/`.

use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::session_client::{SessionClient, SessionClientError};
use crate::session_host::{SessionHost, SessionUser};
use crate::session_host_tcp::{spawn_session_host_server, SessionHostServer};
use thaum_painter_domain::debug_log;
use thaum_painter_domain::storage::{
    SharedDocumentActionRecord, SharedDocumentFile, SharedDocumentRuntime,
};

pub enum SessionNet {
    Host {
        host: Arc<Mutex<SessionHost>>,
        /// Kept for shutdown/drop; the accept loop ends on drop.
        _server: SessionHostServer,
        user_id: String,
        /// The bound listen port — the `invite_addresses()` truth source.
        port: u16,
        /// How many host-log records the local app has consumed into its own
        /// runtime. Starts at the seed length: the pre-host history was
        /// loaded from disk into the local runtime already (that is where
        /// the seed came from), so host sync skips straight past it. Own
        /// records are skipped (applied at publish); foreign records (from
        /// clients) are applied here.
        consumed: usize,
    },
    Client {
        client: SessionClient,
        /// Client-owned auto-rejoin truth: where and who to rejoin as, plus
        /// the current capped backoff. Never used after the host says `Ended`.
        rejoin: ClientRejoin,
    },
}

/// Client-owned rejoin state. Backoff starts at `REJOIN_INITIAL_BACKOFF`,
/// doubles per failed attempt, and caps at `REJOIN_MAX_BACKOFF`. A rejoin is
/// the existing cheap join path: fresh snapshot, full-log replay, runtime
/// rebuilt from the snapshot inside `sync` — no delta machinery.
pub struct ClientRejoin {
    address: String,
    user: SessionUser,
    backoff: Duration,
    next_attempt: Instant,
    /// Consecutive failed rejoin attempts since the last success. Drives the
    /// one-shot firewall hint in the run log — the most common cause of a
    /// client that never stops reconnecting is the host machine's firewall
    /// silently dropping inbound TCP on the listen port.
    failed_attempts: u32,
    /// One-shot flag: the last `sync` performed a rejoin, so the caller's
    /// runtime was rebuilt from a fresh snapshot and must not republish what
    /// it already had. Cleared on read via `SessionNet::take_rejoined`.
    rejoined: bool,
}

const REJOIN_INITIAL_BACKOFF: Duration = Duration::from_millis(500);
const REJOIN_MAX_BACKOFF: Duration = Duration::from_secs(8);

/// After this many consecutive failed rejoin attempts, the run log calls out
/// the host-machine firewall — the dominant real-world cause.
const REJOIN_FIREWALL_HINT_AT: u32 = 3;

impl SessionNet {
    /// Starts hosting: binds the TCP server, registers the snapshot source,
    /// and seeds the host log with the pre-host action history (`runtime
    /// .actions_for_file()` at the call site). The seed is what joiners
    /// replay to rebuild content painted before hosting started.
    pub fn host(
        snapshot_source: Box<dyn Fn() -> SharedDocumentFile + Send>,
        user: SessionUser,
        port: u16,
        seed_records: Vec<SharedDocumentActionRecord>,
    ) -> std::io::Result<Self> {
        let mut host = SessionHost::new();
        host.set_snapshot_source(snapshot_source);
        host.set_host_user(user.clone());
        let consumed = seed_records.len();
        host.seed_log(seed_records);
        let host = Arc::new(Mutex::new(host));
        let server = spawn_session_host_server(Arc::clone(&host), port)?;
        let port = server.port;
        Ok(Self::Host {
            host,
            _server: server,
            user_id: user.user_id,
            port,
            consumed,
        })
    }

    /// Joins a host. Returns the net seam plus the host's snapshot document —
    /// the caller rebuilds its runtime from it before the first sync. Auto-
    /// rejoin is armed from this same address and identity.
    pub fn join(
        address: &str,
        user: SessionUser,
    ) -> Result<(Self, SharedDocumentFile), SessionClientError> {
        let (client, snapshot) = SessionClient::connect(address, user.clone())?;
        Ok((
            Self::Client {
                client,
                rejoin: ClientRejoin {
                    address: address.to_string(),
                    user,
                    backoff: REJOIN_INITIAL_BACKOFF,
                    next_attempt: Instant::now(),
                    failed_attempts: 0,
                    rejoined: false,
                },
            },
            snapshot,
        ))
    }

    /// Host mode: ends the session on purpose. Every connected client
    /// receives `Ended` and further joins are denied. Client mode: nothing —
    /// leaving is the caller dropping the net.
    pub fn end_session(&mut self) {
        if let Self::Host { host, .. } = self {
            host.lock().expect("session host lock").end_session();
        }
    }

    /// Host mode: re-seeds the session after a wholesale document swap
    /// (file:new / file:open while hosting). Installs the new snapshot source,
    /// replaces the host log with the new document's seed records, and evicts
    /// every client — their auto-rejoin rebuilds from the fresh Welcome plus
    /// full replay, so the whole session converges on the new document. The
    /// host-side consume cursor resets with the log. Returns the new publish
    /// cursor: everything already in the caller's runtime is the seed.
    /// Client mode: an error — only the host re-seeds.
    pub fn reseed_host(
        &mut self,
        snapshot_source: Box<dyn Fn() -> SharedDocumentFile + Send>,
        seed_records: Vec<SharedDocumentActionRecord>,
    ) -> Result<usize, String> {
        match self {
            Self::Host { host, consumed, .. } => {
                let mut host = host.lock().expect("session host lock");
                host.set_snapshot_source(snapshot_source);
                host.seed_log(seed_records);
                host.evict_clients();
                *consumed = 0;
                Ok(host.records().len())
            }
            Self::Client { .. } => Err("only the host can re-seed a session".to_string()),
        }
    }

    /// True once the session ended on purpose (host side decided, client
    /// learned `Ended`). A rejoin never runs past this.
    pub fn session_ended(&self) -> bool {
        match self {
            Self::Host { host, .. } => host.lock().expect("session host lock").is_ended(),
            Self::Client { client, .. } => client.session_ended(),
        }
    }

    /// Client mode and dropped but not ended: the chip's `RECONNECTING…` state.
    pub fn is_reconnecting(&self) -> bool {
        match self {
            Self::Host { .. } => false,
            Self::Client { client, .. } => !client.is_connected() && !client.session_ended(),
        }
    }

    /// The current rejoin backoff in ms (client mode, dropped, not ended).
    /// Test exposure for the capped-backoff behavior.
    pub fn rejoin_backoff_ms(&self) -> Option<u64> {
        match self {
            Self::Host { .. } => None,
            Self::Client { client, rejoin } => {
                if client.is_connected() || client.session_ended() {
                    None
                } else {
                    Some(rejoin.backoff.as_millis() as u64)
                }
            }
        }
    }

    /// The addresses a joiner should type to reach this hosted session:
    /// every reachable non-loopback IPv4 interface first, loopback last.
    /// Single source of truth — the panel renders these verbatim and nothing
    /// else computes them. Client mode (and offline hosts) have no invite.
    pub fn invite_addresses(&self) -> Vec<String> {
        let Self::Host { port, .. } = self else {
            return Vec::new();
        };
        let port = *port;
        let mut reachable = Vec::new();
        let mut loopback: Option<String> = None;
        for interface in if_addrs::get_if_addrs().into_iter().flatten() {
            let IpAddr::V4(ip) = interface.ip() else {
                continue;
            };
            let address = format!("{ip}:{port}");
            if ip.is_loopback() {
                loopback = Some(address);
            } else if !reachable.contains(&address) {
                reachable.push(address);
            }
        }
        reachable.extend(loopback);
        reachable
    }

    pub fn user_id(&self) -> &str {
        match self {
            Self::Host { user_id, .. } => user_id,
            Self::Client { client, .. } => &client.user_id,
        }
    }

    pub fn is_host(&self) -> bool {
        matches!(self, Self::Host { .. })
    }

    /// Ships one record the local app already applied to its runtime. Host
    /// mode: enters the host log at the local app's order and broadcasts.
    /// Client mode: sends to the host, which orders and relays.
    pub fn publish(&self, record: SharedDocumentActionRecord) -> Result<(), String> {
        match self {
            Self::Host { host, .. } => {
                host.lock()
                    .expect("session host lock")
                    .apply_local_record(record);
                Ok(())
            }
            Self::Client { client, .. } => client
                .send_action(record)
                .map_err(|error| error.to_string()),
        }
    }

    /// Pulls the network into the caller's runtime: applies foreign records
    /// in host order, skips own (already applied at publish). Returns how
    /// many foreign records were applied. Presence/roster caches update as a
    /// side effect on the client side.
    pub fn sync(&mut self, runtime: &mut SharedDocumentRuntime) -> Result<usize, String> {
        match self {
            Self::Host {
                host,
                user_id,
                consumed,
                ..
            } => {
                let host = host.lock().expect("session host lock");
                let records = host.records();
                let mut applied = 0;
                while *consumed < records.len() {
                    let record = &records[*consumed];
                    *consumed += 1;
                    if record.user_id != *user_id {
                        runtime.apply_action_record(record.clone());
                        applied += 1;
                    }
                }
                Ok(applied)
            }
            Self::Client { client, rejoin } => {
                if client.session_ended() {
                    return Err(SessionClientError::SessionEnded.to_string());
                }
                if !client.is_connected() {
                    // Client-owned auto-rejoin: attempt when the backoff
                    // window has passed; a successful rejoin rebuilds the
                    // caller's runtime from the fresh snapshot (full-log
                    // replay converges on the following syncs).
                    if Instant::now() >= rejoin.next_attempt {
                        match SessionClient::connect(&rejoin.address, rejoin.user.clone()) {
                            Ok((fresh, snapshot)) => {
                                debug_log::info(
                                    "session",
                                    &format!(
                                        "rejoined {} after {} failed attempt(s)",
                                        rejoin.address, rejoin.failed_attempts
                                    ),
                                );
                                rejoin.failed_attempts = 0;
                                *runtime = SharedDocumentRuntime::new(snapshot);
                                *client = fresh;
                                rejoin.backoff = REJOIN_INITIAL_BACKOFF;
                                rejoin.rejoined = true;
                            }
                            Err(error) => {
                                rejoin.backoff = (rejoin.backoff * 2).min(REJOIN_MAX_BACKOFF);
                                rejoin.failed_attempts += 1;
                                debug_log::warn(
                                    "session",
                                    &format!(
                                        "rejoin to {} failed: {error}; retry in {:?}",
                                        rejoin.address, rejoin.backoff
                                    ),
                                );
                                if rejoin.failed_attempts == REJOIN_FIREWALL_HINT_AT {
                                    debug_log::warn(
                                        "session",
                                        &format!(
                                            "rejoins keep failing — on the HOST machine, allow \
                                             inbound TCP for {} (windows: approve the firewall \
                                             prompt for thaum painter, or add an inbound rule)",
                                            rejoin.address
                                        ),
                                    );
                                }
                            }
                        }
                        rejoin.next_attempt = Instant::now() + rejoin.backoff;
                    }
                    return Ok(0);
                }
                client.sync(runtime).map_err(|error| error.to_string())
            }
        }
    }

    /// Whether the last `sync` performed a rejoin: the caller's runtime was
    /// rebuilt from a fresh snapshot, so everything it already had predates
    /// the new link and must never be republished. One-shot; cleared on read.
    pub fn take_rejoined(&mut self) -> bool {
        match self {
            Self::Host { .. } => false,
            Self::Client { rejoin, .. } => std::mem::take(&mut rejoin.rejoined),
        }
    }

    /// Renames the local user. Host mode: roster truth updates here and a
    /// roster broadcast reaches every client. Client mode: a `Rename` goes
    /// to the host, whose roster broadcast brings the name back to all.
    pub fn set_display_name(&mut self, display_name: &str) -> Result<(), String> {
        match self {
            Self::Host { host, .. } => {
                host.lock()
                    .expect("session host lock")
                    .set_host_display_name(display_name);
                Ok(())
            }
            Self::Client { client, .. } => client
                .send_rename(display_name)
                .map_err(|error| error.to_string()),
        }
    }

    /// The host's user id — the roster's entry zero (see `roster`).
    pub fn host_user_id(&self) -> Option<String> {
        match self {
            Self::Host { host, .. } => host
                .lock()
                .expect("session host lock")
                .host_user_id()
                .map(str::to_string),
            Self::Client { .. } => {
                // Roster contract: entry zero is the host.
                self.roster().first().map(|user| user.user_id.clone())
            }
        }
    }

    /// The session roster, host first: entry zero is always the hosting
    /// app's user, followed by joined clients in join order. The panel
    /// crowns entry zero and marks the entry matching `user_id` as you.
    /// Host mode reads the host core; client mode reads its mirror.
    pub fn roster(&self) -> Vec<SessionUser> {
        match self {
            Self::Host { host, .. } => host.lock().expect("session host lock").roster(),
            Self::Client { client, .. } => client.roster().to_vec(),
        }
    }

    pub fn cursor(&self, user_id: &str) -> Option<[i32; 3]> {
        match self {
            Self::Host { .. } => None, // host presence UI rides the client side
            Self::Client { client, .. } => client.cursor(user_id),
        }
    }

    /// Connected flag (client side); the host is "connected" while its server
    /// accepts. Used by the frame loop to stop publishing after a loss.
    pub fn is_connected(&self) -> bool {
        match self {
            Self::Host { .. } => true,
            Self::Client { client, .. } => client.is_connected(),
        }
    }
}

/// Builds the `SessionUser` wire identity from the persisted session identity.
/// Display name and presence color are cosmetic (see
/// `domain/painter-session/identity/`); the user id is the permanent one.
pub fn session_user_from_identity(identity: &thaum_painter_domain::SessionIdentity) -> SessionUser {
    SessionUser {
        user_id: identity.user_id.clone(),
        display_name: identity.display_name.clone(),
        presence_color: identity.presence_color,
    }
}

/// The env-driven session boot shape, parsed once at entrypoint boot:
/// `THAUM_SESSION_HOST` (=port, default 4747) hosts;
/// `THAUM_SESSION_JOIN=host:port` joins. Both set means join wins with a
/// note — one process is either side, never both.
pub enum SessionNetBoot {
    None,
    Host(u16),
    Join(String),
}

pub fn session_net_boot_from_env() -> SessionNetBoot {
    let host = std::env::var("THAUM_SESSION_HOST").ok();
    let join = std::env::var("THAUM_SESSION_JOIN").ok();
    if let Some(address) = join {
        if host.is_some() {
            eprintln!("THAUM_SESSION_JOIN wins over THAUM_SESSION_HOST: one process, one side");
        }
        return SessionNetBoot::Join(address);
    }
    if let Some(host) = host {
        let port = if host.is_empty() || host == "1" {
            crate::session_host_tcp::DEFAULT_SESSION_HOST_PORT
        } else {
            host.parse()
                .unwrap_or(crate::session_host_tcp::DEFAULT_SESSION_HOST_PORT)
        };
        return SessionNetBoot::Host(port);
    }
    SessionNetBoot::None
}

/// Parses `host:port` (or bare `host` with the default port) into a connect
/// address for `SessionNet::join`.
pub fn join_address(raw: &str) -> String {
    if raw.contains(':') {
        raw.to_string()
    } else {
        format!(
            "{raw}:{}",
            crate::session_host_tcp::DEFAULT_SESSION_HOST_PORT
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;
    use thaum_painter_domain::brush::PaintedCell;
    use thaum_painter_domain::paint_color::PaintColor;
    use thaum_painter_domain::storage::{SharedCellPatch, SharedDocumentFile};
    use thaum_renderer_domain::{CellGraphic, CellPoint};

    fn user(id: &str) -> SessionUser {
        SessionUser {
            user_id: id.into(),
            display_name: id.into(),
            presence_color: [1, 2, 3],
        }
    }

    fn paint(color: (u8, u8, u8)) -> PaintedCell {
        PaintedCell {
            graphic: CellGraphic::Glyph('a'),
            color: PaintColor::FlatRgb(color.0, color.1, color.2),
            weight_index: 2,
        }
    }

    fn stroke(
        net: &SessionNet,
        runtime: &SharedDocumentRuntime,
        action_id: &str,
        x: i32,
        color: (u8, u8, u8),
    ) -> SharedDocumentActionRecord {
        SharedDocumentActionRecord::cell_patch_set(
            action_id.to_string(),
            runtime.document.document_id.clone(),
            "layer-1".to_string(),
            net.user_id().to_string(),
            "t".to_string(),
            vec![SharedCellPatch::new(
                CellPoint { x, y: 0, z: 0 },
                None,
                Some(&paint(color)),
            )],
            None,
        )
    }

    fn canvas_len(runtime: &SharedDocumentRuntime) -> usize {
        runtime
            .canvas_for_layer("layer-1", 0)
            .cloned()
            .unwrap_or_default()
            .len()
    }

    /// Wire hops are async; poll sync until the expected count lands.
    fn sync_for(
        net: &mut SessionNet,
        runtime: &mut SharedDocumentRuntime,
        expected: usize,
    ) -> usize {
        let mut applied = 0;
        for _ in 0..100 {
            applied += net.sync(runtime).unwrap();
            if applied >= expected {
                return applied;
            }
            thread::sleep(Duration::from_millis(10));
        }
        applied
    }

    #[test]
    fn host_and_client_converge_through_the_unified_seam() {
        let boot_document = SharedDocumentFile::single_layer("doc-1", "Doc", "layer-1", "Layer 1");
        let source_document = boot_document.clone();
        let mut host_net = SessionNet::host(
            Box::new(move || source_document.clone()),
            user("host-user"),
            0,
            Vec::new(),
        )
        .expect("host boots");
        // The test host binds port 0; read the bound port from the server.
        let port = match &host_net {
            SessionNet::Host { _server, .. } => _server.port,
            SessionNet::Client { .. } => unreachable!(),
        };

        let (mut client_net, snapshot) =
            SessionNet::join(&format!("127.0.0.1:{port}"), user("client-1")).expect("join");
        assert_eq!(snapshot.document_id, "doc-1");
        let mut host_runtime = SharedDocumentRuntime::new(boot_document);
        let mut client_runtime = SharedDocumentRuntime::new(snapshot);

        // Host paints; client syncs it in through the same seam call shape.
        let record = stroke(&host_net, &host_runtime, "h-1", 0, (200, 0, 0));
        host_runtime.apply_action_record(record.clone());
        host_net.publish(record).unwrap();
        assert_eq!(sync_for(&mut client_net, &mut client_runtime, 1), 1);
        assert_eq!(canvas_len(&client_runtime), 1);

        // Client paints; the host's sync applies the foreign record.
        let record = stroke(&client_net, &client_runtime, "c-1", 1, (0, 0, 200));
        client_runtime.apply_action_record(record.clone());
        client_net.publish(record).unwrap();
        assert_eq!(sync_for(&mut host_net, &mut host_runtime, 1), 1);
        assert_eq!(canvas_len(&host_runtime), 2);

        // Idempotent syncs: own records are never re-applied.
        assert_eq!(host_net.sync(&mut host_runtime).unwrap(), 0);
        assert_eq!(client_net.sync(&mut client_runtime).unwrap(), 0);
    }

    /// The blank-cells regression: the Welcome snapshot is structure-only
    /// (canvases are replay-built runtime state), so a joiner only sees
    /// pre-host content if the host log was seeded with the pre-host action
    /// history. Without the seed the joiner gets layers with blank cells.
    #[test]
    fn joiner_replays_the_seeded_prehost_log_into_visible_cells() {
        let boot_document = SharedDocumentFile::single_layer("doc-1", "Doc", "layer-1", "Layer 1");
        let source_document = boot_document.clone();
        // One pre-host stroke — from a foreign identity, as persisted logs
        // legitimately carry (ids are per-machine). The seed must reach
        // joiners as replay truth while the host's own sync cursor skips it.
        let seed_record = SharedDocumentActionRecord::cell_patch_set(
            "pre-1".to_string(),
            "doc-1".to_string(),
            "layer-1".to_string(),
            "earlier-machine".to_string(),
            "t".to_string(),
            vec![SharedCellPatch::new(
                CellPoint { x: 0, y: 0, z: 0 },
                None,
                Some(&paint((200, 0, 0))),
            )],
            None,
        );
        let mut host_net = SessionNet::host(
            Box::new(move || source_document.clone()),
            user("host-user"),
            0,
            vec![seed_record],
        )
        .expect("host boots");
        let port = match &host_net {
            SessionNet::Host { _server, .. } => _server.port,
            SessionNet::Client { .. } => unreachable!(),
        };

        let (mut client_net, snapshot) =
            SessionNet::join(&format!("127.0.0.1:{port}"), user("client-1")).expect("join");
        let mut client_runtime = SharedDocumentRuntime::new(snapshot);
        assert_eq!(canvas_len(&client_runtime), 0); // snapshot is structure-only
        assert_eq!(sync_for(&mut client_net, &mut client_runtime, 1), 1);
        assert_eq!(canvas_len(&client_runtime), 1); // seed replay rebuilt the cell

        // The host's cursor starts past the seed: it must not re-apply the
        // pre-host history it already holds in its own runtime.
        let mut host_runtime = SharedDocumentRuntime::new(boot_document);
        assert_eq!(host_net.sync(&mut host_runtime).unwrap(), 0);
        assert_eq!(canvas_len(&host_runtime), 0);
    }

    #[test]
    fn host_reseed_rebuilds_clients_on_the_new_document() {
        // Wholesale document swap while hosting (file:new / file:open): the
        // re-seed installs the new snapshot, replaces the log, and evicts the
        // old client — whose auto-rejoin must land on the NEW document with
        // the NEW seed replay, and whose user_id must be free immediately (no
        // user-id-in-use squat from the stale-prune window).
        let doc_a = SharedDocumentFile::single_layer("doc-a", "Doc A", "layer-1", "Layer 1");
        let doc_b = SharedDocumentFile::single_layer("doc-b", "Doc B", "layer-1", "Layer 1");
        let doc_b_title = doc_b.title.clone();
        let seed_a = SharedDocumentActionRecord::cell_patch_set(
            "a-1".to_string(),
            "doc-a".to_string(),
            "layer-1".to_string(),
            "host-user".to_string(),
            "t".to_string(),
            vec![SharedCellPatch::new(
                CellPoint { x: 0, y: 0, z: 0 },
                None,
                Some(&paint((200, 0, 0))),
            )],
            None,
        );
        let seed_b = SharedDocumentActionRecord::cell_patch_set(
            "b-1".to_string(),
            "doc-b".to_string(),
            "layer-1".to_string(),
            "host-user".to_string(),
            "t".to_string(),
            vec![SharedCellPatch::new(
                CellPoint { x: 1, y: 1, z: 0 },
                None,
                Some(&paint((0, 0, 200))),
            )],
            None,
        );
        let seed_b_for_closure = seed_b.clone();
        let mut host_net = SessionNet::host(
            Box::new(move || doc_a.clone()),
            user("host-user"),
            0,
            vec![seed_a],
        )
        .expect("host boots");
        let port = match &host_net {
            SessionNet::Host { _server, .. } => _server.port,
            SessionNet::Client { .. } => unreachable!(),
        };
        let (mut client_net, snapshot) =
            SessionNet::join(&format!("127.0.0.1:{port}"), user("client-1")).expect("join");
        assert_eq!(snapshot.title, "Doc A");
        let mut client_runtime = SharedDocumentRuntime::new(snapshot);
        let mut host_runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-a", "Doc A", "layer-1", "Layer 1",
        ));

        // Re-seed onto Doc B. The cursor must cover the new seed.
        let cursor = host_net
            .reseed_host(Box::new(move || doc_b.clone()), vec![seed_b_for_closure])
            .expect("host re-seeds");
        assert_eq!(cursor, 1);

        // The evicted client's next publish hits NotJoined on the host, the
        // connection dies, and the auto-rejoin rebuilds from the new Welcome.
        // The publish may or may not error before the wire notices the
        // eviction; either way the host rejects it as NotJoined and the
        // connection dies, sending the client into the auto-rejoin path.
        let _ = client_net.publish(stroke(&client_net, &client_runtime, "c-1", 3, (0, 200, 0)));
        let mut rejoined = false;
        for _ in 0..400 {
            let _ = client_net.sync(&mut client_runtime);
            if client_net.take_rejoined() {
                rejoined = true;
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(rejoined, "client did not rejoin after the re-seed");
        assert_eq!(client_runtime.document.title, doc_b_title);
        // Seed replay streams right behind the Welcome; poll until it lands.
        let mut replayed = false;
        for _ in 0..300 {
            let _ = client_net.sync(&mut client_runtime);
            if canvas_len(&client_runtime) == 1 {
                replayed = true;
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(replayed, "seed replay did not reach the rejoined client");

        // The host-side consume cursor reset: a client record published after
        // the re-seed reaches the host runtime in host order. (The host skips
        // its own seed record — already applied locally — so the baseline is
        // the new document with no cells until a client record arrives.)
        let record = stroke(&client_net, &client_runtime, "c-2", 4, (0, 200, 0));
        client_runtime.apply_action_record(record.clone());
        client_net.publish(record).unwrap();
        let mut converged = false;
        for _ in 0..300 {
            let _ = host_net.sync(&mut host_runtime);
            if canvas_len(&host_runtime) >= 1 {
                converged = true;
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(converged, "host did not apply post-reseed client records");

        // A brand-new joiner gets the new document too.
        let (mut late_net, late_snapshot) =
            SessionNet::join(&format!("127.0.0.1:{port}"), user("client-2")).expect("late join");
        assert_eq!(late_snapshot.title, "Doc B");
        let _ = late_net;
    }

    #[test]
    fn client_rejoins_after_a_drop_and_converges_again() {
        let boot_document = SharedDocumentFile::single_layer("doc-1", "Doc", "layer-1", "Layer 1");
        let source_document = boot_document.clone();
        let host_net = SessionNet::host(
            Box::new(move || source_document.clone()),
            user("host-user"),
            0,
            Vec::new(),
        )
        .expect("host boots");
        let port = match &host_net {
            SessionNet::Host { _server, .. } => _server.port,
            SessionNet::Client { .. } => unreachable!(),
        };
        let (mut client_net, snapshot) =
            SessionNet::join(&format!("127.0.0.1:{port}"), user("client-1")).expect("join");
        let mut host_runtime = SharedDocumentRuntime::new(boot_document);
        let mut client_runtime = SharedDocumentRuntime::new(snapshot);

        // One stroke while connected, on both sides.
        let record = stroke(&host_net, &host_runtime, "h-1", 0, (200, 0, 0));
        host_runtime.apply_action_record(record.clone());
        host_net.publish(record).unwrap();
        assert_eq!(sync_for(&mut client_net, &mut client_runtime, 1), 1);

        // Simulate a drop: kill the client socket; the reader thread notices.
        if let SessionNet::Client { client, .. } = &client_net {
            client.force_disconnect();
        }
        for _ in 0..100 {
            if client_net.is_reconnecting() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(client_net.is_reconnecting());
        assert_eq!(client_net.rejoin_backoff_ms(), Some(500));

        // The host paints again while the client is down.
        let record = stroke(&host_net, &host_runtime, "h-2", 1, (0, 0, 200));
        host_runtime.apply_action_record(record.clone());
        host_net.publish(record).unwrap();

        // Poll sync: the rejoin fires, the runtime rebuilds from the fresh
        // snapshot, and full-log replay converges both strokes back in.
        let mut converged = false;
        for _ in 0..300 {
            let _ = client_net.sync(&mut client_runtime);
            if canvas_len(&client_runtime) == 2 {
                converged = true;
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(converged, "client did not rejoin and converge");
        assert!(!client_net.is_reconnecting());
        assert_eq!(client_net.rejoin_backoff_ms(), None);
    }

    #[test]
    fn host_ended_sessions_reach_the_client_and_stop_rejoin() {
        let boot_document = SharedDocumentFile::single_layer("doc-1", "Doc", "layer-1", "Layer 1");
        let source_document = boot_document.clone();
        let mut host_net = SessionNet::host(
            Box::new(move || source_document.clone()),
            user("host-user"),
            0,
            Vec::new(),
        )
        .expect("host boots");
        let port = match &host_net {
            SessionNet::Host { _server, .. } => _server.port,
            SessionNet::Client { .. } => unreachable!(),
        };
        let (mut client_net, snapshot) =
            SessionNet::join(&format!("127.0.0.1:{port}"), user("client-1")).expect("join");
        let mut client_runtime = SharedDocumentRuntime::new(snapshot);

        assert!(!client_net.session_ended());
        host_net.end_session();
        assert!(host_net.session_ended());

        // The client learns `Ended` on the next sync and never rejoins.
        let mut ended = false;
        for _ in 0..100 {
            match client_net.sync(&mut client_runtime) {
                Err(error) if error.to_string().contains("host ended the session") => {
                    ended = true;
                    break;
                }
                _ => thread::sleep(Duration::from_millis(10)),
            }
        }
        assert!(ended, "client never learned the session ended");
        assert!(client_net.session_ended());
        assert!(!client_net.is_reconnecting());
        assert_eq!(client_net.rejoin_backoff_ms(), None);
    }

    #[test]
    fn invite_addresses_list_reachable_interfaces_with_loopback_last() {
        let source_document =
            SharedDocumentFile::single_layer("doc-1", "Doc", "layer-1", "Layer 1");
        let host_net = SessionNet::host(
            Box::new(move || source_document.clone()),
            user("host-user"),
            0,
            Vec::new(),
        )
        .expect("host boots");
        let port = match &host_net {
            SessionNet::Host { _server, .. } => _server.port,
            SessionNet::Client { .. } => unreachable!(),
        };
        let addresses = host_net.invite_addresses();
        // Every address carries the bound port.
        assert!(addresses.iter().all(|a| a.ends_with(&format!(":{port}"))));
        // Loopback, when present, is last — the honest always-works reach.
        if let Some(position) = addresses.iter().position(|a| a.starts_with("127.0.0.1:")) {
            assert_eq!(position, addresses.len() - 1);
        }
    }
}

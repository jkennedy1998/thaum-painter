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
use crate::session_relay::relay_link::{spawn_relay_host_bridge, RelayClientBridge, RelayHostHandle};
use thaum_painter_domain::debug_log;
use thaum_painter_domain::storage::{
    SharedDocumentActionRecord, SharedDocumentFile, SharedDocumentRuntime,
};

pub enum SessionNet {
    Host {
        host: Arc<Mutex<SessionHost>>,
        user_id: String,
        /// How many host-log records the local app has consumed into its own
        /// runtime. Starts at the seed length: the pre-host history was
        /// loaded from disk into the local runtime already (that is where
        /// the seed came from), so host sync skips straight past it. Own
        /// records are skipped (applied at publish); foreign records (from
        /// clients) are applied here.
        consumed: usize,
        transport: HostTransport,
    },
    Client {
        client: SessionClient,
        /// Client-owned auto-rejoin truth: where and who to rejoin as, plus
        /// the current capped backoff. Never used after the host says `Ended`.
        rejoin: ClientRejoin,
        /// Relay-lane joiners keep their loopback bridge alive here — it
        /// serves every auto-rejoin connect by re-dialing the relay. LAN
        /// joiners bridge nothing.
        bridge: Option<RelayClientBridge>,
    },
}

/// Host-side transport: a direct LAN listener, the relay bridge, or both
/// at once (one hosted session reachable over either lane — LAN joiners
/// dial the listener, internet joiners dial the relay with the code).
pub enum HostTransport {
    /// Direct LAN TCP server; dropping it ends the accept loop.
    Lan(SessionHostServer),
    /// Relay lane: the handle carries the minted invite code; dropping it
    /// tears the bridge down (the relay room closes, joiners auto-rejoin).
    Relay(RelayHostHandle),
    /// Both lanes on one hosted session: the relay invite code AND a LAN
    /// listener. The net-level answer to mixed LAN + internet parties.
    Both {
        server: SessionHostServer,
        handle: RelayHostHandle,
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
    /// one-shot hint in the run log: on LAN the dominant cause of a client
    /// that never stops reconnecting is the host machine's firewall silently
    /// dropping inbound TCP; over the relay lane the link failure means the
    /// relay itself is unreachable (relay down, code dead, or no internet).
    failed_attempts: u32,
    /// One-shot flag: the last `sync` performed a rejoin, so the caller's
    /// runtime was rebuilt from a fresh snapshot and must not republish what
    /// it already had. Cleared on read via `SessionNet::take_rejoined`.
    rejoined: bool,
    /// True when the rejoin address is a relay loopback bridge, not a LAN
    /// host — picks the right one-shot hint at the failure threshold.
    via_relay: bool,
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
        Ok(Self::Host {
            host,
            user_id: user.user_id,
            consumed,
            transport: HostTransport::Lan(server),
        })
    }

    /// Starts hosting over the relay lane with a LAN listener alongside:
    /// the minted `<room6>-<token10>` invite code covers internet joiners
    /// and the bound LAN port covers same-network joiners (via discovery or
    /// direct dial) — one hosted session, both lanes. If the relay dial
    /// fails the caller falls back to `host` (LAN-only) and reports why.
    pub fn host_relay(
        relay_address: &str,
        snapshot_source: Box<dyn Fn() -> SharedDocumentFile + Send>,
        user: SessionUser,
        seed_records: Vec<SharedDocumentActionRecord>,
        lan_port: u16,
    ) -> std::io::Result<Self> {
        let mut host = SessionHost::new();
        host.set_snapshot_source(snapshot_source);
        host.set_host_user(user.clone());
        let consumed = seed_records.len();
        host.seed_log(seed_records);
        let host = Arc::new(Mutex::new(host));
        let handle = spawn_relay_host_bridge(Arc::clone(&host), relay_address)?;
        // The LAN listener is best-effort: losing it (port in use, firewall)
        // degrades to relay-only hosting instead of failing the whole start.
        let server = spawn_session_host_server(Arc::clone(&host), lan_port).ok();
        debug_log::info(
            "session",
            &format!(
                "hosting over relay {} as {} (invite {}); lan lane {}",
                relay_address,
                user.user_id,
                handle.code,
                match &server {
                    Some(_) => format!("listening on {lan_port}"),
                    None => "unavailable (relay-only)".to_string(),
                }
            ),
        );
        Ok(Self::Host {
            host,
            user_id: user.user_id,
            consumed,
            transport: match server {
                Some(server) => HostTransport::Both { server, handle },
                None => HostTransport::Relay(handle),
            },
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
                    via_relay: false,
                },
                bridge: None,
            },
            snapshot,
        ))
    }

    /// Joins a relay-hosted session by invite code (`<room6>-<token10>`).
    /// Spawns the loopback bridge, then runs the UNCHANGED LAN client against
    /// it — the returned snapshot and every later sync behave exactly like a
    /// LAN join. Auto-rejoin re-dials the relay through the same bridge.
    pub fn join_relay(
        relay_address: &str,
        code: &str,
        user: SessionUser,
    ) -> Result<(Self, SharedDocumentFile), SessionClientError> {
        let bridge = RelayClientBridge::start(relay_address, code, &user.user_id)
            .map_err(SessionClientError::Io)?;
        let (client, snapshot) = SessionClient::connect(bridge.local_address(), user.clone())?;
        Ok((
            Self::Client {
                client,
                rejoin: ClientRejoin {
                    address: bridge.local_address().to_string(),
                    user,
                    backoff: REJOIN_INITIAL_BACKOFF,
                    next_attempt: Instant::now(),
                    failed_attempts: 0,
                    rejoined: false,
                    via_relay: true,
                },
                bridge: Some(bridge),
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
            Self::Host { host, consumed, transport, .. } => {
                let mut host = host.lock().expect("session host lock");
                host.set_snapshot_source(snapshot_source);
                host.seed_log(seed_records);
                host.evict_clients();
                *consumed = 0;
                let records = host.records().len();
                drop(host);
                // Relay lane: the core eviction must also close the wire, or
                // the evicted joiner's relay id stays claimed and their rejoin
                // is denied user-id-in-use until the stale-prune timeout.
                if let HostTransport::Relay(handle)
                | HostTransport::Both { handle, .. } = transport
                {
                    handle.kick_all();
                }
                Ok(records)
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
            Self::Client { client, rejoin, .. } => {
                if client.is_connected() || client.session_ended() {
                    None
                } else {
                    Some(rejoin.backoff.as_millis() as u64)
                }
            }
        }
    }

    /// The bound LAN port (host mode, direct lane); relay hosts have none.
    /// Test and tooling exposure — the panel reads `invite_addresses()`.
    /// The LAN port this hosted session listens on, when the LAN lane is
    /// live (direct LAN hosting, or the both-lanes transport). Discovery
    /// advertising rides the same truth.
    pub fn lan_port(&self) -> Option<u16> {
        match self {
            Self::Host {
                transport: HostTransport::Lan(server),
                ..
            }
            | Self::Host {
                transport: HostTransport::Both { server, .. },
                ..
            } => Some(server.port),
            _ => None,
        }
    }

    /// The addresses a joiner should type to reach this hosted session. LAN
    /// lane: every reachable non-loopback IPv4 interface first, loopback
    /// last. Relay lane: the minted invite code. Both lanes: the code first
    /// (the low-friction invite), then the LAN addresses — the panel renders
    /// these and nothing else computes them. Client mode (and offline hosts)
    /// have no invite.
    pub fn invite_addresses(&self) -> Vec<String> {
        match self {
            Self::Host { transport, .. } => match transport {
                HostTransport::Relay(handle) => vec![handle.code.clone()],
                HostTransport::Both { server, handle } => {
                    // Both lanes: the code leads (the low-friction invite),
                    // the LAN addresses follow.
                    let mut invites = vec![handle.code.clone()];
                    invites.extend(lan_addresses(server.port));
                    invites
                }
                HostTransport::Lan(server) => lan_addresses(server.port),
            },
            Self::Client { .. } => Vec::new(),
        }
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
            Self::Client { client, rejoin, .. } => {
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
                                    if rejoin.via_relay {
                                        debug_log::warn(
                                            "session",
                                            &format!(
                                                "rejoins keep failing — the relay link cannot come \
                                                 back (relay down, invite code dead, or no \
                                                 internet on this machine)",
                                            ),
                                        );
                                    } else {
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

    /// Connected flag (client side); the host is "connected" while its LAN
    /// server accepts, its relay bridge holds the room, or both. Used by the
    /// frame loop to stop publishing after a loss.
    pub fn is_connected(&self) -> bool {
        match self {
            Self::Host { transport, .. } => match transport {
                HostTransport::Lan(_) | HostTransport::Both { .. } => true,
                HostTransport::Relay(handle) => handle.is_alive(),
            },
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

/// Every reachable LAN address for a served port: non-loopback IPv4
/// interfaces first, loopback last. The LAN-lane invite list.
fn lan_addresses(port: u16) -> Vec<String> {
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

/// The env-driven session boot shape, parsed once at entrypoint boot:
/// `THAUM_SESSION_HOST` (=port, default 4747) hosts over LAN;
/// `THAUM_SESSION_JOIN=host:port` joins over LAN;
/// `THAUM_SESSION_RELAY=relay:port` hosts over the relay lane (invite code
/// minted at boot), and `THAUM_SESSION_RELAY` + `THAUM_SESSION_CODE=<code>`
/// joins by invite code. `THAUM_SESSION_CODE` without a relay env dials the
/// built-in default relay. Relay envs win when mixed with LAN envs (one
/// process, one lane) with a note; LAN env behavior is unchanged.
pub enum SessionNetBoot {
    None,
    Host(u16),
    Join(String),
    RelayHost(String),
    RelayJoin { relay: String, code: String },
}

pub fn session_net_boot_from_env() -> SessionNetBoot {
    session_net_boot_from_parts(
        std::env::var("THAUM_SESSION_HOST").ok(),
        std::env::var("THAUM_SESSION_JOIN").ok(),
        std::env::var("THAUM_SESSION_RELAY").ok(),
        std::env::var("THAUM_SESSION_CODE").ok(),
    )
}

/// Pure boot-parse core (env-independent for tests): relay envs win over LAN
/// envs, join wins over host, everything absent boots to no session.
pub fn session_net_boot_from_parts(
    host: Option<String>,
    join: Option<String>,
    relay: Option<String>,
    code: Option<String>,
) -> SessionNetBoot {
    let relay = relay.filter(|r| !r.is_empty());
    let code = code.filter(|c| !c.is_empty());
    if let Some(relay) = relay {
        if host.is_some() || join.is_some() {
            eprintln!(
                "THAUM_SESSION_RELAY wins over THAUM_SESSION_HOST/THAUM_SESSION_JOIN: one \
                 process, one lane"
            );
        }
        return match code {
            Some(code) => SessionNetBoot::RelayJoin { relay, code },
            None => SessionNetBoot::RelayHost(relay),
        };
    }
    // A code with no relay env dials the built-in relay (consumer path).
    if let Some(code) = code {
        return SessionNetBoot::RelayJoin {
            relay: crate::session_relay::DEFAULT_RELAY_ADDRESS.to_string(),
            code,
        };
    }
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
        let port = host_net.lan_port().expect("lan host");

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
        let port = host_net.lan_port().expect("lan host");

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
        let port = host_net.lan_port().expect("lan host");
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
        let port = host_net.lan_port().expect("lan host");
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
        let port = host_net.lan_port().expect("lan host");
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

    /// Placeholder-free boot parse: relay envs win, join beats host, LAN
    /// behavior unchanged, empty strings mean unset.
    #[test]
    fn boot_parse_routes_relay_join_and_host_shapes() {
        use crate::session_net::session_net_boot_from_parts as parse;
        assert!(matches!(parse(None, None, None, None), SessionNetBoot::None));
        assert!(matches!(parse(Some("4747".into()), None, None, None), SessionNetBoot::Host(4747)));
        assert!(matches!(
            parse(None, Some("10.0.0.5:4747".into()), None, None),
            SessionNetBoot::Join(addr) if addr == "10.0.0.5:4747"
        ));
        assert!(matches!(
            parse(Some("1".into()), Some("x:1".into()), Some("r:1".into()), None),
            SessionNetBoot::RelayHost(relay) if relay == "r:1"
        ));
        assert!(matches!(
            parse(None, None, Some("r:1".into()), Some("AAAAAA-BBBBBBBBBB".into())),
            SessionNetBoot::RelayJoin { relay, code }
                if relay == "r:1" && code == "AAAAAA-BBBBBBBBBB"
        ));
        // Empty strings mean unset, not a lane.
        assert!(matches!(parse(None, None, Some(String::new()), None), SessionNetBoot::None));
        assert!(matches!(
            parse(None, None, Some("r:1".into()), Some(String::new())),
            SessionNetBoot::RelayHost(_)
        ));
        // A code with no relay env dials the built-in default relay.
        assert!(matches!(
            parse(None, None, None, Some("AAAAAA-BBBBBBBBBB".into())),
            SessionNetBoot::RelayJoin { relay, code }
                if relay == crate::session_relay::DEFAULT_RELAY_ADDRESS
                    && code == "AAAAAA-BBBBBBBBBB"
        ));
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
        let port = host_net.lan_port().expect("lan host");
        let addresses = host_net.invite_addresses();
        // Every address carries the bound port.
        assert!(addresses.iter().all(|a| a.ends_with(&format!(":{port}"))));
        // Loopback, when present, is last — the honest always-works reach.
        if let Some(position) = addresses.iter().position(|a| a.starts_with("127.0.0.1:")) {
            assert_eq!(position, addresses.len() - 1);
        }
    }

    // -----------------------------------------------------------------------
    // Relay lane: same seam calls, different dial target (phase-3 tests)
    // -----------------------------------------------------------------------

    fn spawn_test_relay() -> (String, crate::session_relay::relay_server::RelayServer) {
        use std::net::TcpListener;
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind ephemeral");
        let address = format!("127.0.0.1:{}", listener.local_addr().unwrap().port());
        let server = crate::session_relay::relay_server::serve_relay(
            listener,
            crate::session_relay::relay_server::RelayCaps::default(),
        )
        .expect("serve relay");
        (address, server)
    }

    #[test]
    fn relay_lane_converges_host_and_joiner_through_the_unified_seam() {
        let (relay_address, _relay) = spawn_test_relay();
        let boot_document = SharedDocumentFile::single_layer("doc-1", "Doc", "layer-1", "Layer 1");
        let source_document = boot_document.clone();
        let mut host_net = SessionNet::host_relay(
            &relay_address,
            Box::new(move || source_document.clone()),
            user("host-user"),
            Vec::new(),
            0,
        )
        .expect("relay host boots");

        // Both lanes by default now: the code leads, LAN addresses follow.
        let invite = host_net.invite_addresses();
        assert!(crate::session_relay::parse_code(&invite[0]).is_some());
        assert!(invite.len() >= 2, "relay+lan invites, got {invite:?}");
        assert!(host_net.lan_port().is_some());

        let (mut client_net, snapshot) =
            SessionNet::join_relay(&relay_address, &invite[0], user("client-1")).expect("relay join");
        assert_eq!(snapshot.document_id, "doc-1");
        let mut host_runtime = SharedDocumentRuntime::new(boot_document);
        let mut client_runtime = SharedDocumentRuntime::new(snapshot);

        // Host paints; the joiner syncs it in through the same seam call.
        let record = stroke(&host_net, &host_runtime, "h-1", 0, (200, 0, 0));
        host_runtime.apply_action_record(record.clone());
        host_net.publish(record).unwrap();
        assert_eq!(sync_for(&mut client_net, &mut client_runtime, 1), 1);
        assert_eq!(canvas_len(&client_runtime), 1);

        // Joiner paints; the host's sync applies the foreign record.
        let record = stroke(&client_net, &client_runtime, "c-1", 1, (0, 0, 200));
        client_runtime.apply_action_record(record.clone());
        client_net.publish(record).unwrap();
        assert_eq!(sync_for(&mut host_net, &mut host_runtime, 1), 1);
        assert_eq!(canvas_len(&host_runtime), 2);

        // Idempotent syncs on the relay lane too.
        assert_eq!(host_net.sync(&mut host_runtime).unwrap(), 0);
        assert_eq!(client_net.sync(&mut client_runtime).unwrap(), 0);

        // Presence rides the relay wire like LAN.
        client_net.set_display_name("Painter One").unwrap();
        let mut renamed = false;
        for _ in 0..100 {
            let _ = host_net.sync(&mut host_runtime);
            if host_net
                .roster()
                .iter()
                .any(|u| u.user_id == "client-1" && u.display_name == "Painter One")
            {
                renamed = true;
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(renamed, "rename never reached the relay host");
    }

    #[test]
    fn relay_joiner_rejoins_after_a_drop_and_converges_again() {
        let (relay_address, _relay) = spawn_test_relay();
        let boot_document = SharedDocumentFile::single_layer("doc-1", "Doc", "layer-1", "Layer 1");
        let source_document = boot_document.clone();
        let host_net = SessionNet::host_relay(
            &relay_address,
            Box::new(move || source_document.clone()),
            user("host-user"),
            Vec::new(),
            0,
        )
        .expect("relay host boots");
        let invite = host_net.invite_addresses().remove(0);
        let (mut client_net, snapshot) =
            SessionNet::join_relay(&relay_address, &invite, user("client-1")).expect("relay join");
        let mut host_runtime = SharedDocumentRuntime::new(boot_document);
        let mut client_runtime = SharedDocumentRuntime::new(snapshot);

        let record = stroke(&host_net, &host_runtime, "h-1", 0, (200, 0, 0));
        host_runtime.apply_action_record(record.clone());
        host_net.publish(record).unwrap();
        assert_eq!(sync_for(&mut client_net, &mut client_runtime, 1), 1);

        // Simulate a drop: kill the client socket; the bridge tears the pair
        // down and the auto-rejoin path re-dials the relay through it.
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

        // The host paints again while the joiner is down.
        let record = stroke(&host_net, &host_runtime, "h-2", 1, (0, 0, 200));
        host_runtime.apply_action_record(record.clone());
        host_net.publish(record).unwrap();

        let mut converged = false;
        for _ in 0..300 {
            let _ = client_net.sync(&mut client_runtime);
            if canvas_len(&client_runtime) == 2 {
                converged = true;
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(converged, "relay joiner did not rejoin and converge");
        assert!(!client_net.is_reconnecting());
    }

    #[test]
    fn relay_host_reseed_rebuilds_joiners_on_the_new_document() {
        let (relay_address, _relay) = spawn_test_relay();
        let doc_a = SharedDocumentFile::single_layer("doc-a", "Doc A", "layer-1", "Layer 1");
        let doc_b = SharedDocumentFile::single_layer("doc-b", "Doc B", "layer-1", "Layer 1");
        let doc_b_title = doc_b.title.clone();
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
        let doc_a_for_closure = doc_a.clone();
        let mut host_net = SessionNet::host_relay(
            &relay_address,
            Box::new(move || doc_a_for_closure.clone()),
            user("host-user"),
            Vec::new(),
            0,
        )
        .expect("relay host boots");
        let invite = host_net.invite_addresses().remove(0);
        let (mut client_net, snapshot) =
            SessionNet::join_relay(&relay_address, &invite, user("client-1")).expect("relay join");
        assert_eq!(snapshot.title, "Doc A");
        let mut client_runtime = SharedDocumentRuntime::new(snapshot);
        let mut host_runtime = SharedDocumentRuntime::new(doc_a);

        let cursor = host_net
            .reseed_host(
                Box::new(move || {
                    SharedDocumentFile::single_layer("doc-b", "Doc B", "layer-1", "Layer 1")
                }),
                vec![seed_b],
            )
            .expect("host re-seeds");
        assert_eq!(cursor, 1);

        // The evicted joiner's next publish dies on the host; auto-rejoin
        // rebuilds from the new Welcome + replay, over the same bridge.
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
        assert!(rejoined, "relay joiner did not rejoin after the re-seed");
        assert_eq!(client_runtime.document.title, doc_b_title);

        let mut replayed = false;
        for _ in 0..300 {
            let _ = client_net.sync(&mut client_runtime);
            if canvas_len(&client_runtime) == 1 {
                replayed = true;
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(replayed, "relay seed replay did not reach the rejoined joiner");

        // Post-reseed joiner records still reach the host runtime in order.
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
        assert!(converged, "relay host did not apply post-reseed joiner records");
    }

    #[test]
    fn dropping_the_relay_host_leaves_joiners_reconnecting_not_ended() {
        let (relay_address, _relay) = spawn_test_relay();
        let boot_document = SharedDocumentFile::single_layer("doc-1", "Doc", "layer-1", "Layer 1");
        let source_document = boot_document.clone();
        let host_net = SessionNet::host_relay(
            &relay_address,
            Box::new(move || source_document.clone()),
            user("host-user"),
            Vec::new(),
            0,
        )
        .expect("relay host boots");
        let invite = host_net.invite_addresses().remove(0);
        let (mut client_net, snapshot) =
            SessionNet::join_relay(&relay_address, &invite, user("client-1")).expect("relay join");
        let mut client_runtime = SharedDocumentRuntime::new(snapshot);
        assert_eq!(sync_for(&mut client_net, &mut client_runtime, 0), 0);

        // The host net drops: the relay room dies (room-closed to the joiner).
        drop(host_net);

        for _ in 0..100 {
            if client_net.is_reconnecting() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(client_net.is_reconnecting(), "joiner never saw the room close");
        assert!(!client_net.session_ended(), "a relay drop is not a purposeful end");
    }

}

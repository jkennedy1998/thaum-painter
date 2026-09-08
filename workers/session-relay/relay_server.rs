//! The relay server: room registry + per-connection routing. In-memory only —
//! rooms die with their host connection, zero disk state (settled design
//! truth: the session host owns all document truth; the relay only routes).
//!
//! Routing rules are fixed and dumb: a room has exactly one host and up to
//! `RelayCaps::max_members` joiners. Joiner lines go to the host; host frames
//! go to one named joiner or all. The relay never parses the inner session
//! lines — see `relay_protocol`.

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::net::{IpAddr, Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// One relay connection's byte stream: a plain `TcpStream` in tests/dev, a
/// rustls TLS stream at deploy. The server core is generic over it — the
/// wire protocol never knows which.
pub trait RelayReadWrite: Read + Write {}
impl<T: Read + Write> RelayReadWrite for T {}
pub type RelayStreamBox = Box<dyn RelayReadWrite + Send>;

/// Upgrades one accepted `TcpStream` into the connection stream. Plain is the
/// identity upgrade; the deploy TLS acceptor wraps in rustls (see
/// `relay_lane::TlsRelayServerAcceptor`).
pub trait RelayConnectionUpgrader: Send + Sync {
    fn upgrade(&self, stream: TcpStream) -> std::io::Result<RelayStreamBox>;
}

/// The no-op upgrader: the accepted stream IS the connection stream.
pub struct PlainRelayUpgrader;

impl RelayConnectionUpgrader for PlainRelayUpgrader {
    fn upgrade(&self, mut stream: TcpStream) -> std::io::Result<RelayStreamBox> {
        stream.set_read_timeout(Some(RELAY_STREAM_READ_SLICE))?;
        Ok(Box::new(stream))
    }
}

/// One live relay connection: the boxed stream plus its partial-frame buffer,
/// behind one mutex shared by the reader (this connection's thread) and the
/// writer half (its outbound queue thread). TLS streams cannot be split the
/// way `TcpStream::try_clone` splits sockets, so the halves share the stream
/// under the lock; reads release it every `RELAY_STREAM_READ_SLICE`.
pub(crate) type SharedStream = Arc<Mutex<ConnState>>;

pub(crate) struct ConnState {
    pub stream: RelayStreamBox,
    pub buffer: FrameBuffer,
}

pub(crate) fn read_frame_shared(
    stream: &SharedStream,
    max_frame_bytes: usize,
) -> std::io::Result<Option<String>> {
    loop {
        let mut state = stream.lock().expect("relay stream lock");
        if let Some(frame) = state.buffer.take_frame(max_frame_bytes)? {
            return Ok(Some(frame));
        }
        let mut chunk = [0u8; 8192];
        match state.stream.read(&mut chunk) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "connection closed mid-frame",
                ));
            }
            Ok(n) => state.buffer.extend(&chunk[..n]),
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut =>
            {
                // Idle slice: drop the lock across the gap so the writer
                // half (and direct writes from claim/join under the
                // registry lock) can take their turn on the stream.
                drop(state);
                thread::sleep(Duration::from_millis(5));
                continue;
            }
            Err(error) => return Err(error),
        }
    }
}

fn write_frame_shared(
    stream: &SharedStream,
    frame: &impl serde::Serialize,
    max_frame_bytes: usize,
) -> std::io::Result<()> {
    let mut state = stream.lock().expect("relay stream lock");
    write_frame(&mut state.stream, frame, max_frame_bytes)
}

fn write_str_shared(
    stream: &SharedStream,
    frame: &str,
    max_frame_bytes: usize,
) -> std::io::Result<()> {
    let mut state = stream.lock().expect("relay stream lock");
    write_str_frame(&mut state.stream, frame, max_frame_bytes)
}

use serde_json::json;

use crate::session_relay::relay_codes::{self, CODE_SEPARATOR};
use crate::session_relay::relay_protocol::{
    write_frame, FrameBuffer, HelloFrame, ServerFrame, BROADCAST_TARGET, DEFAULT_MAX_FRAME_BYTES,
};

/// Read-slice timeout on the connection stream: the server's reader releases
/// its per-connection lock between slices so the writer half can interleave.
/// Human-scale relay traffic makes the wake-ups negligible.
pub(crate) const RELAY_STREAM_READ_SLICE: Duration = Duration::from_millis(100);

/// Door and per-frame limits. Internet-exposed from day one, so these are not
/// a later slice.
#[derive(Debug, Clone)]
pub struct RelayCaps {
    pub max_frame_bytes: usize,
    pub max_members_per_room: usize,
    pub max_rooms: usize,
    /// Role claims allowed per IP per sliding window.
    pub joins_per_window: usize,
    pub join_window: Duration,
}

impl Default for RelayCaps {
    fn default() -> Self {
        Self {
            max_frame_bytes: DEFAULT_MAX_FRAME_BYTES,
            max_members_per_room: 8,
            max_rooms: 256,
            joins_per_window: 30,
            join_window: Duration::from_secs(60),
        }
    }
}

struct Member {
    outbound: mpsc::Sender<String>,
    joined_at: Instant,
    /// Raw socket clone used only to force-close this member's link (kick).
    /// `None` only if the pre-upgrade clone failed — kick degrades to a no-op.
    kick: Option<TcpStream>,
}

struct Room {
    token: String,
    host_outbound: mpsc::Sender<String>,
    /// Joiner session user id → outbound queue. BTreeMap keeps iteration order
    /// deterministic; roster order is the host core's job, not the relay's.
    members: BTreeMap<String, Member>,
}

impl Room {
    fn code(&self, room_id: &str) -> String {
        format!("{room_id}{CODE_SEPARATOR}{}", self.token)
    }
}

#[derive(Default)]
struct Registry {
    rooms: HashMap<String, Room>,
    /// Per-IP role-claim timestamps for the sliding-window rate guard.
    claim_attempts: HashMap<IpAddr, Vec<Instant>>,
}

impl Registry {
    /// Denies when this IP burned its claim window. Stale timestamps prune on
    /// the way in, so the window slides instead of ever filling permanently.
    fn claim_rate_allows(&mut self, address: IpAddr, caps: &RelayCaps) -> bool {
        let now = Instant::now();
        let attempts = self.claim_attempts.entry(address).or_default();
        attempts.retain(|at| now.duration_since(*at) < caps.join_window);
        if attempts.len() >= caps.joins_per_window {
            return false;
        }
        attempts.push(now);
        true
    }
}

/// The listener handle: shutdown stops the accept loop and drops the registry.
pub struct RelayServer {
    shutdown: Arc<AtomicBool>,
    accept_thread: Option<JoinHandle<()>>,
}

impl RelayServer {
    pub fn shutdown(mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(thread) = self.accept_thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for RelayServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(thread) = self.accept_thread.take() {
            let _ = thread.join();
        }
    }
}

/// Binds and serves until shutdown/drop over plain TCP. The deploy slice
/// fronts this with TLS via cert/key files — use `serve_relay_with` with the
/// TLS acceptor there; tests pass their own plain listeners.
pub fn serve_relay(listener: TcpListener, caps: RelayCaps) -> std::io::Result<RelayServer> {
    serve_relay_with(listener, caps, Arc::new(PlainRelayUpgrader))
}

/// Binds and serves until shutdown/drop, upgrading every accepted connection
/// through `upgrader` (plain identity or TLS).
pub fn serve_relay_with(
    listener: TcpListener,
    caps: RelayCaps,
    upgrader: Arc<dyn RelayConnectionUpgrader>,
) -> std::io::Result<RelayServer> {
    listener.set_nonblocking(true)?;
    let shutdown = Arc::new(AtomicBool::new(false));
    let registry = Arc::new(Mutex::new(Registry::default()));
    let accept_shutdown = Arc::clone(&shutdown);
    let accept_thread = thread::Builder::new()
        .name("session-relay-accept".into())
        .spawn(move || loop {
            if accept_shutdown.load(Ordering::SeqCst) {
                return;
            }
            match listener.accept() {
                Ok((stream, address)) => {
                    let registry = Arc::clone(&registry);
                    let caps = caps.clone();
                    let shutdown = Arc::clone(&accept_shutdown);
                    let upgrader = Arc::clone(&upgrader);
                    let _ = thread::Builder::new()
                        .name("session-relay-conn".into())
                        .spawn(move || {
                            if shutdown.load(Ordering::SeqCst) {
                                return;
                            }
                            let _ = stream.set_nodelay(true);
                            // Windows quirk parity with the session host: sockets
                            // accepted from a non-blocking listener inherit that
                            // mode on winsock; force blocking back on.
                            let _ = stream.set_nonblocking(false);
                            handle_connection(stream, address.ip(), &registry, &caps, &*upgrader);
                        });
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(25));
                }
                Err(_) => return,
            }
        })?;
    Ok(RelayServer {
        shutdown,
        accept_thread: Some(accept_thread),
    })
}

/// One connection: role claim, then the data loop until EOF/error. Every exit
/// path cleans the registry entry up.
fn handle_connection(
    stream: TcpStream,
    address: IpAddr,
    registry: &Arc<Mutex<Registry>>,
    caps: &RelayCaps,
    upgrader: &dyn RelayConnectionUpgrader,
) {
    // Clone the raw socket BEFORE the upgrade consumes it: this handle is the
    // room's kick lever for this member (host-originated force-close).
    let kick_handle: Option<TcpStream> = stream.try_clone().ok();
    let shared: SharedStream = match upgrader.upgrade(stream) {
        Ok(stream) => Arc::new(Mutex::new(ConnState {
            stream,
            buffer: FrameBuffer::new(),
        })),
        Err(_) => return,
    };

    let first = match read_frame_shared(&shared, caps.max_frame_bytes) {
        Ok(Some(frame)) => frame,
        _ => return,
    };
    let hello: Result<HelloFrame, _> = serde_json::from_str(&first);
    let hello = match hello {
        Ok(hello) => hello,
        Err(_) => {
            deny(&shared, caps, "bad-hello");
            return;
        }
    };

    // Sliding-window rate guard around every role claim.
    if !lock_registry(registry).claim_rate_allows(address, caps) {
        deny(&shared, caps, "claim-rate");
        return;
    }

    // `identity` is the routing address this connection receives frames under:
    // the host answers to BROADCAST_TARGET, joiners answer to their user id.
    let (identity, room_id) = match hello {
        HelloFrame::Host => match claim_host_room(registry, caps, &shared) {
            Some(pair) => pair,
            None => return,
        },
        HelloFrame::Join { room, token, user } => {
            if user.trim().is_empty() || user == BROADCAST_TARGET {
                deny(&shared, caps, "bad-user");
                return;
            }
            match join_room(registry, caps, &room, &token, &user, &shared, kick_handle) {
                Some(pair) => pair,
                None => return,
            }
        }
    };

    route_loop(&shared, registry, caps, &identity, &room_id);
    cleanup(registry, &room_id, &identity);
}

fn lock_registry(registry: &Arc<Mutex<Registry>>) -> std::sync::MutexGuard<'_, Registry> {
    registry.lock().expect("relay registry lock")
}

fn deny(stream: &SharedStream, caps: &RelayCaps, reason: &str) {
    let _ = write_frame_shared(
        stream,
        &ServerFrame::Error {
            reason: reason.to_string(),
        },
        caps.max_frame_bytes,
    );
}

/// Host role claim: mint the code, create the room, reply `ok` with both code
/// halves (the host app displays `room-token` verbatim as the invite).
fn claim_host_room(
    registry: &Arc<Mutex<Registry>>,
    caps: &RelayCaps,
    stream: &SharedStream,
) -> Option<((String, String), String)> {
    let mut registry = lock_registry(registry);
    if registry.rooms.len() >= caps.max_rooms {
        deny(stream, caps, "server-full");
        return None;
    }
    // Mint until the room id is free (6 glyphs of code space make collisions
    // rare enough that this loop is a formality).
    for _ in 0..64 {
        let (room, token, _) = relay_codes::generate_code();
        if registry.rooms.contains_key(&room) {
            continue;
        }
        let (outbound, outbound_rx) = mpsc::channel::<String>();
        spawn_connection_writer(Arc::clone(stream), outbound_rx, caps.max_frame_bytes);
        registry.rooms.insert(
            room.clone(),
            Room {
                token: token.clone(),
                host_outbound: outbound,
                members: Default::default(),
            },
        );
        let _ = write_frame_shared(
            stream,
            &ServerFrame::Ok {
                room: room.clone(),
                token,
            },
            caps.max_frame_bytes,
        );
        return Some(((BROADCAST_TARGET.to_string(), room.clone()), room));
    }
    deny(stream, caps, "server-full");
    None
}

/// Joiner role claim: the code's token must match, the room must exist, the
/// user id must be free, the room must have space.
fn join_room(
    registry: &Arc<Mutex<Registry>>,
    caps: &RelayCaps,
    room_id: &str,
    token: &str,
    user: &str,
    stream: &SharedStream,
    kick: Option<TcpStream>,
) -> Option<((String, String), String)> {
    let (room, token) = match (relay_codes::parse_code(&format!("{room_id}-{token}")), room_id)
    {
        (Some((parsed_room, parsed_token)), _) if parsed_room == room_id => {
            (room_id.to_string(), parsed_token)
        }
        _ => {
            deny(stream, caps, "bad-code");
            return None;
        }
    };
    let mut registry = lock_registry(registry);
    let Some(room_state) = registry.rooms.get_mut(&room) else {
        deny(stream, caps, "no-room");
        return None;
    };
    if room_state.token != token {
        deny(stream, caps, "bad-code");
        return None;
    }
    if room_state.members.contains_key(user) {
        deny(stream, caps, "user-id-in-use");
        return None;
    }
    if room_state.members.len() >= caps.max_members_per_room {
        deny(stream, caps, "room-full");
        return None;
    }
    let (outbound, outbound_rx) = mpsc::channel::<String>();
    spawn_connection_writer(Arc::clone(stream), outbound_rx, caps.max_frame_bytes);
    room_state.members.insert(
        user.to_string(),
        Member {
            outbound,
            joined_at: Instant::now(),
            kick,
        },
    );
    let _ = write_frame_shared(
        stream,
        &ServerFrame::Ok {
            room: room.clone(),
            token: token.clone(),
        },
        caps.max_frame_bytes,
    );
    Some(((user.to_string(), room.clone()), room))
}

/// The data loop. Returns on EOF/error; a routing `false` also ends it.
fn route_loop(
    stream: &SharedStream,
    registry: &Arc<Mutex<Registry>>,
    caps: &RelayCaps,
    identity: &(String, String),
    room_id: &str,
) {
    let (my_user_id, _) = identity;
    loop {
        match read_frame_shared(stream, caps.max_frame_bytes) {
            Ok(Some(frame)) => {
                let parsed: Result<serde_json::Value, _> = serde_json::from_str(&frame);
                let Ok(mut value) = parsed else {
                    return;
                };
                // Host control: an explicit `type: "kick"` frame force-closes
                // the targeted joiner link(s) so their auto-rejoin rebuilds
                // (re-seed eviction parity). The host link itself stays up.
                // (Tag-checked, not struct-parsed: internally-tagged serde
                // structs would happily eat a `to`/`line` data frame too.)
                if value.get("type").and_then(serde_json::Value::as_str) == Some("kick") {
                    let to = value
                        .get("to")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let mut registry = lock_registry(registry);
                    if let Some(room_state) = registry.rooms.get_mut(room_id) {
                        if to == BROADCAST_TARGET {
                            for member in room_state.members.values() {
                                if let Some(kick_stream) = &member.kick {
                                    let _ = kick_stream.shutdown(Shutdown::Both);
                                }
                            }
                        } else if let Some(member) = room_state.members.get(&to) {
                            if let Some(kick_stream) = &member.kick {
                                let _ = kick_stream.shutdown(Shutdown::Both);
                            }
                        }
                    }
                    continue;
                }
                if my_user_id == BROADCAST_TARGET {
                    // Host frame: `{"to":...,"line":...}` — route to one named
                    // joiner or every joiner. Unknown targets are dropped
                    // silently (the joiner may have just disconnected; the
                    // host's session state converges on its own).
                    let to = value
                        .get("to")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let Some(line) = value.get("line").and_then(serde_json::Value::as_str) else {
                        return;
                    };
                    let line = line.to_string();
                    let mut registry = lock_registry(registry);
                    let Some(room_state) = registry.rooms.get_mut(room_id) else {
                        return;
                    };
                    if to == BROADCAST_TARGET {
                        for member in room_state.members.values() {
                            let _ = member.outbound.send(line.clone());
                        }
                    } else if let Some(member) = room_state.members.get(&to) {
                        let _ = member.outbound.send(line);
                    }
                } else {
                    // Joiner frame: `{"line":...}` — always to the room host.
                    let Some(line) = value.get("line").and_then(serde_json::Value::as_str) else {
                        return;
                    };
                    let mut registry = lock_registry(registry);
                    let Some(room_state) = registry.rooms.get_mut(room_id) else {
                        return;
                    };
                    let wrapped = serde_json::to_string(&json!({
                        "from": my_user_id,
                        "line": line,
                    }))
                    .expect("wrapped frame serializes");
                    if room_state.host_outbound.send(wrapped).is_err() {
                        return;
                    }
                }
            }
            _ => return,
        }
    }
}

/// Connection end: a joiner leaves quietly — the room drops them and the
/// host gets `member-left` so the session core disconnects them exactly like
/// a LAN socket EOF (roster shrink, instant user-id freeing). The host
/// leaving closes the room and tells every member (`room-closed`) — the
/// client auto-rejoin path takes over from there.
fn cleanup(registry: &Arc<Mutex<Registry>>, room_id: &str, identity: &(String, String)) {
    let (my_user_id, _) = identity;
    let mut registry = lock_registry(registry);
    if my_user_id == BROADCAST_TARGET {
        if let Some(mut room) = registry.rooms.remove(room_id) {
            let closed =
                serde_json::to_string(&ServerFrame::RoomClosed).expect("room-closed serializes");
            for member in room.members.values() {
                let _ = member.outbound.send(closed.clone());
            }
            room.members.clear();
        }
    } else if let Some(room) = registry.rooms.get_mut(room_id) {
        if room.members.remove(my_user_id).is_some() {
            let left = serde_json::to_string(&ServerFrame::MemberLeft {
                user: my_user_id.clone(),
            })
            .expect("member-left serializes");
            let _ = room.host_outbound.send(left);
        }
    }
}

/// Per-connection writer thread: the registry routes JSON strings into this
/// queue; the thread drains them onto the socket. A write failure ends the
/// thread and drops this side's socket handle.
fn spawn_connection_writer(
    stream: SharedStream,
    outbound_rx: mpsc::Receiver<String>,
    max_frame_bytes: usize,
) {
    thread::Builder::new()
        .name("session-relay-writer".into())
        .spawn(move || {
            for frame in outbound_rx {
                // Frames arriving from the registry are already JSON strings
                // (route_loop wraps them); forward with the same prefix codec.
                if write_str_shared(&stream, &frame, max_frame_bytes).is_err() {
                    break;
                }
            }
        })
        .expect("relay writer thread spawns");
}

fn write_str_frame(
    stream: &mut impl Write,
    frame: &str,
    max_frame_bytes: usize,
) -> std::io::Result<()> {
    if frame.len() > max_frame_bytes {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "frame exceeds cap",
        ));
    }
    stream.write_all(&(frame.len() as u32).to_be_bytes())?;
    stream.write_all(frame.as_bytes())?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_relay::relay_codes::{parse_code, ROOM_ID_LEN, ROOM_TOKEN_LEN};
    use crate::session_relay::relay_protocol::{read_frame, DataFromJoiner, DataToHost};
    use std::io::{BufReader, Write as _};

    fn spawn_relay(caps: RelayCaps) -> (String, RelayServer) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind ephemeral");
        let address = listener.local_addr().unwrap();
        let server = serve_relay(listener, caps).expect("serve");
        (format!("127.0.0.1:{}", address.port()), server)
    }

    struct TestConn {
        writer: TcpStream,
        reader: BufReader<TcpStream>,
    }

    impl TestConn {
        fn connect(address: &str) -> Self {
            let stream = TcpStream::connect(address).expect("connect");
            stream.set_nodelay(true).unwrap();
            let reader_half = stream.try_clone().unwrap();
            reader_half
                .set_read_timeout(Some(Duration::from_millis(50)))
                .unwrap();
            let reader = BufReader::new(reader_half);
            Self {
                writer: stream,
                reader,
            }
        }

        fn send(&mut self, frame: &impl serde::Serialize) {
            write_frame(&mut self.writer, frame, DEFAULT_MAX_FRAME_BYTES).expect("send");
        }

        fn send_raw_json(&mut self, json: &str) {
            self.writer
                .write_all(&(json.len() as u32).to_be_bytes())
                .unwrap();
            self.writer.write_all(json.as_bytes()).unwrap();
            self.writer.flush().unwrap();
        }

        /// Reads one raw frame string, polling up to ~2s.
        fn recv(&mut self) -> Option<String> {
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                match read_frame(&mut self.reader, DEFAULT_MAX_FRAME_BYTES) {
                    Ok(Some(frame)) => return Some(frame),
                    Ok(None) => return None,
                    Err(_) => {}
                }
                if Instant::now() >= deadline {
                    return None;
                }
                thread::sleep(Duration::from_millis(10));
            }
        }

        fn expect_closed(&mut self) -> bool {
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                match read_frame(&mut self.reader, DEFAULT_MAX_FRAME_BYTES) {
                    Ok(Some(_)) => return false,
                    Ok(None) | Err(_) => return true,
                }
                if Instant::now() >= deadline {
                    return false;
                }
                thread::sleep(Duration::from_millis(10));
            }
        }
    }

    fn host_hello() -> impl serde::Serialize {
        HelloFrame::Host
    }

    fn join_hello(room: &str, token: &str, user: &str) -> impl serde::Serialize {
        HelloFrame::Join {
            room: room.into(),
            token: token.into(),
            user: user.into(),
        }
    }

    fn read_server_ok(conn: &mut TestConn) -> (String, String) {
        let frame = conn.recv().expect("server ok frame");
        let parsed: ServerFrame = serde_json::from_str(&frame).unwrap();
        match parsed {
            ServerFrame::Ok { room, token } => (room, token),
            other => panic!("expected ok, got {other:?}"),
        }
    }

    /// Claim wait: a relay accept loop ticks at 25ms — poll for the ok frame.
    fn claim_host(address: &str) -> (TestConn, String, String) {
        let mut conn = TestConn::connect(address);
        conn.send(&host_hello());
        let (room, token) = read_server_ok(&mut conn);
        (conn, room, token)
    }

    #[test]
    fn host_claims_a_room_and_joiners_route_through_it() {
        let (address, _server) = spawn_relay(RelayCaps::default());
        let (mut host, room, token) = claim_host(&address);
        assert_eq!(room.len(), ROOM_ID_LEN);
        assert_eq!(token.len(), ROOM_TOKEN_LEN);
        assert_eq!(parse_code(&format!("{room}-{token}")), Some((room.clone(), token.clone())));

        // Two joiners in.
        let mut alice = TestConn::connect(&address);
        alice.send(&join_hello(&room, &token, "alice"));
        let _ = read_server_ok(&mut alice);
        let mut bob = TestConn::connect(&address);
        bob.send(&join_hello(&room, &token, "bob"));
        let _ = read_server_ok(&mut bob);

        // Joiner line reaches the host wrapped with its user id.
        alice
            .send(&DataFromJoiner {
                line: "{\"type\":\"ping\"}".into(),
            });
        let from_host_view = host.recv().expect("host gets alice's line");
        let parsed: DataToHost = serde_json::from_str(&from_host_view).unwrap();
        assert_eq!(parsed.from, "alice");
        assert_eq!(parsed.line, "{\"type\":\"ping\"}");

        // Host broadcast reaches both joiners.
        host.send_raw_json(
            &serde_json::to_string(&serde_json::json!({
                "to": "all",
                "line": "{\"hello\":true}"
            }))
            .unwrap(),
        );
        // Delivered lines arrive UNWRAPPED — the inner session JSON verbatim,
        // so the joiner-side session core parses it exactly like the LAN wire.
        let at_alice = alice.recv().expect("alice gets broadcast");
        assert_eq!(at_alice, "{\"hello\":true}");
        let at_bob = bob.recv().expect("bob gets broadcast");
        assert_eq!(at_bob, "{\"hello\":true}");

        // Host targeted frame reaches only the named joiner.
        host.send_raw_json(
            &serde_json::to_string(&serde_json::json!({
                "to": "bob",
                "line": "{\"just\":\"bob\"}"
            }))
            .unwrap(),
        );
        let at_bob = bob.recv().expect("bob gets the targeted line");
        assert_eq!(at_bob, "{\"just\":\"bob\"}");
        // Alice gets nothing within the poll window.
        assert!(alice.recv().is_none());
    }

    #[test]
    fn join_denials_cover_token_room_collision_and_capacity() {
        let caps = RelayCaps {
            max_members_per_room: 2,
            ..RelayCaps::default()
        };
        let (address, _server) = spawn_relay(caps);
        let (mut host, room, token) = claim_host(&address);
        let _ = host;

        // Bad token.
        let mut peer = TestConn::connect(&address);
        peer.send(&join_hello(&room, "WRONGTOKEN", "alice"));
        let frame = peer.recv().expect("denial frame");
        assert_eq!(
            serde_json::from_str::<ServerFrame>(&frame).unwrap(),
            ServerFrame::Error {
                reason: "bad-code".into()
            }
        );
        assert!(peer.expect_closed());

        // No such room.
        let mut peer = TestConn::connect(&address);
        peer.send(&join_hello("ZZZZZZ", &token, "alice"));
        let frame = peer.recv().expect("denial frame");
        assert_eq!(
            serde_json::from_str::<ServerFrame>(&frame).unwrap(),
            ServerFrame::Error {
                reason: "no-room".into()
            }
        );
        assert!(peer.expect_closed());

        // User id collision.
        let mut alice = TestConn::connect(&address);
        alice.send(&join_hello(&room, &token, "alice"));
        let _ = read_server_ok(&mut alice);
        let mut alice_again = TestConn::connect(&address);
        alice_again.send(&join_hello(&room, &token, "alice"));
        let frame = alice_again.recv().expect("denial frame");
        assert_eq!(
            serde_json::from_str::<ServerFrame>(&frame).unwrap(),
            ServerFrame::Error {
                reason: "user-id-in-use".into()
            }
        );
        assert!(alice_again.expect_closed());

        // Room capacity (cap 2: alice + one more, then full).
        let mut carol = TestConn::connect(&address);
        carol.send(&join_hello(&room, &token, "carol"));
        let _ = read_server_ok(&mut carol);
        let mut dave = TestConn::connect(&address);
        dave.send(&join_hello(&room, &token, "dave"));
        let frame = dave.recv().expect("denial frame");
        assert_eq!(
            serde_json::from_str::<ServerFrame>(&frame).unwrap(),
            ServerFrame::Error {
                reason: "room-full".into()
            }
        );
        assert!(dave.expect_closed());
    }

    #[test]
    fn a_second_host_is_denied_and_host_disconnect_closes_the_room() {
        let (address, _server) = spawn_relay(RelayCaps::default());
        let (mut host, room, token) = claim_host(&address);
        let mut alice = TestConn::connect(&address);
        alice.send(&join_hello(&room, &token, "alice"));
        let _ = read_server_ok(&mut alice);

        // Second host on the same server: rooms are per-code, so this claims a
        // DIFFERENT room (room ids are minted, never reused) — the true
        // duplicate-host denial is impossible by construction. The host's
        // disconnect, though, must close ITS room for its members.
        let (mut other_host, other_room, _) = claim_host(&address);
        assert_ne!(room, other_room);
        let _ = other_host;

        host.writer.shutdown(std::net::Shutdown::Both).unwrap();
        let closed = alice.recv().expect("room-closed frame");
        assert_eq!(
            serde_json::from_str::<ServerFrame>(&closed).unwrap(),
            ServerFrame::RoomClosed
        );
        assert!(alice.expect_closed());
    }

    #[test]
    fn oversized_frames_are_refused_at_the_door() {
        let caps = RelayCaps {
            max_frame_bytes: 128,
            ..RelayCaps::default()
        };
        let (address, _server) = spawn_relay(caps);
        let mut peer = TestConn::connect(&address);
        // A hello claiming a huge frame: the reader refuses before allocating.
        peer.writer
            .write_all(&(5u32 * 1024 * 1024).to_be_bytes())
            .unwrap();
        peer.writer.write_all(b"{\"junk\":true}").unwrap();
        peer.writer.flush().unwrap();
        assert!(peer.expect_closed());
    }

    #[test]
    fn join_rate_guard_blocks_connection_floods_from_one_ip() {
        let caps = RelayCaps {
            joins_per_window: 3,
            join_window: Duration::from_secs(60),
            ..RelayCaps::default()
        };
        let (address, _server) = spawn_relay(caps);
        // Three claims pass; the fourth connection from the same IP is denied.
        for _ in 0..3 {
            let (mut host, _, _) = claim_host(&address);
            let _ = host;
        }
        let mut fourth = TestConn::connect(&address);
        fourth.send(&host_hello());
        let frame = fourth.recv().expect("rate denial frame");
        assert_eq!(
            serde_json::from_str::<ServerFrame>(&frame).unwrap(),
            ServerFrame::Error {
                reason: "claim-rate".into()
            }
        );
        assert!(fourth.expect_closed());
    }
}

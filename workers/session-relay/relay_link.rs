//! Relay client lane: the bridges that make the relay wire look like the LAN
//! wire to the existing session cores.
//!
//! Host role: `spawn_relay_host_bridge` dials the relay, claims a room, and
//! drives the transport-agnostic `SessionHost` core exactly like
//! `session-host/tcp.rs` does — relay data frames in, `handle_client_message`,
//! drained host queues out as routed frames. The minted `<room6>-<token10>`
//! code IS the invite.
//!
//! Joiner role: `RelayClientBridge` listens on a loopback port, dials the
//! relay, joins the room under the caller's session user id, and pumps bytes
//! both ways — so the UNCHANGED `SessionClient` connects to `127.0.0.1` and
//! speaks the exact LAN NDJSON wire it always has. Auto-rejoin, keepalive,
//! snapshot rebuild, and reseed convergence all come from the existing client
//! path; this bridge is the only new joiner transport logic.
//!
//! Dialing goes through `relay_tls::connect`: plain TCP only ever touches
//! loopback; every other target dials TLS. One boxed stream is shared by the
//! reader and writer halves under one mutex — TLS streams cannot be split the
//! way `TcpStream::try_clone` splits sockets — and reads release the lock
//! every slice so the writer half always gets its turn (the same shape the
//! relay server core uses per connection).

use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::session_host::{ClientMessage, HostMessage, SessionHost};
use crate::session_host_tcp::{rejection_reason, CLIENT_SEEN_TIMEOUT};
use crate::session_relay::relay_codes;
use crate::session_relay::relay_protocol::{
    read_frame, write_frame, DataFromHost, DataFromJoiner, DataToHost, HelloFrame, KickFromHost,
    ServerFrame, BROADCAST_TARGET, DEFAULT_MAX_FRAME_BYTES,
};
use crate::session_relay::relay_server::{
    read_frame_shared, ConnState, SharedStream, RELAY_STREAM_READ_SLICE,
};
use crate::session_relay::relay_tls::{self, RelayClientLink, HANDSHAKE_TIMEOUT};
use thaum_painter_domain::debug_log;

/// How often the host pump drains its outgoing queues.
const HOST_PUMP_INTERVAL: Duration = Duration::from_millis(10);

/// Shared-stream helper: writes one frame under the connection lock.
fn write_frame_shared(
    shared: &SharedStream,
    frame: &impl serde::Serialize,
) -> std::io::Result<()> {
    let mut state = shared.lock().expect("relay stream lock");
    write_frame(&mut state.stream, frame, DEFAULT_MAX_FRAME_BYTES)
}

/// Handshake exchange on a freshly dialed link: bounded by the socket timeout
/// so a dead relay fails the claim/join instead of hanging the bridge. Ends
/// with the stream in steady-state slice reads.
fn handshake_exchange(
    link: &RelayClientLink,
    outgoing: &HelloFrame,
) -> std::io::Result<ServerFrame> {
    link.socket
        .set_read_timeout(Some(HANDSHAKE_TIMEOUT))
        .ok();
    write_frame_shared(&link.shared, outgoing)?;
    let frame = {
        let mut state = link.shared.lock().expect("relay stream lock");
        read_frame(&mut state.stream, DEFAULT_MAX_FRAME_BYTES)?
    };
    // Steady state: slice reads release the lock between reads so the writer
    // half can interleave for the life of the connection.
    link.socket
        .set_read_timeout(Some(RELAY_STREAM_READ_SLICE))
        .ok();
    match frame {
        Some(frame) => serde_json::from_str::<ServerFrame>(&frame).map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unexpected relay frame during handshake: {error}"),
            )
        }),
        None => Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "relay closed during handshake",
        )),
    }
}

// ---------------------------------------------------------------------------
// Host role: the relay bridge driving the SessionHost core
// ---------------------------------------------------------------------------

pub struct RelayHostHandle {
    /// The minted invite: `<room6>-<token10>`, rendered verbatim by the panel.
    pub code: String,
    alive: Arc<AtomicBool>,
    /// Shared with the route + pump threads: the one locked relay stream.
    shared: SharedStream,
    /// Raw socket clone used only to force-close the link on drop (unblocks
    /// the reader; the TLS/plain stream behind the lock dies with it).
    socket: Option<TcpStream>,
    route_thread: Option<JoinHandle<()>>,
    pump_thread: Option<JoinHandle<()>>,
}

impl RelayHostHandle {
    /// False once the relay link died — the hosted session is unreachable for
    /// NEW joiners and nothing routes anymore.
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    /// Force-closes every joiner's relay link (re-seed eviction parity: the
    /// LAN host closes the socket; the relay host kicks the wire so joiners'
    /// auto-rejoin rebuilds from the new session state).
    pub fn kick_all(&self) {
        let _ = write_frame_shared(
            &self.shared,
            &KickFromHost::Kick {
                to: BROADCAST_TARGET.to_string(),
            },
        );
    }
}

impl Drop for RelayHostHandle {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::SeqCst);
        if let Some(socket) = self.socket.take() {
            let _ = socket.shutdown(Shutdown::Both);
        }
        if let Some(thread) = self.route_thread.take() {
            let _ = thread.join();
        }
        if let Some(thread) = self.pump_thread.take() {
            let _ = thread.join();
        }
    }
}

/// Dials the relay, claims a host room, and spawns the route + pump threads
/// driving `host`. Returns the handle carrying the minted invite code.
pub fn spawn_relay_host_bridge(
    host: Arc<Mutex<SessionHost>>,
    relay_address: &str,
) -> std::io::Result<RelayHostHandle> {
    let link = relay_tls::connect(relay_address)?;
    let (room, token) = match handshake_exchange(&link, &HelloFrame::Host)? {
        ServerFrame::Ok { room, token } => (room, token),
        ServerFrame::Error { reason } => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                format!("relay denied the host claim: {reason}"),
            ));
        }
        other => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unexpected relay frame during host claim: {other:?}"),
            ));
        }
    };

    let alive = Arc::new(AtomicBool::new(true));
    // All relay reads AND writes share one lock: the route thread's reads
    // slice so the pump's drained queue frames and deny replies interleave.
    let shared = Arc::clone(&link.shared);
    let route_alive = Arc::clone(&alive);
    let pump_host = Arc::clone(&host);
    let pump_shared = Arc::clone(&shared);
    let pump_alive = Arc::clone(&alive);
    let route_thread = thread::Builder::new()
        .name("relay-host-route".into())
        .spawn(move || relay_host_route_loop(shared, host, route_alive))?;
    let pump_thread = thread::Builder::new()
        .name("relay-host-pump".into())
        .spawn(move || relay_host_pump_loop(pump_host, pump_shared, pump_alive))?;
    Ok(RelayHostHandle {
        code: format!("{room}-{token}"),
        alive,
        shared: Arc::clone(&link.shared),
        socket: Some(link.socket),
        route_thread: Some(route_thread),
        pump_thread: Some(pump_thread),
    })
}

fn relay_host_route_loop(
    shared: SharedStream,
    host: Arc<Mutex<SessionHost>>,
    alive: Arc<AtomicBool>,
) {
    loop {
        let frame = match read_frame_shared(&shared, DEFAULT_MAX_FRAME_BYTES) {
            Ok(Some(frame)) => frame,
            Ok(None) | Err(_) => break,
        };
        // Server notice first: a joiner's relay link died — disconnect them
        // from the host core exactly like a LAN socket EOF (roster shrink,
        // instant user-id freeing for the honest rejoin).
        if let Ok(ServerFrame::MemberLeft { user }) = serde_json::from_str::<ServerFrame>(&frame) {
            let mut host = host.lock().expect("session host lock");
            host.disconnect(&user);
            drain_and_route_relay(&mut host, &shared);
            continue;
        }
        let Ok(data) = serde_json::from_str::<DataToHost>(&frame) else {
            break; // non-data frame on the data lane: protocol violation
        };
        let Ok(message) = serde_json::from_str::<ClientMessage>(&data.line) else {
            break; // garbage session line: lose the connection loudly (tcp.rs parity)
        };
        // Relay-enforced identity: the hello's user id must match the routing
        // identity the joiner claimed at the relay, or host replies would
        // route to a different (or nonexistent) peer.
        if let ClientMessage::Hello { user, .. } = &message {
            if user.user_id != data.from {
                debug_log::warn(
                    "session",
                    &format!(
                        "relay client {} claimed a mismatched hello id {}",
                        data.from, user.user_id
                    ),
                );
                send_host_frame(&shared, &data.from, &HostMessage::Denied {
                    reason: "identity-mismatch".to_string(),
                });
                break;
            }
        }
        let denied_reason = {
            let mut host = host.lock().expect("session host lock");
            match host.handle_client_message(&data.from, message) {
                Ok(()) => {
                    drain_and_route_relay(&mut host, &shared);
                    None
                }
                Err(rejection) => Some(rejection_reason(rejection)),
            }
        };
        if let Some(reason) = denied_reason {
            debug_log::warn(
                "session",
                &format!("relay client {} denied: {reason}", data.from),
            );
            send_host_frame(&shared, &data.from, &HostMessage::Denied { reason });
        }
    }
    alive.store(false, Ordering::SeqCst);
    debug_log::warn("session", "relay host bridge lost the relay link");
}

fn relay_host_pump_loop(
    host: Arc<Mutex<SessionHost>>,
    shared: SharedStream,
    alive: Arc<AtomicBool>,
) {
    loop {
        if !alive.load(Ordering::SeqCst) {
            return;
        }
        {
            let mut host = host.lock().expect("session host lock");
            host.prune_stale_clients(CLIENT_SEEN_TIMEOUT);
            drain_and_route_relay(&mut host, &shared);
        }
        thread::sleep(HOST_PUMP_INTERVAL);
    }
}

fn send_host_frame(shared: &SharedStream, to: &str, message: &HostMessage) {
    if let Ok(line) = serde_json::to_string(message) {
        let _ = write_frame_shared(
            shared,
            &DataFromHost {
                to: to.to_string(),
                line,
            },
        );
    }
}

fn drain_and_route_relay(host: &mut SessionHost, shared: &SharedStream) {
    for user in host.roster() {
        for message in host.take_outgoing(&user.user_id) {
            send_host_frame(shared, &user.user_id, &message);
        }
    }
}

// ---------------------------------------------------------------------------
// Joiner role: the loopback bridge under the unchanged SessionClient
// ---------------------------------------------------------------------------

pub struct RelayClientBridge {
    local_address: String,
    stop: Arc<AtomicBool>,
    accept_thread: Option<JoinHandle<()>>,
}

impl RelayClientBridge {
    /// Starts the loopback bridge for one relay room. `code` is the
    /// `<room6>-<token10>` invite; `user_id` is the session identity the
    /// relay routes as (and the host core sees — they must match).
    pub fn start(relay_address: &str, code: &str, user_id: &str) -> std::io::Result<Self> {
        let (room, token) = relay_codes::parse_code(code).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "malformed invite code",
            )
        })?;
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let local_address = format!("127.0.0.1:{}", listener.local_addr()?.port());
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let accept_stop = Arc::clone(&stop);
        let relay_address = relay_address.to_string();
        let user_id = user_id.to_string();
        let accept_thread = thread::Builder::new()
            .name("relay-bridge-accept".into())
            .spawn(move || {
                loop {
                    if accept_stop.load(Ordering::SeqCst) {
                        return;
                    }
                    match listener.accept() {
                        Ok((local, _)) => {
                            let relay_address = relay_address.clone();
                            let room = room.clone();
                            let token = token.clone();
                            let user_id = user_id.clone();
                            let _ = thread::Builder::new()
                                .name("relay-bridge-conn".into())
                                .spawn(move || {
                                    serve_bridge_connection(
                                        &relay_address,
                                        &room,
                                        &token,
                                        &user_id,
                                        local,
                                    )
                                });
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(25));
                        }
                        Err(_) => return,
                    }
                }
            })?;
        Ok(Self {
            local_address,
            stop,
            accept_thread: Some(accept_thread),
        })
    }

    /// The loopback address the unchanged `SessionClient` connects to (and
    /// rejoins through — the bridge re-dials the relay per connect).
    pub fn local_address(&self) -> &str {
        &self.local_address
    }
}

impl Drop for RelayClientBridge {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.accept_thread.take() {
            let _ = thread.join();
        }
    }
}

/// One bridge pair: one `SessionClient` connection on `local`, one relay
/// connection joined under `user_id`. Ends when either side dies; the next
/// `SessionClient::connect` starts a fresh pair (that IS the rejoin path).
fn serve_bridge_connection(
    relay_address: &str,
    room: &str,
    token: &str,
    user_id: &str,
    local: TcpStream,
) {
    local.set_nodelay(true).ok();
    // Windows parity: sockets accepted from a non-blocking listener inherit
    // that mode; the bridge halves need blocking I/O.
    local.set_nonblocking(false).ok();
    let link = match relay_tls::connect(relay_address) {
        Ok(link) => link,
        Err(_) => return,
    };
    let join = HelloFrame::Join {
        room: room.to_string(),
        token: token.to_string(),
        user: user_id.to_string(),
    };
    match handshake_exchange(&link, &join) {
        Ok(ServerFrame::Ok { .. }) => {}
        Ok(ServerFrame::Error { reason }) => {
            debug_log::warn("session", &format!("relay join denied: {reason}"));
            return; // local socket drops: the SessionClient handshake fails
        }
        _ => return,
    }
    debug_log::info(
        "session",
        &format!("relay bridge up: {} -> {} room {room}", user_id, relay_address),
    );

    // Outbound: local NDJSON lines -> relay data frames. When it ends it kills
    // the relay socket so the inbound loop unblocks and tears the pair down.
    let mut local_reader = BufReader::new(match local.try_clone() {
        Ok(reader) => reader,
        Err(_) => return,
    });
    let outbound_shared = Arc::clone(&link.shared);
    let outbound_socket = match link.socket.try_clone() {
        Ok(socket) => socket,
        Err(_) => return,
    };
    let outbound = thread::Builder::new()
        .name("relay-bridge-out".into())
        .spawn(move || {
            loop {
                let mut line = String::new();
                match local_reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                if write_frame_shared(
                    &outbound_shared,
                    &DataFromJoiner {
                        line: line.trim_end_matches(['\r', '\n']).to_string(),
                    },
                )
                .is_err()
                {
                    break;
                }
            }
            let _ = outbound_socket.shutdown(Shutdown::Both);
        })
        .ok();

    // Inbound: relay frames -> local NDJSON lines. Joiners receive the inner
    // session line UNWRAPPED — the SessionClient parses it exactly like LAN.
    let mut local_writer = local;
    loop {
        match read_frame_shared(&link.shared, DEFAULT_MAX_FRAME_BYTES) {
            Ok(Some(frame)) => {
                let room_closed = serde_json::from_str::<ServerFrame>(&frame)
                    .map(|server| matches!(server, ServerFrame::RoomClosed))
                    .unwrap_or(false);
                if room_closed {
                    break; // host link died: close local so the client rejoins
                }
                if local_writer.write_all(frame.as_bytes()).is_err()
                    || local_writer.write_all(b"\n").is_err()
                    || local_writer.flush().is_err()
                {
                    break;
                }
            }
            _ => break,
        }
    }
    link.shutdown();
    let _ = local_writer.shutdown(Shutdown::Both);
    if let Some(outbound) = outbound {
        let _ = outbound.join();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_relay::relay_server::{serve_relay, RelayCaps, RelayServer};
    use std::net::TcpListener;

    fn spawn_relay() -> (String, RelayServer) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind ephemeral");
        let address = format!("127.0.0.1:{}", listener.local_addr().unwrap().port());
        let server = serve_relay(listener, RelayCaps::default()).expect("serve");
        (address, server)
    }

    #[test]
    fn host_bridge_claims_a_room_and_the_code_is_the_invite() {
        let (relay_address, _server) = spawn_relay();
        let host = Arc::new(Mutex::new(SessionHost::new()));
        let handle =
            spawn_relay_host_bridge(Arc::clone(&host), &relay_address).expect("host claims");
        let (room, token) = relay_codes::parse_code(&handle.code).expect("code parses");
        assert_eq!(room.len(), relay_codes::ROOM_ID_LEN);
        assert_eq!(token.len(), relay_codes::ROOM_TOKEN_LEN);
        assert!(handle.is_alive());
    }

    #[test]
    fn malformed_codes_are_rejected_before_any_dial() {
        let error = match RelayClientBridge::start("127.0.0.1:1", "not-a-code", "u-1") {
            Err(error) => error,
            Ok(_) => panic!("malformed code must refuse to start a bridge"),
        };
        assert!(error.to_string().contains("malformed invite code"));
    }

    #[test]
    fn joining_an_unknown_room_fails_the_client_handshake() {
        let (relay_address, _server) = spawn_relay();
        let bridge = RelayClientBridge::start(&relay_address, "AAAAAA-BBBBBBBBBB", "u-1")
            .expect("bridge starts");
        let user = crate::session_host::SessionUser {
            user_id: "u-1".into(),
            display_name: "u-1".into(),
            presence_color: [1, 2, 3],
        };
        let result =
            crate::session_client::SessionClient::connect(bridge.local_address(), user);
        assert!(result.is_err(), "unknown room must not produce a session");
    }
}

//! TCP + NDJSON transport for the session host.
//!
//! One listener per hosted document, thread-per-connection. Lines are
//! newline-delimited JSON `ClientMessage`s in; `HostMessage`s out. Each
//! connection gets a dedicated writer task fed by a channel, so a slow peer
//! never blocks the host loop. The host core (`session_host.rs`) stays
//! transport-agnostic: this layer only parses, routes, and drains.
//!
//! Disconnect handling: an EOF/error tears the connection down, removes the
//! user from the roster, and broadcasts the shrink. The record log is never
//! touched on disconnect — the host log remains truth, and the same
//! `user_id` can reconnect (fresh `Welcome` snapshot per Figma's model).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::session_host::{ClientMessage, HostMessage, SessionHost};

/// Default LAN listen port for hosted sessions.
pub const DEFAULT_SESSION_HOST_PORT: u16 = 4747;

/// `THAUM_SESSION_HOST_PORT` overrides the default listen port.
pub fn host_port_from_env() -> u16 {
    std::env::var("THAUM_SESSION_HOST_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_SESSION_HOST_PORT)
}

pub struct SessionHostServer {
    pub port: u16,
    shutdown: Arc<AtomicBool>,
    accept_thread: Option<JoinHandle<()>>,
}

impl SessionHostServer {
    /// Stops accepting, then waits for connection threads to wind down.
    pub fn shutdown(mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(thread) = self.accept_thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for SessionHostServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(thread) = self.accept_thread.take() {
            let _ = thread.join();
        }
    }
}

/// Binds `0.0.0.0:{port}` (port 0 = OS-assigned, read it from `server.port`)
/// and serves the given host core until shutdown/drop.
pub fn spawn_session_host_server(
    host: Arc<Mutex<SessionHost>>,
    port: u16,
) -> std::io::Result<SessionHostServer> {
    let listener = TcpListener::bind(("0.0.0.0", port))?;
    listener.set_nonblocking(true)?;
    let bound_port = listener.local_addr()?.port();
    let shutdown = Arc::new(AtomicBool::new(false));

    let accept_shutdown = Arc::clone(&shutdown);
    let accept_thread = thread::Builder::new()
        .name("session-host-accept".into())
        .spawn(move || {
            // Per-connection outbound senders, keyed by user_id once the
            // connection says `Hello`. Routed-to drains use this map.
            let senders: Arc<Mutex<HashMap<String, mpsc::Sender<String>>>> =
                Arc::new(Mutex::new(HashMap::new()));

            // Outbound pump: the host core only queues; this thread drains
            // every client's queue onto its wire on a short tick. Covers
            // host-local publishes (`apply_local_record`) that no connection
            // thread would otherwise notice, and keeps the per-message drain
            // in `serve_connection` as the low-latency fast path.
            {
                let host = Arc::clone(&host);
                let senders = Arc::clone(&senders);
                let pump_shutdown = Arc::clone(&accept_shutdown);
                let _ = thread::Builder::new()
                    .name("session-host-pump".into())
                    .spawn(move || loop {
                        if pump_shutdown.load(Ordering::SeqCst) {
                            return;
                        }
                        {
                            let mut host = host.lock().expect("session host lock");
                            drain_and_route(&mut host, &senders);
                        }
                        thread::sleep(Duration::from_millis(10));
                    });
            }

            loop {
                if accept_shutdown.load(Ordering::SeqCst) {
                    return;
                }
                match listener.accept() {
                    Ok((stream, _address)) => {
                        let host = Arc::clone(&host);
                        let senders = Arc::clone(&senders);
                        let _ = thread::Builder::new()
                            .name("session-host-conn".into())
                            .spawn(move || serve_connection(host, senders, stream));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(50));
                    }
                    Err(_) => return,
                }
            }
        })?;

    Ok(SessionHostServer {
        port: bound_port,
        shutdown,
        accept_thread: Some(accept_thread),
    })
}

fn serve_connection(
    host: Arc<Mutex<SessionHost>>,
    senders: Arc<Mutex<HashMap<String, mpsc::Sender<String>>>>,
    stream: TcpStream,
) {
    let _ = stream.set_nodelay(true);
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(clone) => clone,
        Err(_) => return,
    });
    let (outbound_tx, outbound_rx) = mpsc::channel::<String>();
    let writer_stream = match stream.try_clone() {
        Ok(clone) => clone,
        Err(_) => return,
    };
    let _writer_thread = thread::Builder::new()
        .name("session-host-writer".into())
        .spawn(move || {
            for line in outbound_rx {
                if write_line(&writer_stream, &line).is_err() {
                    break;
                }
            }
        });

    // Identity is established by the first `Hello`; before that, messages
    // have no owner and are dropped by the host as `NotJoined`.
    let mut joined_user_id: Option<String> = None;

    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break, // peer closed
            Ok(_) => {}
            Err(_) => break,
        }
        let message: ClientMessage = match serde_json::from_str(line.trim()) {
            Ok(message) => message,
            Err(_) => break, // garbage on the wire: fail this connection loudly
        };

        let user_id = match &message {
            ClientMessage::Hello { user, .. } => user.user_id.clone(),
            _ => match &joined_user_id {
                Some(user_id) => user_id.clone(),
                None => continue,
            },
        };

        // Register the outbound sender BEFORE handling: once the host queues
        // messages for this user, any other connection's drain (or the pump)
        // must find the sender, or the messages are silently dropped.
        if joined_user_id.is_none() {
            if let ClientMessage::Hello { .. } = &message {
                senders
                    .lock()
                    .expect("senders lock")
                    .insert(user_id.clone(), outbound_tx.clone());
            }
        }

        let denied_reason = {
            let mut host = host.lock().expect("session host lock");
            match host.handle_client_message(&user_id, message) {
                Ok(()) => {
                    if joined_user_id.is_none() {
                        joined_user_id = Some(user_id.clone());
                    }
                    drain_and_route(&mut host, &senders);
                    None
                }
                Err(rejection) => {
                    // A failed Hello never registered this identity on the
                    // host; drop the early sender registration with it.
                    if joined_user_id.is_none() {
                        senders.lock().expect("senders lock").remove(&user_id);
                    }
                    Some(rejection_reason(rejection))
                }
            }
        };

        if let Some(reason) = denied_reason {
            let denied = serde_json::to_string(&HostMessage::Denied { reason })
                .unwrap_or_else(|_| "{\"type\":\"denied\"}".to_string());
            let _ = outbound_tx.send(denied);
            // Version/identity problems will not fix themselves on this
            // connection; duplicate joins and not-joined chatter also end it.
            break;
        }
    }

    if let Some(user_id) = joined_user_id {
        senders.lock().expect("senders lock").remove(&user_id);
        let mut host = host.lock().expect("session host lock");
        host.disconnect(&user_id);
        drain_and_route(&mut host, &senders);
    }
}

/// Under the host lock: drain every client's queued messages and route each
/// to its connection's writer channel. Called after every state change so
/// fan-out latency is one hop.
fn drain_and_route(
    host: &mut SessionHost,
    senders: &Arc<Mutex<HashMap<String, mpsc::Sender<String>>>>,
) {
    let senders = senders.lock().expect("senders lock");
    for user in host.roster() {
        for message in host.take_outgoing(&user.user_id) {
            if let Ok(text) = serde_json::to_string(&message) {
                if let Some(sender) = senders.get(&user.user_id) {
                    let _ = sender.send(text);
                }
            }
        }
    }
}

fn rejection_reason(rejection: crate::session_host::ClientRejection) -> String {
    use crate::session_host::ClientRejection::*;
    match rejection {
        ProtocolVersion => "protocol-version".to_string(),
        DuplicateUserId => "user-id-in-use".to_string(),
        SnapshotUnavailable => "snapshot-unavailable".to_string(),
        NotJoined => "not-joined".to_string(),
    }
}

fn write_line(stream: &TcpStream, line: &str) -> std::io::Result<()> {
    let mut stream = stream;
    stream.write_all(line.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_host::{SessionHost, SessionUser, SESSION_PROTOCOL_VERSION};
    use std::io::{BufRead, BufReader};
    use std::net::Shutdown;
    use thaum_painter_domain::storage::{SharedCellPatch, SharedDocumentFile};
    use thaum_renderer_domain::CellPoint;

    fn spawn_test_host() -> (Arc<Mutex<SessionHost>>, SessionHostServer) {
        let mut host = SessionHost::new();
        host.set_snapshot_source(Box::new(|| {
            SharedDocumentFile::single_layer("doc-1", "Doc", "layer-1", "Layer 1")
        }));
        let host = Arc::new(Mutex::new(host));
        let server = spawn_session_host_server(Arc::clone(&host), 0).expect("bind ephemeral port");
        (host, server)
    }

    fn connect(port: u16) -> (TcpStream, BufReader<TcpStream>) {
        let stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        stream.set_nodelay(true).unwrap();
        let reader = BufReader::new(stream.try_clone().expect("clone"));
        (stream, reader)
    }

    fn send(stream: &TcpStream, value: &impl serde::Serialize) {
        let mut text = serde_json::to_string(value).expect("serialize");
        text.push('\n');
        let mut stream = stream;
        stream.write_all(text.as_bytes()).expect("send");
        stream.flush().expect("flush");
    }

    fn read_message(reader: &mut BufReader<TcpStream>) -> HostMessage {
        let mut line = String::new();
        reader.read_line(&mut line).expect("read line");
        serde_json::from_str(line.trim()).expect("parse host message")
    }

    /// In-process peer close: dropping the handle is not enough (the server
    /// side's writer clone keeps the socket alive), so force it the way a real
    /// peer's FIN would.
    fn close(stream: &TcpStream) {
        let _ = stream.shutdown(Shutdown::Both);
    }

    fn hello_for(id: &str) -> ClientMessage {
        ClientMessage::Hello {
            user: SessionUser {
                user_id: id.to_string(),
                display_name: id.to_string(),
                presence_color: [0, 255, 0],
            },
            protocol_version: SESSION_PROTOCOL_VERSION,
        }
    }

    fn stroke(
        action_id: &str,
        user_id: &str,
    ) -> thaum_painter_domain::storage::SharedDocumentActionRecord {
        thaum_painter_domain::storage::SharedDocumentActionRecord::cell_patch_set(
            action_id.to_string(),
            "doc-1".to_string(),
            "layer-1".to_string(),
            user_id.to_string(),
            "t".to_string(),
            vec![SharedCellPatch::new(
                CellPoint { x: 1, y: 2, z: 0 },
                None,
                None,
            )],
            None,
        )
    }

    #[test]
    fn two_real_clients_converge_through_sockets() {
        let (host, server) = spawn_test_host();

        let (alice_stream, mut alice_reader) = connect(server.port);
        send(&alice_stream, &hello_for("alice"));
        match read_message(&mut alice_reader) {
            HostMessage::Welcome {
                roster, log_length, ..
            } => {
                assert_eq!(roster.len(), 1);
                assert_eq!(log_length, 0);
            }
            other => panic!("expected welcome, got {other:?}"),
        }

        let (bob_stream, mut bob_reader) = connect(server.port);
        send(&bob_stream, &hello_for("bob"));
        match read_message(&mut bob_reader) {
            HostMessage::Welcome { roster, .. } => assert_eq!(roster.len(), 2),
            other => panic!("expected welcome, got {other:?}"),
        }
        match read_message(&mut alice_reader) {
            HostMessage::Roster { users } => assert_eq!(users.len(), 2),
            other => panic!("expected roster, got {other:?}"),
        }

        // Bob draws; alice receives the record verbatim, bob does not.
        send(
            &bob_stream,
            &ClientMessage::Action {
                record: stroke("b-1", "bob"),
            },
        );
        match read_message(&mut alice_reader) {
            HostMessage::Record { record } => assert_eq!(record.action_id, "b-1"),
            other => panic!("expected record, got {other:?}"),
        }
        assert_eq!(host.lock().unwrap().records().len(), 1);

        // Presence rides the side channel.
        send(
            &alice_stream,
            &ClientMessage::Presence {
                cursor: Some([7, 9, 0]),
            },
        );
        match read_message(&mut bob_reader) {
            HostMessage::Presence { user_id, cursor } => {
                assert_eq!(user_id, "alice");
                assert_eq!(cursor, Some([7, 9, 0]));
            }
            other => panic!("expected presence, got {other:?}"),
        }
        assert_eq!(host.lock().unwrap().records().len(), 1); // presence added none

        // Duplicate user id: denied, connection closes (read gets EOF after
        // the Denied line).
        let (dupe_stream, mut dupe_reader) = connect(server.port);
        send(&dupe_stream, &hello_for("alice"));
        match read_message(&mut dupe_reader) {
            HostMessage::Denied { reason } => assert_eq!(reason, "user-id-in-use"),
            other => panic!("expected denied, got {other:?}"),
        }
        drop(dupe_stream);
        close(&alice_stream);
        close(&bob_stream);
        server.shutdown();
    }

    #[test]
    fn disconnect_shrinks_the_roster_for_survivors() {
        let (host, server) = spawn_test_host();

        let (alice_stream, mut alice_reader) = connect(server.port);
        send(&alice_stream, &hello_for("alice"));
        read_message(&mut alice_reader); // welcome

        let (bob_stream, mut bob_reader) = connect(server.port);
        send(&bob_stream, &hello_for("bob"));
        read_message(&mut bob_reader); // welcome
        read_message(&mut alice_reader); // roster (bob joined)

        close(&bob_stream);
        match read_message(&mut alice_reader) {
            HostMessage::Roster { users } => assert_eq!(users.len(), 1),
            other => panic!("expected roster, got {other:?}"),
        }
        assert!(!host.lock().unwrap().is_connected("bob"));
        close(&alice_stream);
        server.shutdown();
    }
}

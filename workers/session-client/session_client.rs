//! Session client: the connecting side of a hosted multiplayer session.
//!
//! Mirrors `domain/painter-session/sync/`'s `SyncedDocumentSession` semantics
//! over the wire: the caller owns the document runtime (the entrypoint's live
//! runtime is THE runtime); the client feeds it foreign records in host
//! `Welcome` snapshot, applies foreign records in host-arrival order, skips
//! its own (already applied locally at publish). Join is Figma's fresh-copy
//! model: `connect` returns the host's snapshot for the caller to build its
//! runtime from, and every arriving record is applied on top — the client
//! replays the full log, so a reconnect with the same `user_id` converges
//! without any delta catch-up.
//!
//! Transport is the same NDJSON wire as the host: one `ClientMessage` per
//! line out, one `HostMessage` per line in. The `Hello` handshake happens
//! synchronously inside `connect` (snapshot in hand or a loud error), then
//! reader/writer threads take over.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde::de::DeserializeOwned;

use crate::session_host::{ClientMessage, HostMessage, SessionUser, SESSION_PROTOCOL_VERSION};
use thaum_painter_domain::storage::{SharedDocumentActionRecord, SharedDocumentRuntime};

#[derive(Debug)]
pub enum SessionClientError {
    Io(std::io::Error),
    /// Host said no at `Hello` (version mismatch, duplicate id, ...).
    Denied(String),
    /// Host never answered the handshake.
    Timeout,
    Json(String),
    /// The connection died after joining.
    Disconnected,
    /// The host ended the session on purpose — not a drop, never retried.
    SessionEnded,
}

impl std::fmt::Display for SessionClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "session client io error: {error}"),
            Self::Denied(reason) => write!(f, "join denied: {reason}"),
            Self::Timeout => write!(f, "join handshake timed out"),
            Self::Json(error) => write!(f, "bad wire message: {error}"),
            Self::Disconnected => write!(f, "session connection lost"),
            Self::SessionEnded => write!(f, "the host ended the session"),
        }
    }
}

impl std::error::Error for SessionClientError {}

pub struct SessionClient {
    pub user_id: String,
    /// Read cursor into the host's log: how many records this client has
    /// received and applied. Starts at 0 — joiners replay the full log onto
    /// the snapshot, so pre-join edits converge too.
    consumed_count: u64,
    roster: Vec<SessionUser>,
    cursors: HashMap<String, Option<[i32; 3]>>,
    inbound: Arc<Mutex<VecDeque<HostMessage>>>,
    connected: Arc<AtomicBool>,
    /// Set when the host says `Ended` — a purposeful end, never rejoined.
    ended: Arc<AtomicBool>,
    outbound: mpsc::Sender<String>,
    reader_thread: Option<JoinHandle<()>>,
    writer_thread: Option<JoinHandle<()>>,
    stream: TcpStream,
}

impl SessionClient {
    /// Connects and performs the `Hello` handshake. Returns the client plus
    /// the host's snapshot document — the caller builds its runtime from it
    /// (fresh copy; Figma's model). The joiner is already in the roster and
    /// cursors are empty.
    pub fn connect(
        address: &str,
        user: SessionUser,
    ) -> Result<(Self, thaum_painter_domain::storage::SharedDocumentFile), SessionClientError> {
        Self::connect_inner(address, user).map_err(|error| match error {
            JoinError::Io(error) => SessionClientError::Io(error),
            JoinError::Denied(reason) => SessionClientError::Denied(reason),
            JoinError::Timeout => SessionClientError::Timeout,
            JoinError::Json(error) => SessionClientError::Json(error),
        })
    }

    fn connect_inner(
        address: &str,
        user: SessionUser,
    ) -> Result<(Self, thaum_painter_domain::storage::SharedDocumentFile), JoinError> {
        let stream = TcpStream::connect(address)?;
        stream.set_nodelay(true).ok();
        stream.set_read_timeout(Some(Duration::from_secs(5))).ok();

        // Handshake: synchronous, loud, and finished before any threads spawn.
        let mut reader = BufReader::new(stream.try_clone()?);
        let hello = serde_json::to_string(&ClientMessage::Hello {
            user: user.clone(),
            protocol_version: SESSION_PROTOCOL_VERSION,
        })
        .map_err(|error| JoinError::Json(error.to_string()))?;
        {
            let mut stream = &stream;
            stream.write_all(hello.as_bytes())?;
            stream.write_all(b"\n")?;
            stream.flush()?;
        }

        let (snapshot, _log_length, roster) = loop {
            let message: HostMessage = read_message(&mut reader)?;
            match message {
                HostMessage::Welcome {
                    snapshot,
                    log_length,
                    roster,
                } => break (snapshot, log_length, roster),
                HostMessage::Denied { reason } => {
                    let _ = stream.shutdown(Shutdown::Both);
                    return Err(JoinError::Denied(reason));
                }
                // Pre-welcome roster/presence chatter is stale by definition.
                _ => continue,
            }
        };
        stream.set_read_timeout(None).ok();

        let connected = Arc::new(AtomicBool::new(true));
        let ended = Arc::new(AtomicBool::new(false));
        let inbound: Arc<Mutex<VecDeque<HostMessage>>> = Arc::new(Mutex::new(VecDeque::new()));

        // Reader thread: wire -> inbound queue. It takes over the handshake's
        // BufReader itself — a fresh reader could lose lines the handshake
        // buffered past the Welcome (e.g. the replayed history records).
        // EOF or garbage sets connected=false so the frame loop sees the loss.
        let reader_connected = Arc::clone(&connected);
        let reader_inbound = Arc::clone(&inbound);
        let reader_thread = thread::Builder::new()
            .name("session-client-reader".into())
            .spawn(move || {
                let mut reader = reader;
                loop {
                    let mut line = String::new();
                    match reader.read_line(&mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                    match serde_json::from_str::<HostMessage>(line.trim()) {
                        Ok(message) => reader_inbound
                            .lock()
                            .expect("inbound lock")
                            .push_back(message),
                        Err(_) => break, // garbage on the wire: lose the connection loudly
                    }
                }
                reader_connected.store(false, Ordering::SeqCst);
            })?;

        // Writer thread: channel -> wire.
        let (outbound, outbound_rx) = mpsc::channel::<String>();
        let writer_stream = stream.try_clone()?;
        let writer_thread = thread::Builder::new()
            .name("session-client-writer".into())
            .spawn(move || {
                for line in outbound_rx {
                    if write_line(&writer_stream, &line).is_err() {
                        break;
                    }
                }
            })?;

        let mut cursors = HashMap::new();
        for member in &roster {
            cursors.insert(member.user_id.clone(), None);
        }
        let client = Self {
            user_id: user.user_id,
            consumed_count: 0,
            roster,
            cursors,
            inbound,
            connected,
            ended,
            outbound,
            reader_thread: Some(reader_thread),
            writer_thread: Some(writer_thread),
            stream,
        };
        Ok((client, snapshot))
    }

    /// Frame-loop sync: drain inbound messages. Foreign records apply in host
    /// order into the caller's runtime; own records are skipped (applied
    /// locally at publish). Presence and roster messages update the caches
    /// this returns. Returns how many foreign records were applied.
    pub fn sync(
        &mut self,
        runtime: &mut SharedDocumentRuntime,
    ) -> Result<usize, SessionClientError> {
        if !self.is_connected() {
            return Err(if self.session_ended() {
                SessionClientError::SessionEnded
            } else {
                SessionClientError::Disconnected
            });
        }
        let mut applied = 0;
        let messages: Vec<HostMessage> = self
            .inbound
            .lock()
            .expect("inbound lock")
            .drain(..)
            .collect();
        for message in messages {
            match message {
                HostMessage::Record { record } => {
                    self.consumed_count += 1;
                    if record.user_id != self.user_id {
                        runtime.apply_action_record(record);
                        applied += 1;
                    }
                }
                HostMessage::Presence { user_id, cursor } => {
                    self.cursors.insert(user_id, cursor);
                }
                HostMessage::Roster { users } => {
                    for member in &users {
                        self.cursors.entry(member.user_id.clone()).or_insert(None);
                    }
                    self.roster = users;
                }
                HostMessage::Pong | HostMessage::Welcome { .. } => {}
                HostMessage::Ended => {
                    self.ended.store(true, Ordering::SeqCst);
                    self.connected.store(false, Ordering::SeqCst);
                    return Err(SessionClientError::SessionEnded);
                }
                HostMessage::Denied { reason } => {
                    self.connected.store(false, Ordering::SeqCst);
                    return Err(SessionClientError::Denied(reason));
                }
            }
        }
        Ok(applied)
    }

    /// Ship one record the caller already applied locally and appended to its
    /// own log (the canonical apply-then-ship flow — revert records included;
    /// the caller's history stacks were mutated by the undo/redo itself).
    pub fn send_action(
        &self,
        record: SharedDocumentActionRecord,
    ) -> Result<(), SessionClientError> {
        self.send(ClientMessage::Action { record })
    }

    pub fn send_presence(&self, cursor: Option<[i32; 3]>) -> Result<(), SessionClientError> {
        self.send(ClientMessage::Presence { cursor })
    }

    pub fn ping(&self) -> Result<(), SessionClientError> {
        self.send(ClientMessage::Ping)
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    /// True once the host said `Ended` — a purposeful session end, distinct
    /// from a drop. Rejoin logic must never retry past this.
    pub fn session_ended(&self) -> bool {
        self.ended.load(Ordering::SeqCst)
    }

    /// Forces the socket down the way a real peer drop would, so the reader
    /// thread notices the loss. Test exposure for rejoin/drop behavior.
    pub fn force_disconnect(&self) {
        let _ = self.stream.shutdown(Shutdown::Both);
    }

    pub fn roster(&self) -> &[SessionUser] {
        &self.roster
    }

    pub fn cursor(&self, user_id: &str) -> Option<[i32; 3]> {
        self.cursors.get(user_id).copied().flatten()
    }

    pub fn consumed_count(&self) -> u64 {
        self.consumed_count
    }

    /// Full teardown: closes the socket (which ends the reader thread), drops
    /// the outbound channel (which ends the writer), and joins both.
    pub fn shutdown(mut self) {
        let _ = self.stream.shutdown(Shutdown::Both);
        self.connected.store(false, Ordering::SeqCst);
        self.outbound = {
            let (dead, _) = mpsc::channel();
            dead
        };
        if let Some(thread) = self.reader_thread.take() {
            let _ = thread.join();
        }
        if let Some(thread) = self.writer_thread.take() {
            let _ = thread.join();
        }
    }

    fn send(&self, message: ClientMessage) -> Result<(), SessionClientError> {
        if !self.is_connected() {
            return Err(SessionClientError::Disconnected);
        }
        let text = serde_json::to_string(&message)
            .map_err(|error| SessionClientError::Json(error.to_string()))?;
        self.outbound
            .send(text)
            .map_err(|_| SessionClientError::Disconnected)
    }
}

enum JoinError {
    Io(std::io::Error),
    Denied(String),
    Timeout,
    Json(String),
}

impl From<std::io::Error> for JoinError {
    fn from(error: std::io::Error) -> Self {
        JoinError::Io(error)
    }
}

fn read_message<T: DeserializeOwned>(reader: &mut BufReader<TcpStream>) -> Result<T, JoinError> {
    let mut line = String::new();
    reader.read_line(&mut line)?;
    if line.is_empty() {
        return Err(JoinError::Io(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "connection closed during handshake",
        )));
    }
    serde_json::from_str(line.trim()).map_err(|error| JoinError::Json(error.to_string()))
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
    use crate::session_host::SessionHost;
    use crate::session_host_tcp::{spawn_session_host_server, SessionHostServer};
    use std::net::Shutdown;
    use std::sync::{Arc, Mutex as StdMutex};
    use thaum_painter_domain::brush::PaintedCell;
    use thaum_painter_domain::paint_color::PaintColor;
    use thaum_painter_domain::storage::{
        SharedCellPatch, SharedDocumentActionRecord, SharedDocumentFile,
    };
    use thaum_renderer_domain::{CellGraphic, CellPoint};

    fn spawn_host() -> (Arc<StdMutex<SessionHost>>, SessionHostServer) {
        let mut host = SessionHost::new();
        host.set_snapshot_source(Box::new(|| {
            SharedDocumentFile::single_layer("doc-1", "Doc", "layer-1", "Layer 1")
        }));
        let host = Arc::new(StdMutex::new(host));
        let server = spawn_session_host_server(Arc::clone(&host), 0).expect("bind ephemeral port");
        (host, server)
    }

    fn session_user(id: &str) -> SessionUser {
        SessionUser {
            user_id: id.to_string(),
            display_name: id.to_string(),
            presence_color: [10, 200, 30],
        }
    }

    /// One test user: client + the caller-owned runtime it syncs into.
    struct TestPeer {
        client: SessionClient,
        runtime: SharedDocumentRuntime,
    }

    impl TestPeer {
        fn connect(port: u16, id: &str) -> Self {
            let (client, snapshot) =
                SessionClient::connect(&format!("127.0.0.1:{port}"), session_user(id))
                    .expect("client joins");
            Self {
                client,
                runtime: SharedDocumentRuntime::new(snapshot),
            }
        }

        /// The canonical apply-then-ship publish flow.
        fn publish(&mut self, record: SharedDocumentActionRecord) {
            self.runtime.apply_action_record(record.clone());
            self.client.send_action(record).expect("send action");
        }

        fn sync(&mut self) -> usize {
            self.client.sync(&mut self.runtime).expect("sync")
        }

        /// Wire hops are async; poll sync until the expected record count lands.
        fn sync_for(&mut self, expected: usize) -> usize {
            let mut applied = 0;
            for _ in 0..100 {
                applied += self.sync();
                if applied >= expected {
                    return applied;
                }
                thread::sleep(Duration::from_millis(10));
            }
            applied
        }

        fn canvas_len(&self) -> usize {
            self.runtime
                .canvas_for_layer("layer-1", 0)
                .cloned()
                .unwrap_or_default()
                .len()
        }

        fn stroke(
            &self,
            action_id: &str,
            position: CellPoint,
            color: (u8, u8, u8),
        ) -> SharedDocumentActionRecord {
            SharedDocumentActionRecord::cell_patch_set(
                action_id.to_string(),
                self.runtime.document.document_id.clone(),
                "layer-1".to_string(),
                self.client.user_id.clone(),
                "t".to_string(),
                vec![SharedCellPatch::new(position, None, Some(&paint(color)))],
                None,
            )
        }
    }

    fn paint(color: (u8, u8, u8)) -> PaintedCell {
        PaintedCell {
            graphic: CellGraphic::Glyph('a'),
            color: PaintColor::FlatRgb(color.0, color.1, color.2),
            weight_index: 2,
        }
    }

    #[test]
    fn join_returns_snapshot_and_roster_includes_self() {
        let (_host, server) = spawn_host();
        let peer = TestPeer::connect(server.port, "bob");
        assert_eq!(peer.client.consumed_count(), 0);
        assert_eq!(peer.client.roster().len(), 1);
        assert_eq!(peer.client.roster()[0].user_id, "bob");
        assert_eq!(peer.canvas_len(), 0);
        peer.client.shutdown();
        server.shutdown();
    }

    #[test]
    fn two_clients_converge_through_publish_and_sync() {
        let (_host, server) = spawn_host();
        let mut alice = TestPeer::connect(server.port, "alice");
        let mut bob = TestPeer::connect(server.port, "bob");
        alice.sync(); // roster (bob joined)
        bob.sync();

        // Alice paints; bob syncs it in.
        let record = alice.stroke("a-1", CellPoint { x: 0, y: 0, z: 0 }, (255, 0, 0));
        alice.publish(record);
        assert_eq!(alice.canvas_len(), 1); // applied locally at publish
        assert_eq!(bob.sync_for(1), 1);
        assert_eq!(bob.canvas_len(), 1);

        // Bob paints back; alice syncs. Interleaved log order holds.
        let record = bob.stroke("b-1", CellPoint { x: 1, y: 0, z: 0 }, (0, 255, 0));
        bob.publish(record);
        assert_eq!(alice.sync_for(1), 1);
        assert_eq!(alice.canvas_len(), 2);
        // Each client's cursor has consumed only the other's record — the
        // host excludes the sender from its own record's broadcast.
        assert_eq!(alice.client.consumed_count(), 1);
        assert_eq!(bob.client.consumed_count(), 1);

        // Re-syncing applies nothing (own records skipped, cursor advanced).
        assert_eq!(alice.sync(), 0);
        assert_eq!(bob.sync(), 0);

        alice.client.shutdown();
        bob.client.shutdown();
        server.shutdown();
    }

    #[test]
    fn late_joiner_replays_the_full_log_onto_the_snapshot() {
        let (_host, server) = spawn_host();
        let mut alice = TestPeer::connect(server.port, "alice");
        alice.sync();
        let record = alice.stroke("a-1", CellPoint { x: 0, y: 0, z: 0 }, (9, 0, 0));
        alice.publish(record);
        let record = alice.stroke("a-2", CellPoint { x: 1, y: 0, z: 0 }, (0, 9, 0));
        alice.publish(record);

        // Carol joins after two strokes: snapshot plus full-log replay.
        let mut carol = TestPeer::connect(server.port, "carol");
        assert_eq!(carol.sync_for(2), 2);
        assert_eq!(carol.canvas_len(), 2);
        assert_eq!(carol.client.consumed_count(), 2);

        alice.client.shutdown();
        carol.client.shutdown();
        server.shutdown();
    }

    #[test]
    fn presence_round_trips_between_clients() {
        let (_host, server) = spawn_host();
        let mut alice = TestPeer::connect(server.port, "alice");
        let mut bob = TestPeer::connect(server.port, "bob");
        alice.sync();

        alice.client.send_presence(Some([3, 4, 0])).unwrap();
        // Wait for the wire hop, then sync bob's side.
        for _ in 0..50 {
            bob.sync();
            if bob.client.cursor("alice") == Some([3, 4, 0]) {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(bob.client.cursor("alice"), Some([3, 4, 0]));
        assert_eq!(bob.client.cursor("bob"), None);

        alice.client.shutdown();
        bob.client.shutdown();
        server.shutdown();
    }

    #[test]
    fn host_ended_is_distinct_from_a_drop() {
        let (_host, server) = spawn_host();
        let mut peer = TestPeer::connect(server.port, "alice");
        peer.sync();

        _host.lock().unwrap().end_session();
        let mut ended = false;
        for _ in 0..100 {
            match peer.client.sync(&mut peer.runtime) {
                Err(SessionClientError::SessionEnded) => {
                    ended = true;
                    break;
                }
                _ => thread::sleep(Duration::from_millis(10)),
            }
        }
        assert!(ended);
        assert!(peer.client.session_ended());
        assert!(!peer.client.is_connected());
        peer.client.shutdown();
        server.shutdown();
    }

    #[test]
    fn duplicate_identity_join_is_denied_loudly() {
        let (_host, server) = spawn_host();
        let alice = TestPeer::connect(server.port, "alice");
        let result =
            SessionClient::connect(&format!("127.0.0.1:{}", server.port), session_user("alice"));
        match result {
            Err(SessionClientError::Denied(reason)) => assert_eq!(reason, "user-id-in-use"),
            Err(other) => panic!("expected denied, got {other}"),
            Ok(_) => panic!("expected denied, got a session"),
        }
        alice.client.shutdown();
        server.shutdown();
    }

    #[test]
    fn disconnect_is_visible_to_the_frame_loop() {
        let (_host, server) = spawn_host();
        let alice = TestPeer::connect(server.port, "alice");
        alice.client.stream.shutdown(Shutdown::Both).unwrap();
        // Reader thread notices EOF and clears connected.
        for _ in 0..50 {
            if !alice.client.is_connected() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!alice.client.is_connected());
        alice.client.shutdown();
        server.shutdown();
    }
}

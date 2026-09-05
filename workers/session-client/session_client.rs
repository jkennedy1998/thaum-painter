//! Session client: the connecting side of a hosted multiplayer session.
//!
//! Mirrors `domain/painter-session/sync/`'s `SyncedDocumentSession` semantics
//! over the wire: the client owns a document runtime built from the host's
//! `Welcome` snapshot, applies foreign records in host-arrival order, skips
//! its own (already applied locally at publish), and publishes its own edits
//! with the canonical append-then-apply flow. Nothing here is UI; the owning
//! painter app drives `sync` from its frame loop and reads presence/roster
//! caches after it.
//!
//! Transport is the same NDJSON wire as the host: one `ClientMessage` per
//! line out, one `HostMessage` per line in. The `Hello` handshake happens
//! synchronously inside `connect` (snapshot in hand or a loud error), then
//! reader/writer threads take over. Reconnect is a fresh `connect` — same
//! `user_id`, fresh snapshot, Figma's model; there is no delta catch-up.

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
}

impl std::fmt::Display for SessionClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "session client io error: {error}"),
            Self::Denied(reason) => write!(f, "join denied: {reason}"),
            Self::Timeout => write!(f, "join handshake timed out"),
            Self::Json(error) => write!(f, "bad wire message: {error}"),
            Self::Disconnected => write!(f, "session connection lost"),
        }
    }
}

impl std::error::Error for SessionClientError {}

pub struct SessionClient {
    pub user_id: String,
    pub runtime: SharedDocumentRuntime,
    /// Read cursor into the host's log: how many records this client has
    /// received. Starts at the `Welcome` log length; every `Record` advances it.
    consumed_count: u64,
    roster: Vec<SessionUser>,
    cursors: HashMap<String, Option<[i32; 3]>>,
    inbound: Arc<Mutex<VecDeque<HostMessage>>>,
    connected: Arc<AtomicBool>,
    outbound: mpsc::Sender<String>,
    reader_thread: Option<JoinHandle<()>>,
    writer_thread: Option<JoinHandle<()>>,
    stream: TcpStream,
}

impl SessionClient {
    /// Connects, performs the `Hello` handshake, and builds the local runtime
    /// from the host's snapshot. Returns with the joiner already in the
    /// roster (the `Welcome` roster includes self) and cursors empty.
    pub fn connect(address: &str, user: SessionUser) -> Result<Self, SessionClientError> {
        Self::connect_with_runtime(address, user, SharedDocumentRuntime::new)
            .map(|(client, _)| client)
            .map_err(|error| match error {
                JoinError::Io(error) => SessionClientError::Io(error),
                JoinError::Denied(reason) => SessionClientError::Denied(reason),
                JoinError::Timeout => SessionClientError::Timeout,
                JoinError::Json(error) => SessionClientError::Json(error),
            })
    }

    /// Like `connect`, but the runtime is built by the caller from the
    /// snapshot (handy for tests or custom construction). Returns the client
    /// plus the `Welcome` log length as the read cursor baseline.
    pub fn connect_with_runtime<F>(
        address: &str,
        user: SessionUser,
        build_runtime: F,
    ) -> Result<(Self, u64), JoinError>
    where
        F: FnOnce(thaum_painter_domain::storage::SharedDocumentFile) -> SharedDocumentRuntime,
    {
        let stream = TcpStream::connect(address)?;
        stream.set_nodelay(true).ok();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .ok();

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

        let (snapshot, log_length, roster) = loop {
            let message: HostMessage = read_message(&mut reader)?;
            match message {
                HostMessage::Welcome { snapshot, log_length, roster } => {
                    break (snapshot, log_length, roster)
                }
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
        let inbound: Arc<Mutex<VecDeque<HostMessage>>> =
            Arc::new(Mutex::new(VecDeque::new()));

        // Reader thread: wire -> inbound queue. EOF or garbage sets
        // connected=false so the frame loop can see the loss.
        let reader_stream = stream.try_clone()?;
        let reader_connected = Arc::clone(&connected);
        let reader_inbound = Arc::clone(&inbound);
        let reader_thread = thread::Builder::new()
            .name("session-client-reader".into())
            .spawn(move || {
                let mut reader = BufReader::new(reader_stream);
                loop {
                    let mut line = String::new();
                    match reader.read_line(&mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                    match serde_json::from_str::<HostMessage>(line.trim()) {
                        Ok(message) => {
                            reader_inbound.lock().expect("inbound lock").push_back(message)
                        }
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

        let runtime = build_runtime(snapshot);
        let mut cursors = HashMap::new();
        for member in &roster {
            cursors.insert(member.user_id.clone(), None);
        }
        let client = Self {
            user_id: user.user_id,
            runtime,
            consumed_count: log_length,
            roster,
            cursors,
            inbound,
            connected,
            outbound,
            reader_thread: Some(reader_thread),
            writer_thread: Some(writer_thread),
            stream,
        };
        Ok((client, log_length))
    }

    /// Frame-loop sync: drain inbound messages. Foreign records apply in host
    /// order; own records are skipped (applied locally at publish). Presence
    /// and roster messages update the caches this returns. Returns how many
    /// foreign records were applied.
    pub fn sync(&mut self) -> Result<usize, SessionClientError> {
        if !self.is_connected() {
            return Err(SessionClientError::Disconnected);
        }
        let mut applied = 0;
        let messages: Vec<HostMessage> =
            self.inbound.lock().expect("inbound lock").drain(..).collect();
        for message in messages {
            match message {
                HostMessage::Record { record } => {
                    self.consumed_count += 1;
                    if record.user_id != self.user_id {
                        self.runtime.apply_action_record(record);
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
                HostMessage::Denied { reason } => {
                    self.connected.store(false, Ordering::SeqCst);
                    return Err(SessionClientError::Denied(reason));
                }
            }
        }
        Ok(applied)
    }

    /// Forward-edit publish: apply locally, then ship (canonical
    /// append-then-apply minus the disk append — the host owns the log).
    pub fn publish_action(&mut self, record: SharedDocumentActionRecord) -> Result<(), SessionClientError> {
        self.runtime.apply_action_record(record.clone());
        self.send(ClientMessage::Action { record })
    }

    /// History (undo/redo) publish: the local runtime already mutated its
    /// canvases via `undo_top_action`/`redo_top_action`, so the record is
    /// pushed onto the history stacks without re-applying — the live
    /// `apply_shared_history_action` flow, shipped.
    pub fn publish_history_record(
        &mut self,
        record: SharedDocumentActionRecord,
    ) -> Result<(), SessionClientError> {
        self.runtime.push_history_record(record.clone());
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
    use thaum_painter_domain::storage::{SharedCellPatch, SharedDocumentFile, SharedDocumentActionRecord};
    use thaum_renderer_domain::{CellGraphic, CellPoint};

    fn spawn_host() -> (Arc<StdMutex<SessionHost>>, SessionHostServer) {
        let mut host = SessionHost::new();
        host.set_snapshot_source(Box::new(|| {
            SharedDocumentFile::single_layer("doc-1", "Doc", "layer-1", "Layer 1")
        }));
        let host = Arc::new(StdMutex::new(host));
        let server =
            spawn_session_host_server(Arc::clone(&host), 0).expect("bind ephemeral port");
        (host, server)
    }

    fn session_user(id: &str) -> SessionUser {
        SessionUser {
            user_id: id.to_string(),
            display_name: id.to_string(),
            presence_color: [10, 200, 30],
        }
    }

    fn connect_client(port: u16, id: &str) -> SessionClient {
        SessionClient::connect(&format!("127.0.0.1:{port}"), session_user(id))
            .expect("client joins")
    }

    fn paint(color: (u8, u8, u8)) -> PaintedCell {
        PaintedCell {
            graphic: CellGraphic::Glyph('a'),
            color: PaintColor::FlatRgb(color.0, color.1, color.2),
            weight_index: 2,
        }
    }

    fn canvas_len(client: &SessionClient) -> usize {
        client
            .runtime
            .canvas_for_layer("layer-1", 0)
            .cloned()
            .unwrap_or_default()
            .len()
    }

    fn stroke_record(
        client: &SessionClient,
        action_id: &str,
        position: CellPoint,
        color: (u8, u8, u8),
    ) -> SharedDocumentActionRecord {
        SharedDocumentActionRecord::cell_patch_set(
            action_id.to_string(),
            client.runtime.document.document_id.clone(),
            "layer-1".to_string(),
            client.user_id.clone(),
            "t".to_string(),
            vec![SharedCellPatch::new(position, None, Some(&paint(color)))],
            None,
        )
    }

    /// Wire hops are async; poll sync until the expected record count lands.
    fn sync_for(client: &mut SessionClient, expected: usize) -> usize {
        let mut applied = 0;
        for _ in 0..100 {
            applied += client.sync().unwrap();
            if applied >= expected {
                return applied;
            }
            thread::sleep(Duration::from_millis(10));
        }
        applied
    }

    #[test]
    fn join_builds_runtime_from_host_snapshot_and_roster_includes_self() {
        let (_host, server) = spawn_host();
        let client = connect_client(server.port, "bob");
        assert_eq!(client.consumed_count(), 0);
        assert_eq!(client.roster().len(), 1);
        assert_eq!(client.roster()[0].user_id, "bob");
        assert_eq!(canvas_len(&client), 0);
        client.shutdown();
        server.shutdown();
    }

    #[test]
    fn two_clients_converge_through_publish_and_sync() {
        let (_host, server) = spawn_host();
        let mut alice = connect_client(server.port, "alice");
        let mut bob = connect_client(server.port, "bob");
        alice.sync().unwrap(); // roster (bob joined)
        bob.sync().unwrap();

        // Alice paints; bob syncs it in.
        let record = stroke_record(&alice, "a-1", CellPoint { x: 0, y: 0, z: 0 }, (255, 0, 0));
        alice.publish_action(record).unwrap();
        assert_eq!(canvas_len(&alice), 1); // applied locally at publish
        assert_eq!(sync_for(&mut bob, 1), 1);
        assert_eq!(canvas_len(&bob), 1);

        // Bob paints back; alice syncs. Interleaved log order holds.
        let record = stroke_record(&bob, "b-1", CellPoint { x: 1, y: 0, z: 0 }, (0, 255, 0));
        bob.publish_action(record).unwrap();
        assert_eq!(sync_for(&mut alice, 1), 1);
        assert_eq!(canvas_len(&alice), 2);
        // Each client's cursor has consumed only the other's record — the
        // host excludes the sender from its own record's broadcast.
        assert_eq!(alice.consumed_count(), 1);
        assert_eq!(bob.consumed_count(), 1);

        // Re-syncing applies nothing (own records skipped, cursor advanced).
        assert_eq!(alice.sync().unwrap(), 0);
        assert_eq!(bob.sync().unwrap(), 0);

        alice.shutdown();
        bob.shutdown();
        server.shutdown();
    }

    #[test]
    fn presence_round_trips_between_clients() {
        let (_host, server) = spawn_host();
        let mut alice = connect_client(server.port, "alice");
        let mut bob = connect_client(server.port, "bob");
        alice.sync().unwrap();

        alice.send_presence(Some([3, 4, 0])).unwrap();
        // Wait for the wire hop, then sync bob's side.
        for _ in 0..50 {
            bob.sync().unwrap();
            if bob.cursor("alice") == Some([3, 4, 0]) {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(bob.cursor("alice"), Some([3, 4, 0]));
        assert_eq!(bob.cursor("bob"), None);

        alice.shutdown();
        bob.shutdown();
        server.shutdown();
    }

    #[test]
    fn duplicate_identity_join_is_denied_loudly() {
        let (_host, server) = spawn_host();
        let _alice = connect_client(server.port, "alice");
        let result = SessionClient::connect(&format!("127.0.0.1:{}", server.port), session_user("alice"));
        match result {
            Err(SessionClientError::Denied(reason)) => assert_eq!(reason, "user-id-in-use"),
            Err(other) => panic!("expected denied, got {other}"),
            Ok(_) => panic!("expected denied, got a session"),
        }
        server.shutdown();
    }

    #[test]
    fn disconnect_is_visible_to_the_frame_loop() {
        let (_host, server) = spawn_host();
        let alice = connect_client(server.port, "alice");
        alice.stream.shutdown(Shutdown::Both).unwrap();
        // Reader thread notices EOF and clears connected.
        for _ in 0..50 {
            if !alice.is_connected() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!alice.is_connected());
        server.shutdown();
    }
}

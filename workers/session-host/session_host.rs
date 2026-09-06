//! Session host: the host-authoritative core of LAN multiplayer.
//!
//! The host owns three things and nothing else: the ordered record log (arrival
//! order is the sync order — no timestamps arbitrate, matching Figma's model),
//! the connection roster, and presence cursors. Records are opaque to the host:
//! it orders, logs, and broadcasts without interpreting them, so open
//! permissions for all users cost the transport nothing — content edits today,
//! structure-edit records when the domain gains those variants.
//!
//! Document semantics stay in `domain/`: the host never applies records to a
//! document itself. The `Welcome` snapshot is supplied by the owning painter
//! app through `set_snapshot_source` — joiners get a fresh copy of the current
//! document plus the log length as their read cursor, then live records after
//! it (Figma's reconnect model; rejoin = fresh snapshot, no delta catch-up).
//!
//! This module is transport-agnostic and in-memory; `tcp.rs` wraps it in a
//! TCP + NDJSON listener. Tests here exercise ordering, denial, snapshot, and
//! presence with plain queues — no sockets.

use std::collections::{HashMap, VecDeque};

use serde::{Deserialize, Serialize};

use thaum_painter_domain::storage::{SharedDocumentActionRecord, SharedDocumentFile};

/// Bumped on any wire-shape change; a `Hello` with a mismatched version is
/// denied so old clients fail loudly instead of corrupting sessions.
pub const SESSION_PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionUser {
    pub user_id: String,
    pub display_name: String,
    pub presence_color: [u8; 3],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ClientMessage {
    Hello { user: SessionUser, protocol_version: u32 },
    /// Any `SharedDocumentActionRecord`. Opaque to the host: it is appended at
    /// arrival order and broadcast verbatim.
    Action { record: SharedDocumentActionRecord },
    /// Presence rides this channel, never the record log. Cursor is in cell
    /// space, matching `CellPoint` (x, y, z).
    Presence { cursor: Option<[i32; 3]> },
    Ping,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum HostMessage {
    /// Fresh copy of the document + how many records predate the joiner. The
    /// joiner resets its runtime from the snapshot, then applies live records.
    Welcome { snapshot: SharedDocumentFile, log_length: u64, roster: Vec<SessionUser> },
    Record { record: SharedDocumentActionRecord },
    Presence { user_id: String, cursor: Option<[i32; 3]> },
    Roster { users: Vec<SessionUser> },
    Denied { reason: String },
    Pong,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientRejection {
    /// `Hello` version mismatch — old client, fail loudly.
    ProtocolVersion,
    /// Same `user_id` already connected. Rejoin = disconnect then `Hello`
    /// again; a live duplicate identity would fork ownership.
    DuplicateUserId,
    /// No snapshot source wired, so the host cannot hand out a fresh copy.
    SnapshotUnavailable,
    /// `Action`/`Presence`/`Ping` before a successful `Hello`.
    NotJoined,
}

pub struct HostClient {
    pub user: SessionUser,
    pub outgoing: VecDeque<HostMessage>,
}

/// Transport-agnostic session host core. One instance per hosted document.
pub struct SessionHost {
    /// The authoritative sync log in arrival order. The host's owning app
    /// persists this to `actions.jsonl` (host owns saves in session mode).
    records: Vec<SharedDocumentActionRecord>,
    clients: Vec<HostClient>,
    cursors: HashMap<String, Option<[i32; 3]>>,
    snapshot_source: Option<Box<dyn Fn() -> SharedDocumentFile + Send>>,
}

impl Default for SessionHost {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionHost {
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            clients: Vec::new(),
            cursors: HashMap::new(),
            snapshot_source: None,
        }
    }

    /// The owning painter app supplies the current document at join time; the
    /// host itself never touches document semantics.
    pub fn set_snapshot_source(
        &mut self,
        source: Box<dyn Fn() -> SharedDocumentFile + Send>,
    ) {
        self.snapshot_source = Some(source);
    }

    pub fn records(&self) -> &[SharedDocumentActionRecord] {
        &self.records
    }

    pub fn roster(&self) -> Vec<SessionUser> {
        self.clients.iter().map(|client| client.user.clone()).collect()
    }

    pub fn is_connected(&self, user_id: &str) -> bool {
        self.clients.iter().any(|client| client.user.user_id == user_id)
    }

    pub fn cursor(&self, user_id: &str) -> Option<[i32; 3]> {
        self.cursors.get(user_id).copied().flatten()
    }

    /// Handles one client message. Rejections are returned, never queued —
    /// the transport turns them into `Denied` on the offending connection.
    pub fn handle_client_message(
        &mut self,
        user_id: &str,
        message: ClientMessage,
    ) -> Result<(), ClientRejection> {
        match message {
            ClientMessage::Hello { user, protocol_version } => {
                if protocol_version != SESSION_PROTOCOL_VERSION {
                    return Err(ClientRejection::ProtocolVersion);
                }
                if user.user_id != user_id {
                    return Err(ClientRejection::NotJoined);
                }
                if self.is_connected(user_id) {
                    return Err(ClientRejection::DuplicateUserId);
                }
                let snapshot = match &self.snapshot_source {
                    Some(source) => source(),
                    None => return Err(ClientRejection::SnapshotUnavailable),
                };
                let log_length = self.records.len() as u64;
                self.clients.push(HostClient {
                    user: user.clone(),
                    outgoing: VecDeque::new(),
                });
                self.cursors.insert(user.user_id.clone(), None);
                self.queue_to(&user.user_id, HostMessage::Welcome {
                    snapshot,
                    log_length,
                    roster: self.roster(),
                });
                // Full-log replay: the joiner's snapshot is the document as of
                // host-log start, so it applies every historical record from
                // cursor 0 (Figma's fresh-copy-plus-replay convergence).
                let history: Vec<SharedDocumentActionRecord> = self.records.clone();
                for record in history {
                    self.queue_to(&user.user_id, HostMessage::Record { record });
                }
                self.broadcast_others(
                    &user.user_id,
                    HostMessage::Roster { users: self.roster() },
                );
                Ok(())
            }
            ClientMessage::Action { record } => {
                if !self.is_connected(user_id) {
                    return Err(ClientRejection::NotJoined);
                }
                // Arrival order defines the sync order; no stamping, no
                // interpretation — the record is broadcast verbatim.
                self.records.push(record.clone());
                self.broadcast_others(user_id, HostMessage::Record { record });
                Ok(())
            }
            ClientMessage::Presence { cursor } => {
                if !self.is_connected(user_id) {
                    return Err(ClientRejection::NotJoined);
                }
                self.cursors.insert(user_id.to_string(), cursor);
                self.broadcast_others(
                    user_id,
                    HostMessage::Presence { user_id: user_id.to_string(), cursor },
                );
                Ok(())
            }
            ClientMessage::Ping => {
                if !self.is_connected(user_id) {
                    return Err(ClientRejection::NotJoined);
                }
                self.queue_to(user_id, HostMessage::Pong);
                Ok(())
            }
        }
    }

    /// Connection teardown: roster shrinks, presence is dropped, survivors
    /// learn via a roster broadcast. The log is untouched — no record loss.
    pub fn disconnect(&mut self, user_id: &str) {
        self.clients.retain(|client| client.user.user_id != user_id);
        self.cursors.remove(user_id);
        self.broadcast_others(user_id, HostMessage::Roster { users: self.roster() });
    }

    /// The host user's own edits: the local app already applied the record
    /// to its runtime, so this only enters the log and broadcasts to every
    /// connected client (arrival order keeps the log totally ordered).
    pub fn apply_local_record(&mut self, record: SharedDocumentActionRecord) {
        self.records.push(record.clone());
        for client in &mut self.clients {
            client.outgoing.push_back(HostMessage::Record { record: record.clone() });
        }
    }

    /// Drains one client's queued messages (the transport's read side).
    pub fn take_outgoing(&mut self, user_id: &str) -> Vec<HostMessage> {
        match self.clients.iter_mut().find(|client| client.user.user_id == user_id) {
            Some(client) => client.outgoing.drain(..).collect(),
            None => Vec::new(),
        }
    }

    fn queue_to(&mut self, user_id: &str, message: HostMessage) {
        if let Some(client) =
            self.clients.iter_mut().find(|client| client.user.user_id == user_id)
        {
            client.outgoing.push_back(message);
        }
    }

    fn broadcast_others(&mut self, sender_id: &str, message: HostMessage) {
        for client in &mut self.clients {
            if client.user.user_id != sender_id {
                client.outgoing.push_back(message.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use thaum_painter_domain::storage::{SharedCellPatch, SharedDocumentFile};
    use thaum_renderer_domain::CellPoint;

    fn user(id: &str) -> SessionUser {
        SessionUser {
            user_id: id.to_string(),
            display_name: id.to_string(),
            presence_color: [255, 0, 0],
        }
    }

    fn snapshot_document() -> SharedDocumentFile {
        SharedDocumentFile::single_layer("doc-1", "Doc", "layer-1", "Layer 1")
    }

    fn host() -> SessionHost {
        let mut host = SessionHost::new();
        host.set_snapshot_source(Box::new(snapshot_document));
        host
    }

    fn hello(host: &mut SessionHost, id: &str) {
        host.handle_client_message(id, ClientMessage::Hello {
            user: user(id),
            protocol_version: SESSION_PROTOCOL_VERSION,
        })
        .expect("hello accepted");
    }

    fn stroke(action_id: &str, user_id: &str, x: i32) -> SharedDocumentActionRecord {
        SharedDocumentActionRecord::cell_patch_set(
            action_id.to_string(),
            "doc-1".to_string(),
            "layer-1".to_string(),
            user_id.to_string(),
            "t".to_string(),
            vec![SharedCellPatch::new(CellPoint { x, y: 0, z: 0 }, None, None)],
            None,
        )
    }

    #[test]
    fn join_gets_welcome_snapshot_and_others_get_roster() {
        let mut host = host();
        hello(&mut host, "alice");
        host.take_outgoing("alice");
        hello(&mut host, "bob");

        let bob_out = host.take_outgoing("bob");
        assert_eq!(bob_out.len(), 1);
        match &bob_out[0] {
            HostMessage::Welcome { snapshot, log_length, roster } => {
                assert_eq!(snapshot.document_id, snapshot_document().document_id);
                assert_eq!(*log_length, 0);
                assert_eq!(roster.len(), 2);
            }
            other => panic!("expected welcome, got {other:?}"),
        }

        let alice_out = host.take_outgoing("alice");
        assert_eq!(alice_out, vec![HostMessage::Roster { users: host.roster() }]);
    }

    #[test]
    fn actions_log_in_arrival_order_and_broadcast_to_others_only() {
        let mut host = host();
        hello(&mut host, "alice");
        hello(&mut host, "bob");
        host.take_outgoing("alice");
        host.take_outgoing("bob");

        host.handle_client_message("alice", ClientMessage::Action { record: stroke("a-1", "alice", 0) })
            .unwrap();
        host.handle_client_message("bob", ClientMessage::Action { record: stroke("b-1", "bob", 1) })
            .unwrap();
        host.handle_client_message("alice", ClientMessage::Action { record: stroke("a-2", "alice", 2) })
            .unwrap();

        assert_eq!(host.records().len(), 3);
        assert_eq!(host.records()[0].action_id, "a-1");
        assert_eq!(host.records()[1].action_id, "b-1");
        assert_eq!(host.records()[2].action_id, "a-2");

        let alice_out = host.take_outgoing("alice");
        assert_eq!(
            alice_out,
            vec![HostMessage::Record { record: stroke("b-1", "bob", 1) }]
        );
        let bob_out = host.take_outgoing("bob");
        assert_eq!(bob_out.len(), 2);
        assert_eq!(bob_out[0], HostMessage::Record { record: stroke("a-1", "alice", 0) });
        assert_eq!(bob_out[1], HostMessage::Record { record: stroke("a-2", "alice", 2) });
    }

    #[test]
    fn presence_never_enters_the_record_log() {
        let mut host = host();
        hello(&mut host, "alice");
        hello(&mut host, "bob");
        host.take_outgoing("alice");
        host.take_outgoing("bob");

        host.handle_client_message("alice", ClientMessage::Presence { cursor: Some([3, 4, 0]) })
            .unwrap();
        assert!(host.records().is_empty());
        assert_eq!(host.cursor("alice"), Some([3, 4, 0]));
        assert_eq!(
            host.take_outgoing("bob"),
            vec![HostMessage::Presence { user_id: "alice".into(), cursor: Some([3, 4, 0]) }]
        );
    }

    #[test]
    fn protocol_mismatch_duplicate_id_and_unjoined_are_denied() {
        let mut host = host();
        assert_eq!(
            host.handle_client_message(
                "alice",
                ClientMessage::Hello { user: user("alice"), protocol_version: 0 }
            ),
            Err(ClientRejection::ProtocolVersion)
        );
        hello(&mut host, "alice");
        assert_eq!(
            host.handle_client_message(
                "alice",
                ClientMessage::Hello { user: user("alice"), protocol_version: SESSION_PROTOCOL_VERSION }
            ),
            Err(ClientRejection::DuplicateUserId)
        );
        assert_eq!(
            host.handle_client_message("ghost", ClientMessage::Ping),
            Err(ClientRejection::NotJoined)
        );
        assert_eq!(
            host.handle_client_message("ghost", ClientMessage::Action { record: stroke("g-1", "ghost", 0) }),
            Err(ClientRejection::NotJoined)
        );
    }

    #[test]
    fn mismatched_identity_in_hello_is_denied() {
        let mut host = host();
        assert_eq!(
            host.handle_client_message(
                "alice",
                ClientMessage::Hello { user: user("bob"), protocol_version: SESSION_PROTOCOL_VERSION }
            ),
            Err(ClientRejection::NotJoined)
        );
    }

    #[test]
    fn missing_snapshot_source_denies_join() {
        let mut host = SessionHost::new();
        assert_eq!(
            host.handle_client_message(
                "alice",
                ClientMessage::Hello { user: user("alice"), protocol_version: SESSION_PROTOCOL_VERSION }
            ),
            Err(ClientRejection::SnapshotUnavailable)
        );
    }

    #[test]
    fn disconnect_shrinks_roster_and_keeps_the_log() {
        let mut host = host();
        hello(&mut host, "alice");
        hello(&mut host, "bob");
        host.take_outgoing("alice");
        host.take_outgoing("bob");
        host.handle_client_message("bob", ClientMessage::Action { record: stroke("b-1", "bob", 1) })
            .unwrap();
        assert_eq!(host.take_outgoing("alice").len(), 1); // the Record

        host.disconnect("bob");
        assert!(!host.is_connected("bob"));
        assert_eq!(host.cursor("bob"), None);
        assert_eq!(host.records().len(), 1);
        assert_eq!(
            host.take_outgoing("alice"),
            vec![HostMessage::Roster { users: host.roster() }]
        );
    }

    #[test]
    fn local_host_record_enters_log_and_reaches_every_client() {
        let mut host = host();
        hello(&mut host, "alice");
        hello(&mut host, "bob");
        host.take_outgoing("alice");
        host.take_outgoing("bob");

        host.apply_local_record(stroke("host-1", "host-user", 3));
        assert_eq!(host.records().len(), 1);
        assert_eq!(host.records()[0].action_id, "host-1");
        assert_eq!(
            host.take_outgoing("alice"),
            vec![HostMessage::Record { record: stroke("host-1", "host-user", 3) }]
        );
        assert_eq!(
            host.take_outgoing("bob"),
            vec![HostMessage::Record { record: stroke("host-1", "host-user", 3) }]
        );
    }

    #[test]
    fn ping_answers_pong() {
        let mut host = host();
        hello(&mut host, "alice");
        host.take_outgoing("alice");
        host.handle_client_message("alice", ClientMessage::Ping).unwrap();
        assert_eq!(host.take_outgoing("alice"), vec![HostMessage::Pong]);
    }

    #[test]
    fn protocol_round_trips_through_json() {
        let message = ClientMessage::Hello { user: user("alice"), protocol_version: SESSION_PROTOCOL_VERSION };
        let text = serde_json::to_string(&message).unwrap();
        assert_eq!(serde_json::from_str::<ClientMessage>(&text).unwrap(), message);

        let record = stroke("a-1", "alice", 5);
        let message = HostMessage::Record { record: record.clone() };
        let text = serde_json::to_string(&message).unwrap();
        assert_eq!(serde_json::from_str::<HostMessage>(&text).unwrap(), message);
    }
}

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
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use thaum_painter_domain::storage::{SharedDocumentActionRecord, SharedDocumentFile};

/// Bumped on any wire-shape change; a `Hello` with a mismatched version is
/// denied so old clients fail loudly instead of corrupting sessions. v3:
/// `StructureSet` records join the log — a v2 peer would fail to deserialize
/// them (unknown action variant) and silently lose every structure edit.
pub const SESSION_PROTOCOL_VERSION: u32 = 3;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionUser {
    pub user_id: String,
    pub display_name: String,
    pub presence_color: [u8; 3],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ClientMessage {
    Hello {
        user: SessionUser,
        protocol_version: u32,
    },
    /// Any `SharedDocumentActionRecord`. Opaque to the host: it is appended at
    /// arrival order and broadcast verbatim.
    Action {
        record: SharedDocumentActionRecord,
    },
    /// Presence rides this channel, never the record log. Cursor is in cell
    /// space, matching `CellPoint` (x, y, z).
    Presence {
        cursor: Option<[i32; 3]>,
    },
    /// A joined client renames itself: roster truth updates on the host, and
    /// every client (the renamer included) learns it via a roster broadcast.
    Rename {
        display_name: String,
    },
    Ping,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum HostMessage {
    /// Fresh copy of the document + how many records predate the joiner. The
    /// joiner resets its runtime from the snapshot, then applies live records.
    Welcome {
        snapshot: SharedDocumentFile,
        log_length: u64,
        roster: Vec<SessionUser>,
    },
    Record {
        record: SharedDocumentActionRecord,
    },
    Presence {
        user_id: String,
        cursor: Option<[i32; 3]>,
    },
    Roster {
        users: Vec<SessionUser>,
    },
    Denied {
        reason: String,
    },
    /// The host ended the session on purpose. Clients render that honestly
    /// and drop to offline — no silent death, no rejoin attempt.
    Ended,
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
    /// The host already ended the session; no further joins.
    SessionEnded,
}

pub struct HostClient {
    pub user: SessionUser,
    pub outgoing: VecDeque<HostMessage>,
}

/// Transport-agnostic session host core. One instance per hosted document.
pub struct SessionHost {
    /// The hosting app's own identity. Always the first roster entry, so
    /// clients can crown the host and address it; `None` only for a bare
    /// core that no hosting app has claimed yet.
    host_user: Option<SessionUser>,
    /// The authoritative sync log in arrival order. The host's owning app
    /// persists this to `actions.jsonl` (host owns saves in session mode).
    records: Vec<SharedDocumentActionRecord>,
    clients: Vec<HostClient>,
    cursors: HashMap<String, Option<[i32; 3]>>,
    /// Last time each connected client sent anything (keepalive included).
    /// The transport prunes clients silent past the seen timeout, so a
    /// half-open dead connection cannot squat its user_id forever — that
    /// denial loop is what left the chip stuck on RECONNECTING.
    last_seen: HashMap<String, Instant>,
    snapshot_source: Option<Box<dyn Fn() -> SharedDocumentFile + Send>>,
    ended: bool,
}

impl Default for SessionHost {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionHost {
    pub fn new() -> Self {
        Self {
            host_user: None,
            records: Vec::new(),
            clients: Vec::new(),
            cursors: HashMap::new(),
            last_seen: HashMap::new(),
            snapshot_source: None,
            ended: false,
        }
    }

    /// Claims the host identity: the owning app's user becomes roster entry
    /// zero and joins/duplicates are checked against it.
    pub fn set_host_user(&mut self, user: SessionUser) {
        self.host_user = Some(user);
    }

    /// The owning painter app supplies the current document at join time; the
    /// host itself never touches document semantics.
    pub fn set_snapshot_source(&mut self, source: Box<dyn Fn() -> SharedDocumentFile + Send>) {
        self.snapshot_source = Some(source);
    }

    pub fn records(&self) -> &[SharedDocumentActionRecord] {
        &self.records
    }

    /// Seeds the log with the hosting app's pre-host action history (the
    /// serialized truth: squash baselines + unfolded records). The Welcome
    /// snapshot is structure-only — canvas content rebuilds purely through
    /// log replay — so without the seed, joiners would see layers but blank
    /// cells for everything painted before hosting started.
    pub fn seed_log(&mut self, records: Vec<SharedDocumentActionRecord>) {
        self.records = records;
    }

    pub fn roster(&self) -> Vec<SessionUser> {
        let mut users = Vec::with_capacity(self.clients.len() + 1);
        if let Some(host_user) = &self.host_user {
            users.push(host_user.clone());
        }
        users.extend(self.clients.iter().map(|client| client.user.clone()));
        users
    }

    pub fn host_user_id(&self) -> Option<&str> {
        self.host_user.as_ref().map(|user| user.user_id.as_str())
    }

    /// The host app renames itself: roster truth updates and every client
    /// learns the new name through a roster broadcast.
    pub fn set_host_display_name(&mut self, display_name: &str) {
        let Some(host_user) = &mut self.host_user else {
            return;
        };
        host_user.display_name = display_name.to_string();
        let roster = self.roster();
        for client in &mut self.clients {
            client.outgoing.push_back(HostMessage::Roster {
                users: roster.clone(),
            });
        }
    }

    pub fn is_connected(&self, user_id: &str) -> bool {
        self.clients
            .iter()
            .any(|client| client.user.user_id == user_id)
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
        // Every non-Hello message counts as a liveness signal from a joined
        // identity (Hello seeds its own entry below).
        if !matches!(message, ClientMessage::Hello { .. }) {
            self.last_seen
                .insert(user_id.to_string(), Instant::now());
        }
        match message {
            ClientMessage::Hello {
                user,
                protocol_version,
            } => {
                if self.ended {
                    return Err(ClientRejection::SessionEnded);
                }
                if protocol_version != SESSION_PROTOCOL_VERSION {
                    return Err(ClientRejection::ProtocolVersion);
                }
                if user.user_id != user_id {
                    return Err(ClientRejection::NotJoined);
                }
                if self
                    .host_user
                    .as_ref()
                    .is_some_and(|host_user| host_user.user_id == user_id)
                    || self.is_connected(user_id)
                {
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
                self.last_seen.insert(user.user_id.clone(), Instant::now());
                self.queue_to(
                    &user.user_id,
                    HostMessage::Welcome {
                        snapshot,
                        log_length,
                        roster: self.roster(),
                    },
                );
                // Full-log replay: the joiner's snapshot is the document as of
                // host-log start, so it applies every historical record from
                // cursor 0 (Figma's fresh-copy-plus-replay convergence).
                let history: Vec<SharedDocumentActionRecord> = self.records.clone();
                for record in history {
                    self.queue_to(&user.user_id, HostMessage::Record { record });
                }
                self.broadcast_others(
                    &user.user_id,
                    HostMessage::Roster {
                        users: self.roster(),
                    },
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
                    HostMessage::Presence {
                        user_id: user_id.to_string(),
                        cursor,
                    },
                );
                Ok(())
            }
            ClientMessage::Rename { display_name } => {
                let Some(client) = self
                    .clients
                    .iter_mut()
                    .find(|client| client.user.user_id == user_id)
                else {
                    return Err(ClientRejection::NotJoined);
                };
                client.user.display_name = display_name;
                let roster = self.roster();
                for client in &mut self.clients {
                    client.outgoing.push_back(HostMessage::Roster {
                        users: roster.clone(),
                    });
                }
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
        self.last_seen.remove(user_id);
        self.broadcast_others(
            user_id,
            HostMessage::Roster {
                users: self.roster(),
            },
        );
    }

    /// Prunes clients that have sent nothing past `timeout` (keepalive pings
    /// every couple of seconds keep live clients comfortably inside it). A
    /// half-open dead connection otherwise holds its user_id forever, and
    /// every honest rejoin of that identity is denied as a duplicate. Pruned
    /// clients go through the normal teardown — roster shrink broadcast
    /// included — so survivors and rejoins both see the honest state.
    pub fn prune_stale_clients(&mut self, timeout: Duration) {
        let now = Instant::now();
        let stale: Vec<String> = self
            .clients
            .iter()
            .map(|client| client.user.user_id.clone())
            .filter(|id| {
                now.duration_since(self.last_seen.get(id).copied().unwrap_or(now)) >= timeout
            })
            .collect();
        for user_id in stale {
            self.disconnect(&user_id);
        }
    }

    /// The host ends the session on purpose: every connected client learns
    /// `Ended` (rendered as "host ended the session"), and further joins are
    /// denied. The log is untouched.
    pub fn end_session(&mut self) {
        self.ended = true;
        for client in &mut self.clients {
            client.outgoing.push_back(HostMessage::Ended);
        }
    }

    pub fn is_ended(&self) -> bool {
        self.ended
    }

    /// The host user's own edits: the local app already applied the record
    /// to its runtime, so this only enters the log and broadcasts to every
    /// connected client (arrival order keeps the log totally ordered).
    pub fn apply_local_record(&mut self, record: SharedDocumentActionRecord) {
        self.records.push(record.clone());
        for client in &mut self.clients {
            client.outgoing.push_back(HostMessage::Record {
                record: record.clone(),
            });
        }
    }

    /// Drains one client's queued messages (the transport's read side).
    pub fn take_outgoing(&mut self, user_id: &str) -> Vec<HostMessage> {
        match self
            .clients
            .iter_mut()
            .find(|client| client.user.user_id == user_id)
        {
            Some(client) => client.outgoing.drain(..).collect(),
            None => Vec::new(),
        }
    }

    fn queue_to(&mut self, user_id: &str, message: HostMessage) {
        if let Some(client) = self
            .clients
            .iter_mut()
            .find(|client| client.user.user_id == user_id)
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
        host.handle_client_message(
            id,
            ClientMessage::Hello {
                user: user(id),
                protocol_version: SESSION_PROTOCOL_VERSION,
            },
        )
        .expect("hello accepted");
    }

    fn stroke(action_id: &str, user_id: &str, x: i32) -> SharedDocumentActionRecord {
        SharedDocumentActionRecord::cell_patch_set(
            action_id.to_string(),
            "doc-1".to_string(),
            "layer-1".to_string(),
            user_id.to_string(),
            "t".to_string(),
            vec![SharedCellPatch::new(
                CellPoint { x, y: 0, z: 0 },
                None,
                None,
            )],
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
            HostMessage::Welcome {
                snapshot,
                log_length,
                roster,
            } => {
                assert_eq!(snapshot.document_id, snapshot_document().document_id);
                assert_eq!(*log_length, 0);
                assert_eq!(roster.len(), 2);
            }
            other => panic!("expected welcome, got {other:?}"),
        }

        let alice_out = host.take_outgoing("alice");
        assert_eq!(
            alice_out,
            vec![HostMessage::Roster {
                users: host.roster()
            }]
        );
    }

    #[test]
    fn actions_log_in_arrival_order_and_broadcast_to_others_only() {
        let mut host = host();
        hello(&mut host, "alice");
        hello(&mut host, "bob");
        host.take_outgoing("alice");
        host.take_outgoing("bob");

        host.handle_client_message(
            "alice",
            ClientMessage::Action {
                record: stroke("a-1", "alice", 0),
            },
        )
        .unwrap();
        host.handle_client_message(
            "bob",
            ClientMessage::Action {
                record: stroke("b-1", "bob", 1),
            },
        )
        .unwrap();
        host.handle_client_message(
            "alice",
            ClientMessage::Action {
                record: stroke("a-2", "alice", 2),
            },
        )
        .unwrap();

        assert_eq!(host.records().len(), 3);
        assert_eq!(host.records()[0].action_id, "a-1");
        assert_eq!(host.records()[1].action_id, "b-1");
        assert_eq!(host.records()[2].action_id, "a-2");

        let alice_out = host.take_outgoing("alice");
        assert_eq!(
            alice_out,
            vec![HostMessage::Record {
                record: stroke("b-1", "bob", 1)
            }]
        );
        let bob_out = host.take_outgoing("bob");
        assert_eq!(bob_out.len(), 2);
        assert_eq!(
            bob_out[0],
            HostMessage::Record {
                record: stroke("a-1", "alice", 0)
            }
        );
        assert_eq!(
            bob_out[1],
            HostMessage::Record {
                record: stroke("a-2", "alice", 2)
            }
        );
    }

    #[test]
    fn presence_never_enters_the_record_log() {
        let mut host = host();
        hello(&mut host, "alice");
        hello(&mut host, "bob");
        host.take_outgoing("alice");
        host.take_outgoing("bob");

        host.handle_client_message(
            "alice",
            ClientMessage::Presence {
                cursor: Some([3, 4, 0]),
            },
        )
        .unwrap();
        assert!(host.records().is_empty());
        assert_eq!(host.cursor("alice"), Some([3, 4, 0]));
        assert_eq!(
            host.take_outgoing("bob"),
            vec![HostMessage::Presence {
                user_id: "alice".into(),
                cursor: Some([3, 4, 0])
            }]
        );
    }

    #[test]
    fn protocol_mismatch_duplicate_id_and_unjoined_are_denied() {
        let mut host = host();
        assert_eq!(
            host.handle_client_message(
                "alice",
                ClientMessage::Hello {
                    user: user("alice"),
                    protocol_version: 0
                }
            ),
            Err(ClientRejection::ProtocolVersion)
        );
        hello(&mut host, "alice");
        assert_eq!(
            host.handle_client_message(
                "alice",
                ClientMessage::Hello {
                    user: user("alice"),
                    protocol_version: SESSION_PROTOCOL_VERSION
                }
            ),
            Err(ClientRejection::DuplicateUserId)
        );
        assert_eq!(
            host.handle_client_message("ghost", ClientMessage::Ping),
            Err(ClientRejection::NotJoined)
        );
        assert_eq!(
            host.handle_client_message(
                "ghost",
                ClientMessage::Action {
                    record: stroke("g-1", "ghost", 0)
                }
            ),
            Err(ClientRejection::NotJoined)
        );
    }

    #[test]
    fn mismatched_identity_in_hello_is_denied() {
        let mut host = host();
        assert_eq!(
            host.handle_client_message(
                "alice",
                ClientMessage::Hello {
                    user: user("bob"),
                    protocol_version: SESSION_PROTOCOL_VERSION
                }
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
                ClientMessage::Hello {
                    user: user("alice"),
                    protocol_version: SESSION_PROTOCOL_VERSION
                }
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
        host.handle_client_message(
            "bob",
            ClientMessage::Action {
                record: stroke("b-1", "bob", 1),
            },
        )
        .unwrap();
        assert_eq!(host.take_outgoing("alice").len(), 1); // the Record

        host.disconnect("bob");
        assert!(!host.is_connected("bob"));
        assert_eq!(host.cursor("bob"), None);
        assert_eq!(host.records().len(), 1);
        assert_eq!(
            host.take_outgoing("alice"),
            vec![HostMessage::Roster {
                users: host.roster()
            }]
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
            vec![HostMessage::Record {
                record: stroke("host-1", "host-user", 3)
            }]
        );
        assert_eq!(
            host.take_outgoing("bob"),
            vec![HostMessage::Record {
                record: stroke("host-1", "host-user", 3)
            }]
        );
    }

    #[test]
    fn ping_answers_pong() {
        let mut host = host();
        hello(&mut host, "alice");
        host.take_outgoing("alice");
        host.handle_client_message("alice", ClientMessage::Ping)
            .unwrap();
        assert_eq!(host.take_outgoing("alice"), vec![HostMessage::Pong]);
    }

    #[test]
    fn end_session_broadcasts_ended_and_denies_new_joins() {
        let mut host = host();
        hello(&mut host, "alice");
        hello(&mut host, "bob");
        host.take_outgoing("alice");
        host.take_outgoing("bob");

        host.end_session();
        assert!(host.is_ended());
        assert_eq!(host.take_outgoing("alice"), vec![HostMessage::Ended]);
        assert_eq!(host.take_outgoing("bob"), vec![HostMessage::Ended]);

        assert_eq!(
            host.handle_client_message(
                "carol",
                ClientMessage::Hello {
                    user: user("carol"),
                    protocol_version: SESSION_PROTOCOL_VERSION,
                }
            ),
            Err(ClientRejection::SessionEnded)
        );
    }

    #[test]
    fn stale_clients_are_pruned_and_their_identity_frees_up() {
        let mut host = host();
        hello(&mut host, "alice");
        assert!(host.is_connected("alice"));

        // Silent past the seen timeout: pruned through normal teardown.
        std::thread::sleep(Duration::from_millis(30));
        host.prune_stale_clients(Duration::from_millis(20));
        assert!(!host.is_connected("alice"));
        assert_eq!(host.cursor("alice"), None);

        // The freed identity rejoins honestly instead of being denied as a
        // duplicate forever (the stuck-RECONNECTING bug).
        hello(&mut host, "alice");
        assert!(host.is_connected("alice"));
    }

    #[test]
    fn host_user_leads_the_roster_and_duplicates_are_denied() {
        let mut host = host();
        host.set_host_user(user("host-app"));
        hello(&mut host, "alice");

        let roster = host.roster();
        assert_eq!(roster.len(), 2);
        assert_eq!(roster[0].user_id, "host-app");
        assert_eq!(host.host_user_id(), Some("host-app"));

        // The host's own identity is taken: a client cannot steal it.
        assert_eq!(
            host.handle_client_message(
                "host-app",
                ClientMessage::Hello {
                    user: user("host-app"),
                    protocol_version: SESSION_PROTOCOL_VERSION,
                }
            ),
            Err(ClientRejection::DuplicateUserId)
        );
    }

    #[test]
    fn rename_updates_roster_truth_and_reaches_every_client() {
        let mut host = host();
        hello(&mut host, "alice");
        host.take_outgoing("alice");
        hello(&mut host, "bob");
        host.take_outgoing("alice");
        host.take_outgoing("bob");

        host.handle_client_message(
            "alice",
            ClientMessage::Rename {
                display_name: "Alice A".to_string(),
            },
        )
        .unwrap();

        let roster = host.roster();
        assert_eq!(roster[0].display_name, "Alice A");
        // The renamer included: both learn the same roster truth.
        for id in ["alice", "bob"] {
            assert_eq!(
                host.take_outgoing(id),
                vec![HostMessage::Roster {
                    users: roster.clone()
                }]
            );
        }

        // Host-side rename flows the same way.
        host.set_host_user(user("host-app"));
        host.set_host_display_name("The Host");
        assert_eq!(host.roster()[0].display_name, "The Host");
    }

    #[test]
    fn protocol_round_trips_through_json() {
        let message = ClientMessage::Hello {
            user: user("alice"),
            protocol_version: SESSION_PROTOCOL_VERSION,
        };
        let text = serde_json::to_string(&message).unwrap();
        assert_eq!(
            serde_json::from_str::<ClientMessage>(&text).unwrap(),
            message
        );

        let record = stroke("a-1", "alice", 5);
        let message = HostMessage::Record {
            record: record.clone(),
        };
        let text = serde_json::to_string(&message).unwrap();
        assert_eq!(serde_json::from_str::<HostMessage>(&text).unwrap(), message);
    }
}

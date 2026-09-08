//! Session net: the entrypoint-facing seam over the session host/client pair.
//!
//! The owning painter app holds one `Option<SessionNet>` and treats hosting
//! and joining identically: `publish(record)` for records it just applied
//! locally, `sync(&mut runtime)` to pull everyone else's records in host
//! order. Hosting mode wraps a `SessionHost` + TCP server; joining mode wraps
//! a `SessionClient`. Neither mode owns the runtime — the app's live runtime
//! stays the single source of truth it applies to.
//!
//! Convergence model (Figma's): the host's snapshot source must capture the
//! document as it was when the server started (the log starts empty then);
//! joiners rebuild by fresh snapshot + full-log replay. Structure edits that
//! still bypass the record log (`document.json` path) are not replayed — that
//! gap closes with the structure-edit record variants in `domain/file/storage/`.

use std::sync::{Arc, Mutex};

use crate::session_client::{SessionClient, SessionClientError};
use crate::session_host::{SessionHost, SessionUser};
use crate::session_host_tcp::{spawn_session_host_server, SessionHostServer};
use thaum_painter_domain::storage::{
    SharedDocumentActionRecord, SharedDocumentFile, SharedDocumentRuntime,
};

pub enum SessionNet {
    Host {
        host: Arc<Mutex<SessionHost>>,
        /// Kept for shutdown/drop; the accept loop ends on drop.
        _server: SessionHostServer,
        user_id: String,
        /// How many host-log records the local app has consumed into its own
        /// runtime. Own records are skipped (applied at publish); foreign
        /// records (from clients) are applied here.
        consumed: usize,
    },
    Client {
        client: SessionClient,
    },
}

impl SessionNet {
    /// Starts hosting: binds the TCP server and registers the snapshot
    /// source. The source should return the document as of server start.
    pub fn host(
        snapshot_source: Box<dyn Fn() -> SharedDocumentFile + Send>,
        user: SessionUser,
        port: u16,
    ) -> std::io::Result<Self> {
        let mut host = SessionHost::new();
        host.set_snapshot_source(snapshot_source);
        let host = Arc::new(Mutex::new(host));
        let server = spawn_session_host_server(Arc::clone(&host), port)?;
        Ok(Self::Host {
            host,
            _server: server,
            user_id: user.user_id,
            consumed: 0,
        })
    }

    /// Joins a host. Returns the net seam plus the host's snapshot document —
    /// the caller rebuilds its runtime from it before the first sync.
    pub fn join(
        address: &str,
        user: SessionUser,
    ) -> Result<(Self, SharedDocumentFile), SessionClientError> {
        let (client, snapshot) = SessionClient::connect(address, user)?;
        Ok((Self::Client { client }, snapshot))
    }

    pub fn user_id(&self) -> &str {
        match self {
            Self::Host { user_id, .. } => user_id,
            Self::Client { client } => &client.user_id,
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
            Self::Client { client } => client
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
            Self::Client { client } => client.sync(runtime).map_err(|error| error.to_string()),
        }
    }

    pub fn roster(&self) -> Vec<SessionUser> {
        match self {
            Self::Host { host, .. } => host.lock().expect("session host lock").roster(),
            Self::Client { client } => client.roster().to_vec(),
        }
    }

    pub fn cursor(&self, user_id: &str) -> Option<[i32; 3]> {
        match self {
            Self::Host { .. } => None, // host presence UI rides the client side
            Self::Client { client } => client.cursor(user_id),
        }
    }

    /// Connected flag (client side); the host is "connected" while its server
    /// accepts. Used by the frame loop to stop publishing after a loss.
    pub fn is_connected(&self) -> bool {
        match self {
            Self::Host { .. } => true,
            Self::Client { client } => client.is_connected(),
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
}

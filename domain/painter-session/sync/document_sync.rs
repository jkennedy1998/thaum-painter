//! Loopback document sync: independent sessions converge by exchanging the
//! shared document's action records through one totally-ordered log.
//!
//! This is the multiplayer proof seam. The future session host ships exactly
//! these `SharedDocumentActionRecord`s over a wire; nothing in this seam is UI
//! and nothing talks to disk. A session applies foreign records in log order
//! and skips its own (already applied locally at publish time), so any number
//! of sessions converge on the same document state. Structure edits (layers,
//! document shape) stay on the `document.json` revision path — the loopback
//! carries content edits (cell patches) and undo/redo history records only.

use crate::storage::{SharedDocumentActionRecord, SharedDocumentRuntime};

/// One totally-ordered record log shared by every synced session — the
/// loopback stand-in for the future multiplayer transport. Order of the log
/// is the order all sessions apply; that shared order is what convergence
/// rests on.
#[derive(Debug, Default)]
pub struct LoopbackActionBus {
    records: Vec<SharedDocumentActionRecord>,
}

impl LoopbackActionBus {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append one record to the shared log.
    pub fn publish(&mut self, record: SharedDocumentActionRecord) {
        self.records.push(record);
    }

    pub fn records(&self) -> &[SharedDocumentActionRecord] {
        &self.records
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

/// One user's editing session over its own document runtime, syncing through
/// the shared log. `action_counter` mints per-session action ids the same way
/// the live entrypoint does; `consumed_count` is this session's read cursor
/// into the shared log.
#[derive(Debug)]
pub struct SyncedDocumentSession {
    pub user_id: String,
    pub runtime: SharedDocumentRuntime,
    pub action_counter: u64,
    consumed_count: usize,
}

impl SyncedDocumentSession {
    pub fn new(user_id: impl Into<String>, runtime: SharedDocumentRuntime) -> Self {
        Self {
            user_id: user_id.into(),
            runtime,
            action_counter: 0,
            consumed_count: 0,
        }
    }

    /// Publish a forward-edit record: apply it locally first (the canonical
    /// append-then-apply flow, minus the disk append), then append it to the
    /// shared log.
    pub fn publish_action(
        &mut self,
        bus: &mut LoopbackActionBus,
        record: SharedDocumentActionRecord,
    ) {
        self.runtime.apply_action_record(record.clone());
        bus.publish(record);
    }

    /// Publish a history (undo/redo revert) record: the local runtime already
    /// mutated its canvases via `undo_top_action`/`redo_top_action`, so the
    /// record is pushed onto the history stacks without re-applying, exactly
    /// like the live `apply_shared_history_action` flow.
    pub fn publish_history_record(
        &mut self,
        bus: &mut LoopbackActionBus,
        record: SharedDocumentActionRecord,
    ) {
        self.runtime.push_history_record(record.clone());
        bus.publish(record);
    }

    /// Pull every log record past this session's cursor. Foreign records are
    /// applied in log order; own records are skipped (they were applied at
    /// publish time and re-applying would double their patches and stack
    /// entries). Returns how many foreign records were applied.
    pub fn sync_from_bus(&mut self, bus: &LoopbackActionBus) -> usize {
        let mut applied = 0;
        while self.consumed_count < bus.records().len() {
            let record = &bus.records()[self.consumed_count];
            self.consumed_count += 1;
            if record.user_id == self.user_id {
                continue;
            }
            self.runtime.apply_action_record(record.clone());
            applied += 1;
        }
        applied
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brush::{Canvas, PaintedCell};
    use crate::paint_color::PaintColor;
    use crate::session_document::{action_timestamp_string, next_action_id};
    use crate::storage::{SharedCellPatch, SharedDocumentFile};
    use thaum_renderer_domain::{CellGraphic, CellPoint};

    fn single_layer_document() -> SharedDocumentRuntime {
        let document = SharedDocumentFile::single_layer("doc-1", "Doc", "layer-1", "Layer 1");
        SharedDocumentRuntime::new(document)
    }

    fn session(user_id: &str) -> SyncedDocumentSession {
        SyncedDocumentSession::new(user_id, single_layer_document())
    }

    fn paint(color: (u8, u8, u8)) -> PaintedCell {
        PaintedCell {
            graphic: CellGraphic::Glyph('a'),
            color: PaintColor::FlatRgb(color.0, color.1, color.2),
            weight_index: 2,
        }
    }

    fn stroke_record(
        session: &mut SyncedDocumentSession,
        position: CellPoint,
        color: (u8, u8, u8),
    ) -> SharedDocumentActionRecord {
        SharedDocumentActionRecord::cell_patch_set(
            next_action_id(&mut session.action_counter, &session.user_id),
            session.runtime.document.document_id.clone(),
            "layer-1".to_string(),
            session.user_id.clone(),
            action_timestamp_string(),
            vec![SharedCellPatch::new(position, None, Some(&paint(color)))],
            None,
        )
    }

    fn canvas_for(session: &SyncedDocumentSession) -> Canvas {
        session
            .runtime
            .canvas_for_layer("layer-1", 0)
            .cloned()
            .unwrap_or_default()
    }

    fn assert_sessions_converged(sessions: &[&SyncedDocumentSession]) {
        let first = canvas_for(sessions[0]);
        for session in &sessions[1..] {
            assert_eq!(
                canvas_for(session),
                first,
                "session {} diverged from {}",
                session.user_id,
                sessions[0].user_id
            );
        }
    }

    #[test]
    fn two_sessions_converge_on_interleaved_strokes() {
        let mut bus = LoopbackActionBus::new();
        let mut alice = session("alice");
        let mut bob = session("bob");

        let record = stroke_record(&mut alice, CellPoint { x: 0, y: 0, z: 0 }, (255, 0, 0));
        alice.publish_action(&mut bus, record);
        bob.sync_from_bus(&bus);
        let record = stroke_record(&mut bob, CellPoint { x: 1, y: 0, z: 0 }, (0, 255, 0));
        bob.publish_action(&mut bus, record);
        alice.sync_from_bus(&bus);
        // A second stroke from alice after bob's lands on top of the log.
        let record = stroke_record(&mut alice, CellPoint { x: 2, y: 0, z: 0 }, (0, 0, 255));
        alice.publish_action(&mut bus, record);
        bob.sync_from_bus(&bus);

        assert_sessions_converged(&[&alice, &bob]);
        let canvas = canvas_for(&alice);
        assert_eq!(canvas.len(), 3);
        assert_eq!(
            canvas.get(&CellPoint { x: 1, y: 0, z: 0 }),
            Some(&paint((0, 255, 0)))
        );
    }

    #[test]
    fn a_session_skips_its_own_records_on_sync() {
        let mut bus = LoopbackActionBus::new();
        let mut alice = session("alice");

        let record = stroke_record(&mut alice, CellPoint { x: 0, y: 0, z: 0 }, (255, 0, 0));
        alice.publish_action(&mut bus, record);
        // Re-syncing sees its own record but must not re-apply it.
        assert_eq!(alice.sync_from_bus(&bus), 0);
        assert_eq!(canvas_for(&alice).len(), 1);
    }

    #[test]
    fn sessions_that_sync_late_catch_up_on_the_full_log() {
        let mut bus = LoopbackActionBus::new();
        let mut alice = session("alice");
        let mut carol = session("carol");

        for x in 0..3i32 {
            let record = stroke_record(
                &mut alice,
                CellPoint { x, y: 0, z: 0 },
                (10 * x as u8 + 1, 0, 0),
            );
            alice.publish_action(&mut bus, record);
        }
        // Carol joins after three strokes already happened.
        assert_eq!(carol.sync_from_bus(&bus), 3);
        assert_sessions_converged(&[&alice, &carol]);
    }

    #[test]
    fn undo_as_revert_record_converges_across_sessions() {
        let mut bus = LoopbackActionBus::new();
        let mut alice = session("alice");
        let mut bob = session("bob");

        let record = stroke_record(&mut alice, CellPoint { x: 0, y: 0, z: 0 }, (255, 0, 0));
        alice.publish_action(&mut bus, record);
        bob.sync_from_bus(&bus);
        assert_eq!(canvas_for(&bob).len(), 1);

        // Alice undoes her stroke: local revert mutation, then the passive
        // revert record goes out (the live apply_shared_history_action flow).
        let revert = alice
            .runtime
            .undo_top_action("layer-1")
            .expect("undo finds the stroke");
        let record = SharedDocumentActionRecord::revert_patch_set(
            next_action_id(&mut alice.action_counter, &alice.user_id),
            alice.runtime.document.document_id.clone(),
            "layer-1".to_string(),
            alice.user_id.clone(),
            action_timestamp_string(),
            revert.patches,
            Some(revert.block_id),
            revert.action_id,
        );
        alice.publish_history_record(&mut bus, record);
        assert_eq!(canvas_for(&alice).len(), 0);

        // Bob applies the revert record and converges on the emptied canvas.
        assert_eq!(bob.sync_from_bus(&bus), 1);
        assert_sessions_converged(&[&alice, &bob]);
        assert_eq!(canvas_for(&bob).len(), 0);
    }

    #[test]
    fn three_sessions_converge_on_a_shared_log() {
        let mut bus = LoopbackActionBus::new();
        let mut alice = session("alice");
        let mut bob = session("bob");
        let mut carol = session("carol");

        let record = stroke_record(&mut alice, CellPoint { x: 0, y: 0, z: 0 }, (1, 1, 1));
        alice.publish_action(&mut bus, record);
        let record = stroke_record(&mut bob, CellPoint { x: 1, y: 0, z: 0 }, (2, 2, 2));
        bob.publish_action(&mut bus, record);
        let record = stroke_record(&mut carol, CellPoint { x: 2, y: 0, z: 0 }, (3, 3, 3));
        carol.publish_action(&mut bus, record);
        alice.sync_from_bus(&bus);
        bob.sync_from_bus(&bus);
        carol.sync_from_bus(&bus);

        assert_sessions_converged(&[&alice, &bob, &carol]);
        assert_eq!(canvas_for(&alice).len(), 3);
    }
}

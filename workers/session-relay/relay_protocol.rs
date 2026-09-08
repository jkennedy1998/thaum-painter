//! Relay wire protocol: role-claim hello, routed frames, error/closure frames.
//!
//! Every byte on the relay wire is a length-prefixed NDJSON frame (4-byte
//! big-endian length + JSON). The relay NEVER parses session content: inner
//! `line` strings are the existing `ClientMessage`/`HostMessage` JSON passed
//! through verbatim, so the session cores cannot tell the relay lane from the
//! LAN wire.
//!
//! Frame flow (all JSON, all length-prefixed):
//! - first frame client→relay (role claim):
//!   `{"type":"host"}` — claim/mint a room
//!   `{"type":"join","room":...,"token":...,"user":...}` — join as joiner
//! - first frame relay→client:
//!   `{"type":"ok","room":...,"token":...}` (host gets its minted code halves)
//!   `{"type":"error","reason":...}` — then the connection closes
//! - data joiner→relay: `{"line":...}` — routed to the room host
//! - data host→relay:  `{"to":"<user_id|all>","line":...}` — routed to that
//!   joiner or every joiner
//! - data relay→host:  `{"from":"<user_id>","line":...}` (wrapped: the host
//!   core needs the peer identity to route the message)
//! - data relay→joiner: the inner line UNWRAPPED — the joiner-side session
//!   core parses it exactly like the LAN wire (all session truth arrives from
//!   the host, so no peer identity is needed)
//! - closure relay→joiner: `room-closed` — the host link died; room gone
//! - closure relay→host: `member-left` — a joiner's link died; the host core
//!   disconnects them like a LAN socket EOF

use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

/// Hard cap on one frame's JSON bytes. Session records ride inside `line`
/// strings; a stroke record is normally a few KB — 1 MiB is generous headroom
/// while still bounding any hostile peer's allocation.
pub const DEFAULT_MAX_FRAME_BYTES: usize = 1024 * 1024;

/// 4-byte big-endian length prefix, then the JSON bytes.
pub const LENGTH_PREFIX_BYTES: usize = 4;

/// Incremental frame extraction for a shared read/write stream: the server's
/// reader releases its per-connection lock between short read slices, so a
/// frame may arrive split across many reads. `FrameBuffer` keeps the partial
/// bytes; `take_frame` pulls one complete frame when (and only when) one is
/// fully buffered. Oversized/empty frames are refused before any body bytes
/// are read, matching `read_frame`.
pub struct FrameBuffer {
    buf: Vec<u8>,
}

impl Default for FrameBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameBuffer {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Feeds freshly-read wire bytes into the buffer.
    pub fn extend(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// Extracts one complete frame from the buffer, if one has fully arrived.
    pub fn take_frame(&mut self, max_frame_bytes: usize) -> std::io::Result<Option<String>> {
        if self.buf.len() < LENGTH_PREFIX_BYTES {
            return Ok(None);
        }
        let length = u32::from_be_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]])
            as usize;
        if length == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "empty frame",
            ));
        }
        if length > max_frame_bytes {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("frame of {length} bytes exceeds the {max_frame_bytes} byte cap"),
            ));
        }
        if self.buf.len() < LENGTH_PREFIX_BYTES + length {
            return Ok(None);
        }
        let bytes: Vec<u8> = self
            .buf
            .drain(..LENGTH_PREFIX_BYTES + length)
            .skip(LENGTH_PREFIX_BYTES)
            .collect();
        String::from_utf8(bytes)
            .map(Some)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum HelloFrame {
    /// Claim/mint a room. The relay mints room id + token and returns them in
    /// the `ok` frame — the code IS the invite.
    Host,
    /// Join an existing room as a joiner. `user` is the session-side user id
    /// the room host uses as the routing address for this joiner.
    Join {
        room: String,
        token: String,
        user: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ServerFrame {
    /// Role claim accepted. For hosts, `room` + `token` are the freshly minted
    /// code halves; for joiners, `room` echoes the joined room.
    Ok { room: String, token: String },
    /// Denial — the connection closes right after.
    Error { reason: String },
    /// The room's host link died; the room is gone.
    RoomClosed,
    /// Server→host only: a joiner's relay link died. The host session core
    /// must disconnect them exactly like a LAN socket EOF — roster shrink
    /// broadcast, instant user-id freeing for the honest rejoin.
    MemberLeft { user: String },
}

/// Host→relay control: force-close the targeted joiner's relay link (or every
/// joiner's). The host session core has already evicted them (re-seed) — this
/// closes the wire so the joiner's auto-rejoin rebuilds from the new state,
/// mirroring how a LAN eviction closes the socket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum KickFromHost {
    /// Force-close the targeted joiner's relay link (`to` = session user id),
    /// or every joiner's (`to` = `"all"`). An enum so the tag value renames
    /// to `kick` — a struct's tag would serialize as the raw type name and
    /// collide with the data-lane tag checks.
    Kick { to: String },
}

/// A routed data frame. `to`/`from` carry the session-side user id (the host
/// is addressed by its joiners implicitly — joiner lines always go to the host).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataFromJoiner {
    pub line: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataFromHost {
    /// Target joiner's session user id, or `"all"`.
    pub to: String,
    pub line: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataToHost {
    pub from: String,
    pub line: String,
}

/// The reserved `to` target meaning "every joiner in the room".
pub const BROADCAST_TARGET: &str = "all";

/// Reads one length-prefixed frame, enforcing `max_frame_bytes` BEFORE
/// allocating. `Ok(None)` = clean EOF at a frame boundary.
pub fn read_frame(
    reader: &mut impl Read,
    max_frame_bytes: usize,
) -> std::io::Result<Option<String>> {
    let mut prefix = [0u8; LENGTH_PREFIX_BYTES];
    let mut read = 0;
    while read < LENGTH_PREFIX_BYTES {
        let n = reader.read(&mut prefix[read..])?;
        if n == 0 {
            if read == 0 {
                return Ok(None);
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "frame length prefix cut short",
            ));
        }
        read += n;
    }
    let length = u32::from_be_bytes(prefix) as usize;
    if length == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "empty frame",
        ));
    }
    if length > max_frame_bytes {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("frame of {length} bytes exceeds the {max_frame_bytes} byte cap"),
        ));
    }
    let mut bytes = vec![0u8; length];
    let mut filled = 0;
    while filled < length {
        let n = reader.read(&mut bytes[filled..])?;
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "frame body cut short",
            ));
        }
        filled += n;
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

/// Writes one length-prefixed frame, refusing to even serialize a frame that
/// would exceed the cap.
pub fn write_frame(
    writer: &mut impl Write,
    frame: &impl Serialize,
    max_frame_bytes: usize,
) -> std::io::Result<()> {
    let json = serde_json::to_vec(frame)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    if json.len() > max_frame_bytes {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("frame of {} bytes exceeds the {max_frame_bytes} byte cap", json.len()),
        ));
    }
    writer.write_all(&(json.len() as u32).to_be_bytes())?;
    writer.write_all(&json)?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip_through_the_prefix_codec() {
        let frame = DataFromHost {
            to: BROADCAST_TARGET.into(),
            line: "{\"type\":\"record\"}".into(),
        };
        let mut buffer = Vec::new();
        write_frame(&mut buffer, &frame, DEFAULT_MAX_FRAME_BYTES).unwrap();
        let mut cursor = std::io::Cursor::new(buffer);
        let read_back = read_frame(&mut cursor, DEFAULT_MAX_FRAME_BYTES)
            .unwrap()
            .expect("one frame then eof handled by next call");
        let parsed: DataFromHost = serde_json::from_str(&read_back).unwrap();
        assert_eq!(parsed, frame);
        assert!(read_frame(&mut cursor, DEFAULT_MAX_FRAME_BYTES)
            .unwrap()
            .is_none());
    }

    #[test]
    fn oversized_frames_are_refused_before_allocation() {
        // Announce a frame far past the cap; the reader must refuse without
        // touching the (nonexistent) body.
        let mut wire = (5u32 * DEFAULT_MAX_FRAME_BYTES as u32).to_be_bytes().to_vec();
        wire.extend_from_slice(b"never read");
        let mut cursor = std::io::Cursor::new(wire);
        let error = read_frame(&mut cursor, DEFAULT_MAX_FRAME_BYTES).unwrap_err();
        assert!(error.to_string().contains("exceeds"));
    }

    #[test]
    fn oversized_payloads_refuse_at_write() {
        let frame = DataFromHost {
            to: "x".repeat(DEFAULT_MAX_FRAME_BYTES + 1),
            line: "y".into(),
        };
        let mut buffer = Vec::new();
        let error = write_frame(&mut buffer, &frame, DEFAULT_MAX_FRAME_BYTES).unwrap_err();
        assert!(error.to_string().contains("exceeds"));
        assert!(buffer.is_empty());
    }

    #[test]
    fn hello_and_server_frames_keep_their_wire_tags() {
        let host_hello = serde_json::to_value(HelloFrame::Host).unwrap();
        assert_eq!(host_hello["type"], "host");
        let join = serde_json::to_value(HelloFrame::Join {
            room: "ABCDEF".into(),
            token: "GHIJKLMNOP".into(),
            user: "u-1".into(),
        })
        .unwrap();
        assert_eq!(join["type"], "join");
        assert_eq!(join["room"], "ABCDEF");
        let error = serde_json::to_value(ServerFrame::Error {
            reason: "room-full".into(),
        })
        .unwrap();
        assert_eq!(error["type"], "error");
        assert_eq!(error["reason"], "room-full");
    }
}

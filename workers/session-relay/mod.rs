//! Relay transport for multiplayer painting sessions beyond LAN: the relay
//! wire protocol, the room-routing server core, the invite-code format, the
//! TLS boundary, and the standalone `thaum-session-relay` server binary.
//!
//! Ownership boundary: see `contract.md`. The relay never parses session
//! content — inner lines are opaque strings passed through verbatim, so the
//! session host/client cores cannot tell this lane from the LAN wire. Rooms
//! are in-memory only and die with their host connection; the session host
//! owns all document truth.

pub mod relay_codes;
#[path = "relay_link.rs"]
pub mod relay_link;
pub mod relay_protocol;
pub mod relay_server;
pub mod relay_tls;

pub use relay_codes::{generate_code, parse_code};
pub use relay_protocol::{
    read_frame, write_frame, DataFromHost, DataFromJoiner, DataToHost, HelloFrame, KickFromHost,
    ServerFrame, BROADCAST_TARGET, DEFAULT_MAX_FRAME_BYTES,
};
pub use relay_server::{serve_relay, serve_relay_with, RelayCaps, RelayServer};
pub use relay_tls::{
    connect as relay_connect, connect_trusting as relay_connect_trusting, is_plain_target,
    RelayClientLink, DEFAULT_RELAY_ADDRESS, TlsRelayServerAcceptor,
};

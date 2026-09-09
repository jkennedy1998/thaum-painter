//! Live relay dial test — the painter's REAL client lane against the REAL
//! deployed relay. Opt-in via `THAUM_RELAY_LIVE=1` so ordinary `cargo test`
//! runs never touch the network.
//!
//! Run:
//!   THAUM_RELAY_LIVE=1 cargo test -p thaum-painter-workers \
//!     --test relay_live_dial -- --nocapture
//!
//! Pass = the painter lane can claim a host room over the deployed TLS relay
//! and receives a minted `<room6>-<token10>` invite code — the exact path
//! `SessionNet::host_relay` takes when the panel starts hosting with invites.

use std::sync::{Arc, Mutex};

use thaum_painter_workers::session_host::SessionHost;
use thaum_painter_workers::session_relay::relay_link::spawn_relay_host_bridge;
use thaum_painter_workers::session_relay::relay_tls::DEFAULT_RELAY_ADDRESS;

#[test]
fn painter_lane_claims_host_room_on_deployed_relay() {
    if std::env::var("THAUM_RELAY_LIVE").ok().as_deref() != Some("1") {
        eprintln!("skipped: set THAUM_RELAY_LIVE=1 to dial {DEFAULT_RELAY_ADDRESS}");
        return;
    }
    let host = Arc::new(Mutex::new(SessionHost::new()));
    let handle = spawn_relay_host_bridge(host, DEFAULT_RELAY_ADDRESS)
        .expect("painter relay lane dials the deployed relay and claims a room");
    let code = handle.code.clone();
    assert!(handle.is_alive(), "host bridge alive after claim");
    let (room, token) = code
        .split_once('-')
        .expect("minted code is <room6>-<token10>");
    assert_eq!(room.len(), 6, "room id is 6 glyphs: {code}");
    assert_eq!(token.len(), 10, "room token is 10 glyphs: {code}");
    println!("live relay dial OK via {DEFAULT_RELAY_ADDRESS}; minted invite code: {code}");
    drop(handle);
}

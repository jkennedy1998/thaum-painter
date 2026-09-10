//! TLS at the relay boundary — the one place the wire gains encryption.
//!
//! Server side: `TlsRelayServerAcceptor` wraps each accepted socket in rustls
//! before the room-routing core sees it (the upgrader seam already exists in
//! `relay_server.rs`). Certs come from files (certbot output for the deployed
//! hostname) — the binary never generates or ships identity.
//!
//! Client side: `connect` is the one dial rule the whole client lane uses.
//! Plain TCP only ever touches loopback (or an explicit `plain://` scheme for
//! in-process tests) — mirroring the server's loopback-only `--plain` stance,
//! so an unencrypted relay connection can never cross the internet by
//! accident. Every other target dials TLS with the webpki system roots, which
//! validates the deployed Let's Encrypt cert like any browser would.

use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::{CertificateDer, ServerName};
use rustls::server::ServerConnection;
use rustls_pemfile::certs;
use rustls::RootCertStore;

use crate::session_relay::relay_server::{
    ConnState, RelayConnectionUpgrader, RelayStreamBox, SharedStream, RELAY_STREAM_READ_SLICE,
};

/// Handshake window: a relay that doesn't finish the TLS or claim exchange in
/// five seconds is dead to us.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

/// The built-in relay the client lane dials when an invite code is used
/// without an explicit relay address. The deployed dev relay on JOBO, TLS on
/// 443. Hosting stays LAN-by-default (the free direct lane); only code-shaped
/// joins fall back here. `THAUM_SESSION_RELAY` overrides for both lanes.
pub const DEFAULT_RELAY_ADDRESS: &str = "relay.jartanddesign.com:443";

/// One dialed relay connection: the shared boxed stream (read + write halves
/// interleave under the lock, read slices release it) plus the raw socket for
/// timeout control and force-close.
pub struct RelayClientLink {
    pub shared: SharedStream,
    pub socket: TcpStream,
}

impl RelayClientLink {
    /// Force-closes the underlying socket: both halves fail, the reader
    /// unblocks, the link dies.
    pub fn shutdown(&self) {
        use std::net::Shutdown;
        let _ = self.socket.shutdown(Shutdown::Both);
    }
}

/// True when `address` must dial plaintext: explicit `plain://` scheme, or a
/// loopback host (an explicit `tls://` forces TLS even on loopback). The
/// default for every other target is TLS, no exceptions.
pub fn is_plain_target(address: &str) -> bool {
    if address.starts_with("tls://") {
        return false;
    }
    let host = address
        .strip_prefix("plain://")
        .unwrap_or(address)
        .rsplit_once(':')
        .map(|(host, _)| host.trim_matches(|c| c == '[' || c == ']'))
        .unwrap_or(address);
    host == "localhost"
        || host
            .parse::<std::net::IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false)
}

/// Dials the relay with the production trust rule: loopback / `plain://` =
/// plain TCP, anything else = TLS against the webpki system roots.
pub fn connect(address: &str) -> std::io::Result<RelayClientLink> {
    connect_trusting(address, None)
}

/// Dials the relay with explicit extra trust roots (tests pin a generated
/// self-signed cert here; production passes `None` for webpki roots).
pub fn connect_trusting(
    address: &str,
    extra_roots: Option<Vec<CertificateDer<'static>>>,
) -> std::io::Result<RelayClientLink> {
    let plain = is_plain_target(address);
    let target = address
        .strip_prefix("plain://")
        .or_else(|| address.strip_prefix("tls://"))
        .unwrap_or(address);
    let host = target
        .rsplit_once(':')
        .map(|(host, _)| host.trim_matches(|c| c == '[' || c == ']'))
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "relay address needs host:port",
            )
        })?
        .to_string();

    // Hairpin escape first: when the relay is public, probe the local
    // machine for a relay serving the same port before the honest dial.
    // A machine that hosts the relay itself cannot dial its own public IP
    // without router NAT loopback (seen live 2026-09-09: the painter on
    // JOBO timed out reaching the deployed relay on JOBO). TLS still
    // verifies the real hostname, so a foreign local service fails the
    // handshake and we fall through to the honest dial.
    if !plain {
        if let Some(result) = try_local_relay(&host, target, &extra_roots) {
            // Only a PROVEN local relay answers the hairpin probe: the loop
            // back dial succeeded AND the TLS handshake verified the real
            // hostname. Any other loopback listener fails that handshake —
            // fall through to the honest dial (the comment always promised
            // this; the code used to return the failure instead,
            // J 2026-09-09).
            if let Ok(link) = result {
                return Ok(link);
            }
        }
    }

    let socket = dial_socket(target)?;
    if plain {
        socket.set_read_timeout(Some(RELAY_STREAM_READ_SLICE)).ok();
        let control = socket.try_clone()?;
        let stream: RelayStreamBox = Box::new(socket);
        return Ok(RelayClientLink {
            shared: Arc::new(std::sync::Mutex::new(ConnState {
                stream,
                buffer: Default::default(),
            })),
            socket: control,
        });
    }
    tls_link(socket, host, extra_roots)
}

/// The hairpin escape: probe loopback on the target's port. Returns `None`
/// whenever the probe cannot prove the local machine serves the relay —
/// loopback refused (nothing local) or any TLS failure (foreign local
/// service) — so the caller falls through to the honest public dial.
fn try_local_relay(
    host: &str,
    target: &str,
    extra_roots: &Option<Vec<CertificateDer<'static>>>,
) -> Option<std::io::Result<RelayClientLink>> {
    let resolved: Vec<_> = target.to_socket_addrs().ok()?.collect();
    // Public targets only: an explicit private/loopback address already
    // names its machine, and hairpin NAT is a public-IP phenomenon.
    if !resolved.iter().any(|address| is_public_ip(address.ip())) {
        return None;
    }
    let port = resolved.first()?.port();
    let loopback = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let socket = TcpStream::connect_timeout(&loopback, DIAL_ADDRESS_TIMEOUT).ok()?;
    Some(tls_link(socket, host.to_string(), extra_roots.clone()))
}

/// Rough public-address test (`IpAddr::is_global` is the honest rule but is
/// gated behind newer std than this workspace pins; the private-use, loopback,
/// link-local, and unspecified exclusions cover every real relay address).
fn is_public_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            !v4.is_loopback()
                && !v4.is_private()
                && !v4.is_link_local()
                && !v4.is_unspecified()
                && !v4.is_broadcast()
        }
        std::net::IpAddr::V6(v6) => {
            !v6.is_loopback() && !v6.is_unspecified() && (v6.segments()[0] & 0xfe00) != 0xfe00
        }
    }
}

/// Wraps an already-dialed socket in the client TLS link: trust roots, SNI
/// from the dialed hostname, handshake driven to completion under the
/// handshake window.
fn tls_link(
    socket: TcpStream,
    host: String,
    extra_roots: Option<Vec<CertificateDer<'static>>>,
) -> std::io::Result<RelayClientLink> {
    // TLS path: keep a socket clone for shutdown/timeout control before the
    // stream takes ownership of its twin.
    let control = socket.try_clone()?;

    let mut roots = RootCertStore::empty();
    match extra_roots {
        Some(certs) => {
            for cert in certs {
                roots
                    .add(cert)
                    .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
            }
        }
        None => {
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        }
    }
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let name = ServerName::try_from(host.clone())
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
    let connection = rustls::ClientConnection::new(Arc::new(config), name)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;

    // Handshake window: bound the socket so a dead relay fails the dial
    // instead of hanging the host/joiner bridge.
    socket.set_read_timeout(Some(HANDSHAKE_TIMEOUT)).ok();
    socket.set_nodelay(true).ok();
    let mut stream = rustls::StreamOwned::new(connection, socket);
    drive_handshake(&mut stream, &host)?;

    // Steady state: slice reads so the shared lock releases between reads.
    control.set_read_timeout(Some(RELAY_STREAM_READ_SLICE)).ok();
    let boxed: RelayStreamBox = Box::new(stream);
    Ok(RelayClientLink {
        shared: Arc::new(std::sync::Mutex::new(ConnState {
            stream: boxed,
            buffer: Default::default(),
        })),
        socket: control,
    })
}

/// Flushes the ClientHello out, then reads until the rustls handshake
/// completes. The socket timeout bounds the whole exchange: a relay that
/// never answers fails the dial instead of hanging the caller.
fn drive_handshake<S>(
    stream: &mut rustls::StreamOwned<rustls::ClientConnection, S>,
    peer: &str,
) -> std::io::Result<()>
where
    S: std::io::Read + std::io::Write,
{
    use std::io::Write as _;
    stream.flush()?;
    let mut probe = [0u8; 1];
    loop {
        if !stream.conn.is_handshaking() {
            return Ok(());
        }
        match stream.read(&mut probe) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    format!("relay {peer} closed during TLS handshake"),
                ))
            }
            Ok(_) => {}
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut =>
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!("relay TLS handshake with {peer} timed out"),
                ))
            }
            Err(error) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("relay TLS handshake with {peer} failed: {error}"),
                ))
            }
        }
    }
}

/// Per-address connect budget. Bounds each dial so one resolved family that
/// silently drops SYNs (a published AAAA behind a closed inbound IPv6
/// firewall, seen live) cannot blackhole the whole dial past the client's
/// handshake window.
const DIAL_ADDRESS_TIMEOUT: Duration = Duration::from_secs(3);

fn dial_socket(target: &str) -> std::io::Result<TcpStream> {
    let mut addresses: Vec<_> = target.to_socket_addrs()?.collect();
    // IPv4 first: the relay is published behind an IPv4 port-forward; an
    // unreachable AAAA must be the fallback attempt, not the first one.
    addresses.sort_by_key(|address| !address.is_ipv4());
    let mut last = None;
    for address in &addresses {
        match TcpStream::connect_timeout(address, DIAL_ADDRESS_TIMEOUT) {
            Ok(socket) => return Ok(socket),
            Err(error) => last = Some(error),
        }
    }
    Err(last.unwrap_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::AddrNotAvailable,
            format!("relay address {target} resolved to nothing"),
        )
    }))
}

/// The deploy upgrader: rustls over every accepted socket, certs from files.
pub struct TlsRelayServerAcceptor {
    config: Arc<rustls::ServerConfig>,
}

impl TlsRelayServerAcceptor {
    /// Loads a PEM cert chain + private key (certbot's `fullchain.pem` /
    /// `privkey.pem`). Refuses anything incomplete so a mis-deploy fails at
    /// boot, not at first connection.
    pub fn from_pem_files(cert_path: &str, key_path: &str) -> std::io::Result<Self> {
        let certs: Vec<CertificateDer<'static>> = certs(&mut BufReader::new(
            File::open(cert_path).map_err(|error| {
                std::io::Error::new(
                    error.kind(),
                    format!("relay TLS cert {cert_path}: {error}"),
                )
            })?,
        ))
        .collect::<Result<_, _>>()
        .map_err(|error| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, format!("bad cert PEM: {error}"))
        })?;
        if certs.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("relay TLS cert {cert_path} contains no certificates"),
            ));
        }
        let key = rustls_pemfile::private_key(&mut BufReader::new(
            File::open(key_path).map_err(|error| {
                std::io::Error::new(error.kind(), format!("relay TLS key {key_path}: {error}"))
            })?,
        ))
        .map_err(|error| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, format!("bad key PEM: {error}"))
        })?
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("relay TLS key {key_path} contains no private key"),
            )
        })?;
        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .map_err(|error| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, format!("bad TLS pair: {error}"))
            })?;
        Ok(Self {
            config: Arc::new(config),
        })
    }
}

impl RelayConnectionUpgrader for TlsRelayServerAcceptor {
    fn upgrade(&self, stream: TcpStream) -> std::io::Result<RelayStreamBox> {
        stream.set_read_timeout(Some(RELAY_STREAM_READ_SLICE)).ok();
        let connection = ServerConnection::new(Arc::clone(&self.config)).map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("relay TLS accept setup failed: {error}"),
            )
        })?;
        let mut tls = rustls::StreamOwned::new(connection, stream);
        // Drive the handshake to completion here: the room-routing core
        // expects a usable stream, not a half-open one. Mid-handshake
        // WouldBlock is NORMAL (the next flight is in flight); an overall
        // deadline turns a dead client into a handshake failure, not a hang.
        let deadline = std::time::Instant::now() + HANDSHAKE_TIMEOUT;
        let mut scratch = [0u8; 8192];
        let mut prefix: Vec<u8> = Vec::new();
        while tls.conn.is_handshaking() {
            if std::time::Instant::now() > deadline {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "relay TLS handshake timed out",
                ));
            }
            match tls.read(&mut scratch) {
                Ok(0) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "client closed during relay TLS handshake",
                    ))
                }
                // A read can complete the handshake AND carry the first
                // application-data records in the same batch — never discard
                // them; they replay before any fresh read (see below).
                Ok(n) if !tls.conn.is_handshaking() => prefix.extend_from_slice(&scratch[..n]),
                Ok(_) => {}
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        || error.kind() == std::io::ErrorKind::TimedOut =>
                {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("relay TLS accept failed: {error}"),
                    ))
                }
            }
        }
        Ok(Box::new(HandshakeBufferedStream {
            tls,
            prefix,
            prefix_pos: 0,
        }))
    }
}

/// TLS server stream that replays application bytes captured during the
/// handshake before reading anything fresh from rustls.
struct HandshakeBufferedStream {
    tls: rustls::StreamOwned<rustls::server::ServerConnection, TcpStream>,
    prefix: Vec<u8>,
    prefix_pos: usize,
}

impl Read for HandshakeBufferedStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.prefix_pos < self.prefix.len() {
            let take = (self.prefix.len() - self.prefix_pos).min(buf.len());
            buf[..take].copy_from_slice(&self.prefix[self.prefix_pos..self.prefix_pos + take]);
            self.prefix_pos += take;
            return Ok(take);
        }
        self.tls.read(buf)
    }
}

impl Write for HandshakeBufferedStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.tls.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.tls.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_relay::relay_server::read_frame_shared;
    use crate::session_relay::relay_protocol::write_frame;
    use crate::session_relay::relay_server::{serve_relay_with, RelayCaps};
    use std::net::TcpListener;

    fn self_signed_cert() -> (CertificateDer<'static>, String, String) {
        let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
            .expect("generate test cert");
        let cert_path = std::env::temp_dir().join(format!(
            "relay-test-{}-cert.pem",
            std::process::id()
        ));
        let key_path = std::env::temp_dir().join(format!(
            "relay-test-{}-key.pem",
            std::process::id()
        ));
        std::fs::write(&cert_path, certified.cert.pem()).expect("write cert");
        std::fs::write(&key_path, certified.signing_key.serialize_pem()).expect("write key");
        (certified.cert.der().clone(), cert_path.display().to_string(), key_path.display().to_string())
    }

    #[test]
    fn loopback_and_plain_scheme_dial_plaintext_everything_else_tls() {
        assert!(is_plain_target("127.0.0.1:4748"));
        assert!(is_plain_target("localhost:4748"));
        assert!(is_plain_target("plain://localhost:4748"));
        assert!(!is_plain_target("relay.jartanddesign.com:443"));
        assert!(!is_plain_target("10.0.0.5:4748"));
    }

    #[test]
    fn public_ip_test_excludes_local_ranges() {
        assert!(is_public_ip(std::net::IpAddr::from([73, 36, 136, 170])));
        assert!(!is_public_ip(std::net::IpAddr::from([127, 0, 0, 1])));
        assert!(!is_public_ip(std::net::IpAddr::from([10, 0, 0, 68])));
        assert!(!is_public_ip(std::net::IpAddr::from([192, 168, 1, 1])));
    }

    #[test]
    fn tls_client_and_server_exchange_frames_end_to_end() {
        let (cert_der, cert_path, key_path) = self_signed_cert();
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
        let address = format!("127.0.0.1:{}", listener.local_addr().unwrap().port());
        let acceptor = Arc::new(
            TlsRelayServerAcceptor::from_pem_files(&cert_path, &key_path).expect("acceptor"),
        );
        let server =
            serve_relay_with(listener, RelayCaps::default(), acceptor).expect("serve");

        let host_port = address.rsplit_once(':').expect("host:port").1.to_string();
        let client = connect_trusting(&format!("tls://localhost:{host_port}"), Some(vec![cert_der]))
            .expect("client dials TLS relay");

        // Claim a host room through the encrypted relay: hello out, the
        // minted ServerFrame::Ok comes back over the same TLS stream.
        use crate::session_relay::relay_protocol::HelloFrame;
        use crate::session_relay::relay_server::read_frame_shared as read_shared;
        {
            let mut state = client.shared.lock().expect("lock");
            write_frame(&mut state.stream, &HelloFrame::Host, 1 << 20).expect("write hello");
        }
        let frame = read_shared(&client.shared, 1 << 20)
            .expect("read claim answer")
            .expect("claim answer present");
        let claim: crate::session_relay::relay_protocol::ServerFrame =
            serde_json::from_str(&frame).expect("claim answer parses");
        match claim {
            crate::session_relay::relay_protocol::ServerFrame::Ok { room, token } => {
                assert_eq!(room.len(), crate::session_relay::relay_codes::ROOM_ID_LEN);
                assert_eq!(token.len(), crate::session_relay::relay_codes::ROOM_TOKEN_LEN);
            }
            other => panic!("expected a room claim, got {other:?}"),
        }
        let _ = server; // keep the accept loop alive for the exchange
        drop(client);
    }
}

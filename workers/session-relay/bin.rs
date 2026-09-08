//! The `thaum-session-relay` binary: the standalone room-routing server.
//!
//! Consumer-ready stance from day one: TLS via cert/key files is the real
//! listener (deploy slice wires certbot output from the jartanddesign
//! subdomain); plain TCP is a dev/test escape hatch that is ONLY allowed to
//! bind loopback, so an unencrypted listener can never face the internet by
//! accident.

use std::net::TcpListener;

use thaum_painter_workers::session_relay::{serve_relay, RelayCaps};

fn main() {
    let mut port: u16 = 4748;
    let mut tls_cert: Option<String> = None;
    let mut tls_key: Option<String> = None;
    let mut plain = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => {
                port = args
                    .next()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(port);
            }
            "--tls-cert" => tls_cert = args.next(),
            "--tls-key" => tls_key = args.next(),
            "--plain" => plain = true,
            other => {
                eprintln!("unknown argument: {other}");
                eprintln!(
                    "usage: thaum-session-relay [--port 4748] [--tls-cert <path> \
                     --tls-key <path>] [--plain (loopback-only dev mode)]"
                );
                std::process::exit(2);
            }
        }
    }

    if plain {
        // Plain mode is the unencrypted listener: loopback only, ever.
        let listener = match TcpListener::bind(("127.0.0.1", port)) {
            Ok(listener) => listener,
            Err(error) => {
                eprintln!("failed to bind 127.0.0.1:{port}: {error}");
                std::process::exit(1);
            }
        };
        eprintln!(
            "session relay (PLAIN, loopback-only dev mode) listening on 127.0.0.1:{port}"
        );
        if let Ok(server) = serve_relay(listener, RelayCaps::default()) {
            keep_alive(server);
        }
        return;
    }

    let (Some(cert), Some(key)) = (tls_cert, tls_key) else {
        eprintln!(
            "the relay must run TLS: pass --tls-cert and --tls-key (certbot files). \
             For local development only, use --plain (loopback-only)."
        );
        std::process::exit(2);
    };
    // TLS is the only internet-facing listener: rustls wraps every accepted
    // socket via the same upgrader seam the plain loopback dev mode bypasses.
    let acceptor =
        match thaum_painter_workers::session_relay::TlsRelayServerAcceptor::from_pem_files(
            &cert,
            &key,
        ) {
            Ok(acceptor) => acceptor,
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(2);
            }
        };
    let listener = match TcpListener::bind(("0.0.0.0", port)) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("failed to bind 0.0.0.0:{port}: {error}");
            std::process::exit(1);
        }
    };
    eprintln!("session relay (TLS) listening on 0.0.0.0:{port}");
    match thaum_painter_workers::session_relay::serve_relay_with(
        listener,
        thaum_painter_workers::session_relay::RelayCaps::default(),
        std::sync::Arc::new(acceptor),
    ) {
        Ok(server) => keep_alive(server),
        Err(error) => {
            eprintln!("relay serve failed: {error}");
            std::process::exit(1);
        }
    }
}

fn keep_alive(server: thaum_painter_workers::session_relay::RelayServer) {
    // Park the main thread; Ctrl+C / SIGTERM tears the process down and the
    // Drop impl stops the accept loop. Rooms die with the process by design.
    let _ = server;
    loop {
        std::thread::park();
    }
}

# session-relay deploy runbook (JOBO dev relay)

One-command bring-up for the deployed dev relay. The binary is the same one
clients get — self-hosting is "hand a friend the binary + this runbook".

## Shape

- `thaum-session-relay --port 443 --tls-cert <fullchain> --tls-key <privkey>`
  — the ONLY internet-facing mode. Plaintext (`--plain`) binds loopback only,
  so an unencrypted relay can never face the internet by accident.
- Rooms are memory-only: no disk state, no persistence. Rooms die with the
  host connection; the host owns all document truth.
- Clients dial `<hostname>:443` (TLS, webpki roots — a Let's Encrypt cert
  validates like any website). Invite codes are `<room6>-<token10>`.

## One-time: DNS + cert

1. Point an A record for `relay.jartanddesign.com` at JOBO's public IP.
2. Forward/allow TCP 443 to JOBO (firewall + router if NAT'd).
3. Certbot standalone (port 80 must be free during issuance):
   `sudo certbot certonly --standalone -d relay.jartanddesign.com`

## One-time: install

```
cargo build --release -p thaum-painter-workers --bin thaum-session-relay
sudo cp orchestration/builds/.cargo-cache/release/thaum-session-relay /opt/thaum-session-relay/
sudo cp orchestration/session-relay/deploy/thaum-session-relay.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now thaum-session-relay
```

## Verify

- `systemctl status thaum-session-relay` — active, listening on 0.0.0.0:443.
- TLS: `openssl s_client -connect relay.jartanddesign.com:443 -brief` shows
  the LE cert.
- Smoke: `cargo test -p thaum-painter-workers --lib session_relay` locally
  (the in-process TLS round-trip test), then a real app test: host on one
  device, join with the minted code from another network.

## Renewal

Certbot renewal replaces the PEM files; the relay loads them at boot only:

```
sudo tee /etc/letsencrypt/renewal-hooks/deploy/thaum-session-relay.sh <<'EOF'
#!/bin/sh
systemctl restart thaum-session-relay
EOF
sudo chmod +x /etc/letsencrypt/renewal-hooks/deploy/thaum-session-relay.sh
```

## Caps (first build, by design)

Record 1 MiB, 8 members/room, 256 rooms, 30 claims/min/IP — all in
`RelayCaps::default()` (`workers/session-relay/relay_server.rs`). The relay
never parses session content; it routes opaque lines per room.

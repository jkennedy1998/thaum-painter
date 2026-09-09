//! LAN session discovery: how "join local" finds hosted sessions without
//! any typed address. The host side answers one UDP broadcast probe with
//! its display name + LAN port; the joiner side broadcasts the probe and
//! collects replies for a short window. Zero registry, zero disk state —
//! a host is discoverable exactly while its responder lives.
//!
//! The `DiscoveryPoller` owns the scan cadence so no caller re-rolls timer
//! logic: the orchestration layer feeds it the panel's visible/not-visible
//! truth every frame (`set_active`), and the poller scans immediately on
//! the false→true transition and every five seconds while active. Scans
//! run on a worker thread (the 400ms reply window must not hitch the frame
//! loop); results surface through `take_discovered` for the next `sync`.

use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use thaum_painter_domain::debug_log;

/// One discovered LAN host: the host's display name plus a dialable
/// `ip:port`. The panel renders the name; the orchestration layer dials
/// the address verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredHost {
    pub name: String,
    pub address: String,
}

/// The UDP port both sides agree on. Probes are broadcast here; responders
/// listen here. Fixed like the session TCP default (4747): one well-known
/// discovery seam, not a negotiated one.
pub const DISCOVERY_PORT: u16 = 47478;

const PROBE: &str = "THAUM-DISCOVER-QUERY";
const REPLY_MAGIC: &str = "THAUM-DISCOVER-REPLY";
const REPLY_WINDOW: Duration = Duration::from_millis(400);
/// Panel-open cadence (settled with J): scan immediately on open, then
/// every five seconds while the panel stays open, nothing while closed.
const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Spawns the host-side responder thread: answers discovery probes with
/// `name` + `port` until dropped or `stop()`. Fails soft — a machine that
/// cannot bind the discovery port (firewall, collision) still hosts fine;
/// it just does not appear in "join local" lists.
pub fn spawn_discovery_responder(
    host_name: impl Into<String>,
    lan_port: u16,
) -> std::io::Result<DiscoveryResponder> {
    let socket = UdpSocket::bind(("0.0.0.0", DISCOVERY_PORT))?;
    socket.set_read_timeout(Some(REPLY_WINDOW))?;
    let stopped = Arc::new(AtomicBool::new(false));
    let worker_stopped = Arc::clone(&stopped);
    let reply = format!("{REPLY_MAGIC}\t{}\t{lan_port}", host_name.into());
    let worker = thread::spawn(move || {
        let mut probe = [0u8; PROBE.len() + 1];
        while !worker_stopped.load(Ordering::Relaxed) {
            match socket.recv_from(&mut probe) {
                Ok((read, source)) => {
                    if probe[..read] != *PROBE.as_bytes() {
                        continue;
                    }
                    let _ = socket.send_to(reply.as_bytes(), source);
                }
                Err(_) => continue, // read timeout: loop back to the stop check
            }
        }
    });
    Ok(DiscoveryResponder { stopped, worker: Some(worker) })
}

/// Handle keeping one host discoverable. Dropping it detaches the thread's
/// stop flag too (the thread exits on its next stop check); explicit
/// `stop()` joins immediately.
pub struct DiscoveryResponder {
    stopped: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl DiscoveryResponder {
    /// Stops and joins the responder thread.
    pub fn stop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for DiscoveryResponder {
    fn drop(&mut self) {
        self.stop();
    }
}

/// One broadcast discovery scan, blocking for the reply window. Used by the
/// poller's worker thread; also the unit-test seam for the wire shape.
fn scan_once() -> Vec<DiscoveredHost> {
    let socket = match UdpSocket::bind(("0.0.0.0", 0)) {
        Ok(socket) => socket,
        Err(error) => {
            debug_log::error("session", &format!("discovery scan bind failed: {error}"));
            return Vec::new();
        }
    };
    socket.set_broadcast(true).ok();
    socket
        .send_to(PROBE.as_bytes(), ("255.255.255.255", DISCOVERY_PORT))
        .ok();
    socket.set_read_timeout(Some(Duration::from_millis(100))).ok();
    let start = Instant::now();
    let mut buffer = [0u8; 512];
    let mut hosts = Vec::new();
    while start.elapsed() < REPLY_WINDOW {
        match socket.recv_from(&mut buffer) {
            Ok((read, source)) => {
                let Ok(line) = std::str::from_utf8(&buffer[..read]) else {
                    continue;
                };
                if let Some(host) = parse_reply(line, &source.ip().to_string()) {
                    if !hosts.contains(&host) {
                        hosts.push(host);
                    }
                }
            }
            Err(_) => continue,
        }
    }
    hosts
}

/// Parses one reply line into a host. The reply's source address is the
/// dialable truth — the port in the payload is what the host serves on,
/// the IP is where the reply came from.
fn parse_reply(line: &str, source_ip: &str) -> Option<DiscoveredHost> {
    let mut parts = line.split('\t');
    if parts.next()? != REPLY_MAGIC {
        return None;
    }
    let name = parts.next()?.trim();
    let port = parts.next()?.trim().parse::<u16>().ok()?;
    if name.is_empty() {
        return None;
    }
    Some(DiscoveredHost { name: name.to_string(), address: format!("{source_ip}:{port}") })
}

/// Frame-facing discovery cadence owner. The orchestration layer calls
/// `set_active` once per frame with the panel's visibility truth; the
/// poller translates that into scans (immediately on open, then every
/// five seconds) without any caller-side timers.
pub struct DiscoveryPoller {
    active: bool,
    last_scan: Option<Instant>,
    worker: Option<JoinHandle<Vec<DiscoveredHost>>>,
    latest: Arc<Mutex<Vec<DiscoveredHost>>>,
    results_seen: usize,
}

impl Default for DiscoveryPoller {
    fn default() -> Self {
        Self::new()
    }
}

impl DiscoveryPoller {
    pub fn new() -> Self {
        Self {
            active: false,
            last_scan: None,
            worker: None,
            latest: Arc::new(Mutex::new(Vec::new())),
            results_seen: 0,
        }
    }

    /// Opens or closes the discovery window. `false → true` scans
    /// immediately; staying `true` rescans every `POLL_INTERVAL`; closing
    /// drops in-flight interest (a finishing scan's results are simply
    /// superseded when reopened).
    pub fn set_active(&mut self, active: bool) {
        self.active = active;
        if !active {
            self.last_scan = None;
            return;
        }
        self.pump(true);
    }

    /// Per-frame pump: harvests finished scans, starts new ones on cadence.
    fn pump(&mut self, force_first: bool) {
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.is_finished())
        {
            if let Some(worker) = self.worker.take() {
                if let Ok(found) = worker.join() {
                    *self.latest.lock().expect("discovery results lock") = found;
                    self.results_seen += 1;
                }
            }
        }
        let due = force_first
            || self
                .last_scan
                .is_none_or(|last| last.elapsed() >= POLL_INTERVAL);
        if self.active && due && self.worker.is_none() {
            self.last_scan = Some(Instant::now());
            let latest = Arc::clone(&self.latest);
            self.worker = Some(thread::spawn(move || {
                let found = scan_once();
                if !found.is_empty() {
                    *latest.lock().expect("discovery results lock") = found.clone();
                }
                found
            }));
        }
    }

    /// The newest finished scan's hosts, if one landed since the last take.
    /// Call once per frame; empty scans still count as results so the panel
    /// can render "none found" from real freshness.
    pub fn take_discovered(&mut self) -> Option<Vec<DiscoveredHost>> {
        self.pump(false);
        if self.results_seen > 0 {
            self.results_seen -= 1;
            Some(self.latest.lock().expect("discovery results lock").clone())
        } else {
            None
        }
    }
}

impl Drop for DiscoveryPoller {
    fn drop(&mut self) {
        // Detach: a scan thread is short-lived and owns nothing the frame
        // loop needs; joining here would hitch shutdown for up to the
        // reply window.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_parse_only_well_shaped_lines() {
        let host = parse_reply(&format!("{REPLY_MAGIC}\tLiving Room\t4747"), "192.168.1.5")
            .expect("valid reply parses");
        assert_eq!(host.name, "Living Room");
        assert_eq!(host.address, "192.168.1.5:4747");

        assert!(parse_reply("garbage", "192.168.1.5").is_none());
        assert!(parse_reply(&format!("{REPLY_MAGIC}\t\t4747"), "192.168.1.5").is_none());
        assert!(parse_reply(&format!("{REPLY_MAGIC}\tHost\tnotaport"), "192.168.1.5").is_none());
    }

    #[test]
    fn responder_answers_one_probe_and_stop_joins() {
        let mut responder = spawn_discovery_responder("Test Host", 4747)
            .expect("responder binds the discovery port in tests");
        // Give the responder thread a beat to enter recv.
        thread::sleep(Duration::from_millis(100));

        let socket = UdpSocket::bind(("127.0.0.1", 0)).expect("probe socket");
        socket
            .send_to(PROBE.as_bytes(), ("127.0.0.1", DISCOVERY_PORT))
            .expect("probe send");
        socket.set_read_timeout(Some(Duration::from_secs(2))).ok();
        let mut buffer = [0u8; 512];
        let (read, _) = socket
            .recv_from(&mut buffer)
            .expect("responder answers the probe");
        let reply = std::str::from_utf8(&buffer[..read]).expect("reply is utf8");

        let host = parse_reply(reply, "127.0.0.1").expect("reply parses");
        assert_eq!(host.name, "Test Host");
        assert_eq!(host.address, "127.0.0.1:4747");

        responder.stop();
    }

    #[test]
    fn poller_scans_immediately_on_open_and_harvests_results() {
        let mut poller = DiscoveryPoller::new();
        poller.set_active(true);
        // The scan window is 400ms; give it time to finish, then harvest.
        thread::sleep(REPLY_WINDOW + Duration::from_millis(200));
        let found = poller
            .take_discovered()
            .expect("the immediate open-scan finishes");
        // An empty LAN usually scans empty, but a parallel responder test on
        // the same discovery port may answer — the point is the cadence
        // landed, so only shape-check the results.
        assert!(found.iter().all(|host| !host.name.is_empty()));

        // Closing the window stops scanning; reopening scans immediately.
        poller.set_active(false);
        assert!(poller.take_discovered().is_none());
        poller.set_active(true);
        thread::sleep(REPLY_WINDOW + Duration::from_millis(200));
        assert!(poller.take_discovered().is_some());
    }
}

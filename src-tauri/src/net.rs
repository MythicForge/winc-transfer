use crate::crypto::{self, EncryptedStream};
use crate::model::*;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How often an idle paired sender pings the receiver. Between pairing and the
/// manifest the sender may spend minutes scanning folders and waiting on the
/// user; without traffic, routers / Windows / power-managed adapters drop the
/// idle connection and the receiver never gets the transfer.
pub const HEARTBEAT: Duration = Duration::from_secs(10);


/* ---------------- link detection ---------------- */

/// Classify an adapter from its friendly name + hardware description.
/// Returns (named_cable, kind).
fn classify(hay: &str, link_local: bool) -> (bool, &'static str) {
    let named_cable = hay.contains("thunderbolt")
        || hay.contains("usb4")
        || hay.contains("usb 4")
        || hay.contains("p2p") // "USB4(TM) P2P Networking Adapter"
        || hay.contains("bridge");
    let kind = if hay.contains("thunderbolt") {
        "thunderbolt"
    } else if hay.contains("usb4") || hay.contains("usb 4") {
        "usb4"
    } else if link_local || named_cable {
        "other"
    } else {
        "network"
    };
    (named_cable, kind)
}

/// Every adapter with a classification, for detection + UI.
///
/// Windows enumerates via `ipconfig::get_adapters()` (GetAdaptersAddresses), NOT
/// if-addrs: if-addrs only yields adapters that already hold an IPv4 address,
/// and the direct-cable adapter (Thunderbolt/USB4 P2P) spends its first
/// ~5–30 s after link-up IPv6-only while Windows' DHCP discovery times out and
/// APIPA kicks in — during which the cable was invisible to us even though
/// Network Connections showed it "up". A named cable adapter with no IPv4 yet
/// is reported with `ip: ""` so the UI can show "cable detected, waiting for
/// address" instead of nothing.
#[cfg(windows)]
pub fn list_adapters() -> Vec<AdapterInfo> {
    let mut out: Vec<AdapterInfo> = Vec::new();
    let adapters = match ipconfig::get_adapters() {
        Ok(v) => v,
        Err(e) => {
            log::error!("GetAdaptersAddresses failed — no adapters visible: {e}");
            return out;
        }
    };
    for a in adapters {
        if a.oper_status() != ipconfig::OperStatus::IfOperStatusUp {
            continue;
        }
        let desc = a.description().to_string();
        let friendly = a.friendly_name().to_string();
        let hay = format!("{} {}", friendly, desc).to_lowercase();
        if hay.contains("loopback") {
            continue;
        }
        // first IPv4, preferring APIPA 169.254/16 — the direct-cable signature
        // (no DHCP on the cable, so Windows self-assigns)
        let mut v4: Option<Ipv4Addr> = None;
        for ip in a.ip_addresses() {
            if let std::net::IpAddr::V4(x) = ip {
                if x.is_loopback() {
                    continue;
                }
                let apipa = x.octets()[0] == 169 && x.octets()[1] == 254;
                if apipa {
                    v4 = Some(*x);
                    break;
                }
                if v4.is_none() {
                    v4 = Some(*x);
                }
            }
        }
        let link_local = matches!(v4, Some(ip) if ip.octets()[0] == 169 && ip.octets()[1] == 254);
        let (named_cable, kind) = classify(&hay, link_local);
        if v4.is_none() && !named_cable {
            continue; // IPv4-less ordinary adapters are noise
        }
        let name = if desc.is_empty() || desc == friendly {
            friendly
        } else {
            format!("{} — {}", friendly, desc)
        };
        out.push(AdapterInfo {
            name,
            ip: v4.map(|x| x.to_string()).unwrap_or_default(),
            link_local,
            cable: link_local || named_cable,
            kind: kind.into(),
        });
    }
    out
}

/// Non-Windows fallback (dev/mock only): if-addrs, IPv4 adapters.
#[cfg(not(windows))]
pub fn list_adapters() -> Vec<AdapterInfo> {
    let mut out: Vec<AdapterInfo> = Vec::new();
    let ifaces = match if_addrs::get_if_addrs() {
        Ok(v) => v,
        Err(_) => return out,
    };
    for i in ifaces {
        if i.is_loopback() {
            continue;
        }
        let ip = match i.ip() {
            std::net::IpAddr::V4(v4) => v4,
            _ => continue,
        };
        let hay = i.name.to_lowercase();
        let link_local = ip.octets()[0] == 169 && ip.octets()[1] == 254;
        let (named_cable, kind) = classify(&hay, link_local);
        out.push(AdapterInfo {
            name: i.name.clone(),
            ip: ip.to_string(),
            link_local,
            cable: link_local || named_cable,
            kind: kind.into(),
        });
    }
    out
}

/// Pick the best direct-cable adapter. Priority: link-local (169.254) APIPA,
/// then an adapter named like Thunderbolt/USB4/bridge. Wi-Fi/LAN never counts as
/// "up" — a real network address must not be mistaken for the transfer cable.
pub fn link_status() -> LinkStatus {
    let adapters = list_adapters();
    let pick = adapters
        .iter()
        .find(|a| a.link_local)
        .or_else(|| adapters.iter().find(|a| a.cable && !a.ip.is_empty()))
        // cable adapter with no IPv4 yet: not usable (up: false), but surfaced
        // so the UI can show "cable detected, waiting for an address"
        .or_else(|| adapters.iter().find(|a| a.cable));
    match pick {
        Some(a) => LinkStatus {
            up: !a.ip.is_empty(),
            adapter: Some(a.name.clone()),
            local_ip: if a.ip.is_empty() { None } else { Some(a.ip.clone()) },
            kind: Some(a.kind.clone()),
        },
        None => LinkStatus::default(),
    }
}

/// The IP to bind/advertise: cable first, else any usable address (for manual mode).
fn local_ip() -> Option<Ipv4Addr> {
    let adapters = list_adapters();
    let chosen = adapters
        .iter()
        .find(|a| a.link_local)
        .or_else(|| adapters.iter().find(|a| a.cable && !a.ip.is_empty()))
        .or_else(|| adapters.iter().find(|a| !a.ip.is_empty()));
    chosen.and_then(|a| a.ip.parse().ok())
}

/* ---------------- discovery ---------------- */

/// RECEIVER: broadcast a beacon so the sender can find us. Runs until `stop`.
///
/// On a machine that's on Wi-Fi *and* the cable, a plain 255.255.255.255 broadcast
/// egresses the default route (Wi-Fi), so the sender would learn our Wi-Fi IP and
/// transfer over Wi-Fi. To keep it on the cable, we bind the socket to the cable's
/// link-local IP and send to the link-local directed broadcast 169.254.255.255.
pub fn run_beacon(name: String, tcp_port: u16, stop: Arc<AtomicBool>) {
    let cable = local_ip();
    let is_ll = cable.map(|i| i.octets()[0] == 169 && i.octets()[1] == 254).unwrap_or(false);
    let bind_ip = cable.unwrap_or(Ipv4Addr::UNSPECIFIED);

    let sock = match UdpSocket::bind((bind_ip, 0)).or_else(|_| UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))) {
        Ok(s) => s,
        Err(e) => {
            log::error!("beacon: could not bind a UDP socket on {bind_ip}: {e} — the sender will never find this PC");
            return;
        }
    };
    if let Err(e) = sock.set_broadcast(true) {
        log::warn!("beacon: SO_BROADCAST rejected: {e}");
    }

    let bcast = if is_ll {
        Ipv4Addr::new(169, 254, 255, 255) // stays on the cable subnet
    } else {
        Ipv4Addr::BROADCAST // Wi-Fi/LAN fallback when there's no cable
    };
    let beacon = Beacon {
        magic: PROTO_MAGIC.into(),
        name,
        ip: cable.map(|i| i.to_string()).unwrap_or_default(),
        port: tcp_port,
    };
    let payload = serde_json::to_vec(&beacon).unwrap_or_default();
    let mut targets = vec![SocketAddrV4::new(bcast, DISCOVERY_PORT)];
    // Because the socket is bound to the cable IP, a 255.255.255.255 broadcast
    // still egresses only the cable — a fallback for stacks that drop directed
    // (169.254.255.255) broadcasts. It never leaks to Wi-Fi.
    if is_ll {
        targets.push(SocketAddrV4::new(Ipv4Addr::BROADCAST, DISCOVERY_PORT));
    }
    log::info!(
        "beacon: advertising '{}' at {}:{} from {} to {}{}",
        beacon.name,
        if beacon.ip.is_empty() { "(no cable IP)" } else { &beacon.ip },
        tcp_port,
        bind_ip,
        bcast,
        if is_ll { " (+255.255.255.255)" } else { " — no cable, using the LAN broadcast" }
    );

    // A send error every tick means the beacon is going nowhere; log the first
    // one and then only on change, so a dead beacon is obvious without flooding.
    let mut ticks: u64 = 0;
    let mut last_err: Option<String> = None;
    while !stop.load(Ordering::Relaxed) {
        for t in &targets {
            if let Err(e) = sock.send_to(&payload, t) {
                let msg = format!("{t}: {e}");
                if last_err.as_deref() != Some(msg.as_str()) {
                    log::error!("beacon: send failed — {msg}");
                    last_err = Some(msg);
                }
            }
        }
        ticks += 1;
        if ticks % 25 == 0 {
            log::debug!("beacon: still advertising ({}s)", ticks * 8 / 10);
        }
        std::thread::sleep(Duration::from_millis(800));
    }
    log::info!("beacon: stopped after {ticks} tick(s)");
}

/// SENDER: listen for one receiver beacon. Returns the peer or None on timeout.
pub fn discover(timeout: Duration) -> Option<Peer> {
    let sock = match UdpSocket::bind((Ipv4Addr::UNSPECIFIED, DISCOVERY_PORT)) {
        Ok(s) => s,
        Err(e) => {
            log::error!(
                "discovery: could not bind UDP {DISCOVERY_PORT}: {e} — another copy of WINC running, or the port is in use"
            );
            return None;
        }
    };
    sock.set_read_timeout(Some(Duration::from_millis(500))).ok()?;
    let deadline = Instant::now() + timeout;
    let mut buf = [0u8; 1024];
    while Instant::now() < deadline {
        match sock.recv_from(&mut buf) {
            Ok((n, SocketAddr::V4(src))) => {
                log::debug!("discovery: {n} byte(s) from {src}");
                if let Ok(b) = serde_json::from_slice::<Beacon>(&buf[..n]) {
                    if b.magic == PROTO_MAGIC {
                        // prefer the cable IP the receiver advertised; fall back to
                        // the packet source only if the beacon didn't carry one.
                        let ip = if b.ip.is_empty() { src.ip().to_string() } else { b.ip };
                        return Some(Peer {
                            name: b.name,
                            ip,
                            port: b.port,
                        });
                    }
                    log::warn!("discovery: ignoring packet from {src} — wrong magic '{}'", b.magic);
                }
            }
            Ok(_) => {}
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut => {}
            Err(e) => {
                log::error!("discovery: socket read failed, giving up: {e}");
                break;
            }
        }
    }
    None
}

/// Socket send/receive buffers. Windows' defaults leave a fast sender idle
/// waiting for ACKs between writes of 1 MB frames; a few MB keeps the pipe full.
const SOCK_BUF: usize = 4 * 1024 * 1024;

/// Per-connection socket settings for bulk transfer.
fn tune(stream: &TcpStream) {
    stream.set_nodelay(true).ok();
    let s = socket2::SockRef::from(stream);
    if let Err(e) = s.set_send_buffer_size(SOCK_BUF).and_then(|_| s.set_recv_buffer_size(SOCK_BUF)) {
        log::warn!("could not size socket buffers: {e}");
    }
}

/* ---------------- sender side ---------------- */

/// Connect to the receiver, run the encrypted (SPAKE2) handshake, and prove the
/// code. Keeps the connection open (returned) for the later file stream.
pub fn connect_and_pair(peer: &Peer, my_name: &str, code: &str) -> io::Result<EncryptedStream> {
    let addr: SocketAddr = format!("{}:{}", peer.ip, peer.port)
        .parse()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "bad peer address"))?;
    log::info!("connecting to {addr} (8s timeout)");
    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(8)).inspect_err(|e| {
        log::error!(
            "connect to {addr} failed: {e} — receiver not listening, wrong IP, or inbound TCP blocked on the other PC"
        );
    })?;
    tune(&stream);
    log::debug!("TCP up to {addr}; running SPAKE2 handshake");
    let mut enc = crypto::pake_handshake(&stream, code, true).inspect_err(|e| {
        log::error!("handshake with {addr} failed: {e}");
    })?;

    enc.write_json(&Hello {
        magic: PROTO_MAGIC.into(),
        name: my_name.to_string(),
    })?;
    // On a wrong code the receiver can't decrypt our Hello and drops the
    // connection, so on Windows this read fails with a reset (os error 10054)
    // rather than a tidy refusal — surface it as "wrong code" either way.
    let ack: HelloAck = enc.read_json().map_err(|e| {
        log::warn!("no usable ack from {addr} ({e}) — treating as a wrong pairing code");
        io::Error::new(io::ErrorKind::PermissionDenied, "wrong code")
    })?;
    if !ack.ok {
        log::warn!("receiver at {addr} rejected the pairing code");
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "wrong code"));
    }
    log::info!("handshake with {addr} complete");
    Ok(enc)
}

/// Keep-alive while paired but not yet sending: an empty frame. The receiver
/// skips empty frames until the opening message arrives.
pub fn heartbeat(stream: &mut EncryptedStream) -> io::Result<()> {
    stream.write_frame(&[])?;
    stream.flush()
}

/* ---------------- receiver side ---------------- */

/// Accept one sender, verify the code, return the connection + peer identity.
/// Polls so a `cancel` (Start over) — or `deadline`, while waiting for a resume —
/// can break out of the wait instead of blocking forever.
pub fn accept_and_verify(
    listener: &TcpListener,
    code: &str,
    cancel: &AtomicBool,
    deadline: Option<Instant>,
) -> io::Result<(EncryptedStream, Peer)> {
    listener.set_nonblocking(true)?;
    let (stream, addr) = loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        if deadline.is_some_and(|d| Instant::now() >= d) {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "no sender connected in time"));
        }
        match listener.accept() {
            Ok(pair) => break pair,
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(120));
                continue;
            }
            Err(e) => {
                log::error!("accept failed: {e}");
                return Err(e);
            }
        }
    };
    log::info!("inbound connection from {addr}");
    stream.set_nonblocking(false)?; // blocking again for the transfer
    tune(&stream);
    let mut enc = crypto::pake_handshake(&stream, code, false).inspect_err(|e| {
        log::error!("handshake with {addr} failed: {e}");
    })?;

    // A decrypt failure here means the sender used a different code — one
    // wrong online guess kills the connection, which is the rate limit. The
    // short sleep is cheap defense-in-depth against rapid retries.
    let hello: Hello = match enc.read_json() {
        Ok(h) => h,
        Err(e) => {
            log::warn!("could not decrypt hello from {addr} ({e}) — wrong pairing code; dropping the connection");
            if e.kind() == io::ErrorKind::PermissionDenied {
                std::thread::sleep(Duration::from_millis(400));
            }
            return Err(e);
        }
    };
    if hello.magic != PROTO_MAGIC {
        if hello.magic.starts_with("WINC") {
            log::error!(
                "{addr} runs a different WINC version (protocol '{}', this PC '{PROTO_MAGIC}') — install the same WINC version on both PCs",
                hello.magic
            );
        } else {
            log::error!("{addr} is not a WINC peer (magic '{}')", hello.magic);
        }
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not a WINC peer"));
    }
    enc.write_json(&HelloAck { ok: true })?;
    let ip = match addr {
        SocketAddr::V4(v4) => v4.ip().to_string(),
        SocketAddr::V6(v6) => v6.ip().to_string(),
    };
    Ok((
        enc,
        Peer {
            name: hello.name,
            ip,
            port: addr.port(),
        },
    ))
}

/// Join a relative path under `dest`, refusing any component that would escape it.
pub(crate) fn safe_join(dest: &Path, rel: &str) -> PathBuf {
    let mut p = dest.to_path_buf();
    for comp in rel.split(['/', '\\']) {
        if comp.is_empty() || comp == "." || comp == ".." {
            continue;
        }
        p.push(comp);
    }
    p
}

/// Bind the TCP listener. Prefer the fixed TRANSFER_PORT so a person entering an
/// IP by hand never needs a port; fall back to an ephemeral port if it's taken.
pub fn bind_listener() -> io::Result<(TcpListener, u16)> {
    let l = TcpListener::bind((Ipv4Addr::UNSPECIFIED, TRANSFER_PORT)).or_else(|e| {
        log::warn!("TCP {TRANSFER_PORT} unavailable ({e}) — falling back to an ephemeral port");
        TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0))
    })?;
    let port = l.local_addr()?.port();
    log::info!("listening on 0.0.0.0:{port}");
    Ok((l, port))
}

/// Best-effort local name to advertise.
pub fn host_name(fallback: &str) -> String {
    std::env::var("COMPUTERNAME")
        .ok()
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| fallback.to_string())
}

/// The IP we advertise to the peer (for display only).
pub fn advertised_ip() -> String {
    local_ip().map(|i| i.to_string()).unwrap_or_else(|| "0.0.0.0".into())
}

/// Every usable IPv4 on this PC, direct-cable link first — so a person doing
/// manual IP entry can read out the right one.
pub fn local_ipv4s() -> Vec<String> {
    let mut adapters = list_adapters();
    // cable/link-local first, then the rest, preserving order
    adapters.sort_by_key(|a| if a.link_local { 0 } else if a.cable { 1 } else { 2 });
    let mut out: Vec<String> = Vec::new();
    for a in adapters {
        if !a.ip.is_empty() && !out.contains(&a.ip) {
            out.push(a.ip);
        }
    }
    out
}

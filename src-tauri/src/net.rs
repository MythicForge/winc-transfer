use crate::model::*;
use crate::sources::Item;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const BUF: usize = 256 * 1024;

/* ---------------- link detection ---------------- */

/// Every non-loopback IPv4 adapter with a classification, for detection + UI.
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
            _ => continue, // IPv6 entries are listed separately by if-addrs; skip
        };
        let lname = i.name.to_lowercase();
        // Windows "Direct Cable Networking" (Thunderbolt/USB4 Net) has no DHCP, so
        // it self-assigns an APIPA 169.254/16 address — that is the strongest signal.
        let link_local = ip.octets()[0] == 169 && ip.octets()[1] == 254;
        let named_cable = lname.contains("thunderbolt")
            || lname.contains("usb4")
            || lname.contains("usb 4")
            || lname.contains("bridge");
        let kind = if lname.contains("thunderbolt") {
            "thunderbolt"
        } else if lname.contains("usb4") || lname.contains("usb 4") {
            "usb4"
        } else if link_local || named_cable {
            "other"
        } else {
            "network"
        };
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
        .or_else(|| adapters.iter().find(|a| a.cable));
    match pick {
        Some(a) => LinkStatus {
            up: true,
            adapter: Some(a.name.clone()),
            local_ip: Some(a.ip.clone()),
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
        .or_else(|| adapters.iter().find(|a| a.cable))
        .or_else(|| adapters.first());
    chosen.and_then(|a| a.ip.parse().ok())
}

/* ---------------- discovery ---------------- */

/// RECEIVER: broadcast a beacon so the sender can find us. Runs until `stop`.
pub fn run_beacon(name: String, tcp_port: u16, stop: Arc<AtomicBool>) {
    let sock = match UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)) {
        Ok(s) => s,
        Err(_) => return,
    };
    let _ = sock.set_broadcast(true);
    let beacon = Beacon {
        magic: PROTO_MAGIC.into(),
        name,
        port: tcp_port,
    };
    let payload = serde_json::to_vec(&beacon).unwrap_or_default();
    let target = SocketAddrV4::new(Ipv4Addr::BROADCAST, DISCOVERY_PORT);
    while !stop.load(Ordering::Relaxed) {
        let _ = sock.send_to(&payload, target);
        std::thread::sleep(Duration::from_millis(800));
    }
}

/// SENDER: listen for one receiver beacon. Returns the peer or None on timeout.
pub fn discover(timeout: Duration) -> Option<Peer> {
    let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, DISCOVERY_PORT)).ok()?;
    sock.set_read_timeout(Some(Duration::from_millis(500))).ok()?;
    let deadline = Instant::now() + timeout;
    let mut buf = [0u8; 1024];
    while Instant::now() < deadline {
        match sock.recv_from(&mut buf) {
            Ok((n, SocketAddr::V4(src))) => {
                if let Ok(b) = serde_json::from_slice::<Beacon>(&buf[..n]) {
                    if b.magic == PROTO_MAGIC {
                        return Some(Peer {
                            name: b.name,
                            ip: src.ip().to_string(),
                            port: b.port,
                        });
                    }
                }
            }
            Ok(_) => {}
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut => {}
            Err(_) => break,
        }
    }
    None
}

/* ---------------- framing ---------------- */

fn write_msg<W: Write, T: Serialize>(w: &mut W, v: &T) -> io::Result<()> {
    let mut s = serde_json::to_string(v)?;
    s.push('\n');
    w.write_all(s.as_bytes())?;
    w.flush()
}

fn read_msg<R: BufRead, T: DeserializeOwned>(r: &mut R) -> io::Result<T> {
    let mut line = String::new();
    if r.read_line(&mut line)? == 0 {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "peer closed"));
    }
    serde_json::from_str(line.trim_end())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/* ---------------- sender side ---------------- */

/// Connect to the receiver and complete the code handshake. Keeps the
/// connection open (returned) for the later file stream.
pub fn connect_and_pair(peer: &Peer, my_name: &str, code: &str) -> io::Result<TcpStream> {
    let addr: SocketAddr = format!("{}:{}", peer.ip, peer.port)
        .parse()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "bad peer address"))?;
    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(8))?;
    stream.set_nodelay(true).ok();
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream.try_clone()?);

    write_msg(
        &mut writer,
        &Hello {
            magic: PROTO_MAGIC.into(),
            name: my_name.to_string(),
            code: code.to_string(),
        },
    )?;
    let ack: HelloAck = read_msg(&mut reader)?;
    if !ack.ok {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "wrong code"));
    }
    Ok(stream)
}

/// Stream the selected items over an already-paired connection.
pub fn send_files<F: Fn(TransferProgress)>(
    stream: &TcpStream,
    items: &[Item],
    cancel: &Arc<AtomicBool>,
    emit: F,
) -> io::Result<()> {
    let entries: Vec<FileEntry> = items
        .iter()
        .map(|i| FileEntry {
            rel: i.rel.clone(),
            size: std::fs::metadata(&i.abs).map(|m| m.len()).unwrap_or(0),
        })
        .collect();
    let total_bytes: u64 = entries.iter().map(|e| e.size).sum();
    let total_files = entries.len() as u64;

    let mut writer = stream.try_clone()?;
    write_msg(
        &mut writer,
        &Manifest {
            files: entries.clone(),
            total_bytes,
            total_files,
        },
    )?;

    let mut sent: u64 = 0;
    let mut buf = vec![0u8; BUF];
    let start = Instant::now();
    let mut last_emit = Instant::now() - Duration::from_secs(1);

    for (idx, item) in items.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            emit(TransferProgress::error("Cancelled"));
            return Ok(());
        }
        let mut f = match std::fs::File::open(&item.abs) {
            Ok(f) => f,
            Err(_) => continue, // locked/removed file: skip, keep the manifest count honest below
        };
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            writer.write_all(&buf[..n])?;
            sent += n as u64;
            if last_emit.elapsed() >= Duration::from_millis(150) {
                let secs = start.elapsed().as_secs_f64().max(0.001);
                emit(TransferProgress::running(
                    sent,
                    total_bytes,
                    idx as u64,
                    total_files,
                    sent as f64 / secs,
                    &item.rel,
                ));
                last_emit = Instant::now();
            }
        }
    }
    writer.flush()?;
    emit(TransferProgress::done(total_bytes, total_files));
    Ok(())
}

/* ---------------- receiver side ---------------- */

/// Accept one sender, verify the code, return the connection + peer identity.
pub fn accept_and_verify(listener: &TcpListener, code: &str) -> io::Result<(TcpStream, Peer)> {
    let (stream, addr) = listener.accept()?;
    stream.set_nodelay(true).ok();
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream.try_clone()?);

    let hello: Hello = read_msg(&mut reader)?;
    let ok = hello.magic == PROTO_MAGIC && hello.code == code;
    write_msg(&mut writer, &HelloAck { ok })?;
    if !ok {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "code mismatch"));
    }
    let ip = match addr {
        SocketAddr::V4(v4) => v4.ip().to_string(),
        SocketAddr::V6(v6) => v6.ip().to_string(),
    };
    Ok((
        stream,
        Peer {
            name: hello.name,
            ip,
            port: addr.port(),
        },
    ))
}

/// Read the manifest + file stream into `dest`, emitting progress.
pub fn receive_files<F: Fn(TransferProgress)>(
    stream: &TcpStream,
    dest: &Path,
    cancel: &Arc<AtomicBool>,
    emit: F,
) -> io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let manifest: Manifest = read_msg(&mut reader)?;

    let mut got: u64 = 0;
    let mut buf = vec![0u8; BUF];
    let start = Instant::now();
    let mut last_emit = Instant::now() - Duration::from_secs(1);

    for (idx, entry) in manifest.files.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            emit(TransferProgress::error("Cancelled"));
            return Ok(());
        }
        let out_path = safe_join(dest, &entry.rel);
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(&out_path)?;
        let mut remaining = entry.size;
        while remaining > 0 {
            let want = remaining.min(buf.len() as u64) as usize;
            let n = reader.read(&mut buf[..want])?;
            if n == 0 {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "stream ended early"));
            }
            out.write_all(&buf[..n])?;
            remaining -= n as u64;
            got += n as u64;
            if last_emit.elapsed() >= Duration::from_millis(150) {
                let secs = start.elapsed().as_secs_f64().max(0.001);
                emit(TransferProgress::running(
                    got,
                    manifest.total_bytes,
                    idx as u64,
                    manifest.total_files,
                    got as f64 / secs,
                    &entry.rel,
                ));
                last_emit = Instant::now();
            }
        }
    }
    emit(TransferProgress::done(manifest.total_bytes, manifest.total_files));
    Ok(())
}

/// Join a relative path under `dest`, refusing any component that would escape it.
fn safe_join(dest: &Path, rel: &str) -> PathBuf {
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
    let l = TcpListener::bind((Ipv4Addr::UNSPECIFIED, TRANSFER_PORT))
        .or_else(|_| TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)))?;
    let port = l.local_addr()?.port();
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
        if !out.contains(&a.ip) {
            out.push(a.ip);
        }
    }
    out
}

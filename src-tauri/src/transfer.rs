//! The file stream, sender and receiver, with auto-resume.
//!
//! Every file is a run of encrypted frames (one per ≤256 KB chunk) closed by
//! an empty frame. When the link breaks — a frame fails its integrity check
//! (bytes altered on the network path) or the connection drops — the receiver
//! drops the connection and goes back to listening; the sender reconnects with
//! the same pairing code, names the transfer, and the receiver answers with
//! the file to restart from. That file is rewritten whole; everything before
//! it stays. Local faults (disk full, can't create a file) are not resumed.
//!
//! Connection setup is passed in as closures (`reconnect` / `reaccept`), so
//! this module has no Tauri dependency and the tests can break the link on
//! purpose.

use crate::crypto::{EncryptedStream, Opener, Sealer, TAG_LEN};
use crate::model::*;
use crate::net::safe_join;
use crate::sources::Item;
use std::collections::BTreeSet;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Plaintext per data frame. Big enough that per-frame costs (syscalls, tag,
/// channel hop) vanish, small enough that Stop and progress stay snappy.
const BUF: usize = 1024 * 1024;
/// Sender threads opening, reading and sealing files ahead of the socket.
const READERS: usize = 4;
/// Receiver threads opening and writing files behind the socket.
const WRITERS: usize = 4;
/// Frames queued per reader/writer thread — bounds memory in flight to about
/// (READERS + WRITERS) × READ_AHEAD × BUF.
const READ_AHEAD: usize = 8;
/// Manifest entries per frame — ~3 MB of JSON, far under crypto::MAX_FRAME.
const MANIFEST_CHUNK: usize = 20_000;
/// Receiver gives up if nothing at all (not even a heartbeat) arrives for this
/// long before the opening message — the old PC slept, crashed or lost the network.
const PRE_OPENING_WAIT: Duration = Duration::from_secs(120);
/// How long either side keeps trying to get a broken transfer back.
pub const RESUME_WINDOW: Duration = Duration::from_secs(120);
/// Damage that hits the same file this many times in a row is systematic,
/// not a blip — stop instead of looping.
const MAX_SAME_FILE_FAILURES: u32 = 4;
/// Hard cap on resumes per transfer.
const MAX_RESUMES: u32 = 30;
/// Sender: a write blocked this long means the receiver is gone.
const WRITE_STALL: Duration = Duration::from_secs(120);
/// Sender: how long to wait for the receiver's `Finished` after the last frame.
const FINISH_WAIT: Duration = Duration::from_secs(120);
/// Receiver: data normally flows non-stop once the transfer starts. Silence
/// this long means the link is wedged (e.g. a damaged length header has it
/// waiting for bytes that will never come) — treat it as broken and resume.
const RECV_STALL: Duration = Duration::from_secs(90);
/// Receiver, after Stop: how long to wait for the sender's reconnect to tell
/// it so. It comes back within a second of the link closing.
const STOP_NOTICE_WAIT: Duration = Duration::from_secs(6);
/// Receiver, after giving up on a damaged link: same, but the sender may be
/// mid-backoff, so allow longer.
const GIVE_UP_NOTICE_WAIT: Duration = Duration::from_secs(20);

fn is_cancel(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::Interrupted
}

fn cancelled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "cancelled")
}

/// Running byte offsets: `offsets[i]` = bytes in files before `i`, so a resume
/// at file `i` can put the progress bar back where it belongs.
fn offsets(sizes: impl Iterator<Item = u64>) -> Vec<u64> {
    let mut acc = 0u64;
    let mut out = vec![0];
    for s in sizes {
        acc += s;
        out.push(acc);
    }
    out
}

/// Counts resumes and stops the loop when the damage looks systematic.
struct ResumeBudget {
    total: u32,
    last_at: Option<usize>,
    same: u32,
}

impl ResumeBudget {
    fn new() -> Self {
        Self { total: 0, last_at: None, same: 0 }
    }

    /// Record a failure at file `at`; Err when it's time to give up.
    fn spend(&mut self, at: usize, why: &io::Error) -> io::Result<()> {
        self.total += 1;
        self.same = if self.last_at == Some(at) { self.same + 1 } else { 1 };
        self.last_at = Some(at);
        if self.same >= MAX_SAME_FILE_FAILURES || self.total > MAX_RESUMES {
            log::error!(
                "giving up after {} resume(s) ({} in a row on file #{at}): {why}",
                self.total,
                self.same
            );
            return Err(io::Error::new(
                why.kind(),
                format!(
                    "the transfer kept breaking ({} attempts) — {why}. The network path between the PCs is damaging data; try the direct cable",
                    self.total
                ),
            ));
        }
        Ok(())
    }
}

/* ---------------- sender ---------------- */

enum SendFlow {
    Done,
    Cancelled,
}

type Live = Mutex<Option<TcpStream>>;

/// Run `body` alongside a watcher that shuts down the socket registered in
/// `live` the moment `cancel` is set — so Stop lands even while a read or write
/// is blocked on a stalled peer, instead of waiting out WRITE_STALL / RECV_STALL.
fn with_cancel_watch<T>(cancel: &AtomicBool, body: impl FnOnce(&Live) -> T) -> T {
    struct Done<'a>(&'a AtomicBool);
    impl Drop for Done<'_> {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }

    let live: Live = Mutex::new(None);
    let done = AtomicBool::new(false);
    std::thread::scope(|sc| {
        sc.spawn(|| {
            while !done.load(Ordering::Relaxed) {
                if cancel.load(Ordering::Relaxed) {
                    if let Some(s) = live.lock().unwrap().take() {
                        let _ = s.shutdown(Shutdown::Both);
                    }
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        // set even if `body` returns early or panics, so the watcher exits
        let _done = Done(&done);
        body(&live)
    })
}

/// Point the cancel watcher at the connection now in use.
fn watch(live: &Live, stream: &EncryptedStream) {
    *live.lock().unwrap() = stream.socket().ok();
}

/// Close the connection for real. The watcher holds a second handle on the
/// socket, so dropping `stream` alone would leave it open and the peer would
/// never see the break.
fn close(live: &Live, stream: EncryptedStream) {
    if let Some(s) = live.lock().unwrap().take() {
        let _ = s.shutdown(Shutdown::Both);
    }
    drop(stream);
}

/// Stream `items` over `stream`; on any link failure, get a new connection
/// from `reconnect` and resume where the receiver says.
pub fn send<F, R>(
    stream: EncryptedStream,
    items: &[Item],
    cancel: &AtomicBool,
    reconnect: R,
    emit: F,
) -> io::Result<()>
where
    F: Fn(TransferProgress),
    R: FnMut() -> io::Result<EncryptedStream>,
{
    with_cancel_watch(cancel, |live| send_watched(stream, items, cancel, reconnect, emit, live))
}

fn send_watched<F, R>(
    mut stream: EncryptedStream,
    items: &[Item],
    cancel: &AtomicBool,
    mut reconnect: R,
    emit: F,
    live: &Live,
) -> io::Result<()>
where
    F: Fn(TransferProgress),
    R: FnMut() -> io::Result<EncryptedStream>,
{
    watch(live, &stream);
    let entries: Vec<FileEntry> = items
        .iter()
        .map(|i| FileEntry {
            rel: i.rel.clone(),
            size: i.size,
        })
        .collect();
    let total_bytes: u64 = entries.iter().map(|e| e.size).sum();
    let total_files = entries.len() as u64;
    let offs = offsets(entries.iter().map(|e| e.size));
    if items.len() >= BENCH_FILE as usize {
        return Err(io::Error::other("too many files for one transfer — split the selection"));
    }
    let transfer_id: u64 = rand::random();
    let chunks = entries.chunks(MANIFEST_CHUNK);

    log::info!(
        "manifest: {total_files} file(s), {:.2} GB in {} chunk(s)",
        total_bytes as f64 / 1e9,
        chunks.len()
    );
    stream.set_write_timeout(Some(WRITE_STALL))?;
    stream.write_json(&Opening::Start {
        transfer_id,
        total_bytes,
        total_files,
        chunks: chunks.len() as u32,
    })?;
    for chunk in chunks {
        stream.write_json(&chunk)?;
    }

    let start = Instant::now();
    let mut next = 0usize;
    let mut failed: BTreeSet<usize> = BTreeSet::new();
    let mut budget = ResumeBudget::new();
    loop {
        let out = send_from(&mut stream, items, next, &offs, &mut failed, cancel, &emit, start)
            .and_then(|flow| match flow {
                SendFlow::Done => await_finished(&mut stream).map(|_| SendFlow::Done),
                c => Ok(c),
            });
        match out {
            Ok(SendFlow::Done) => break,
            Ok(SendFlow::Cancelled) => {
                close(live, stream);
                return stop_sending(&mut reconnect, transfer_id, &emit);
            }
            // Stop was pressed — the error is the watcher closing the socket
            // under a blocked write, not a break to resume
            Err(_) if cancel.load(Ordering::Relaxed) => {
                close(live, stream);
                return stop_sending(&mut reconnect, transfer_id, &emit);
            }
            Err(e) => {
                log::warn!("link to the receiver broke while sending: {e} — resuming");
                close(live, stream);
                let (s, at) = match resume_sender(&mut reconnect, transfer_id, cancel, &emit, &offs, &budget) {
                    Ok(got) => got,
                    // stopped while reconnecting: the receiver is still waiting for us
                    Err(e) if is_cancel(&e) => return stop_sending(&mut reconnect, transfer_id, &emit),
                    Err(e) => return Err(e),
                };
                budget.spend(at, &e)?;
                watch(live, &s);
                stream = s;
                next = at;
                log::info!("resumed at file #{at} of {total_files}");
            }
        }
    }

    let sent = offs[items.len()];
    log::info!(
        "sent {:.2} GB in {:.1}s ({:.0} MB/s), {} resume(s), {} skipped",
        sent as f64 / 1e9,
        start.elapsed().as_secs_f64(),
        sent as f64 / start.elapsed().as_secs_f64().max(0.001) / 1e6,
        budget.total,
        failed.len()
    );
    if failed.is_empty() {
        emit(TransferProgress::done(total_bytes, total_files));
    } else {
        let names: Vec<&str> = failed.iter().map(|&i| items[i].rel.as_str()).collect();
        log::error!("{} file(s) could not be read: {}", names.len(), names.join(", "));
        emit(TransferProgress::error(&format!(
            "{} file(s) could not be read (cloud/offline or locked): {}",
            names.len(),
            names.join(", ")
        )));
    }
    Ok(())
}

/// One file's worth of output from a reader: sealed frames, then the sealed
/// end-of-file marker (`end: Some(read_ok)`).
struct Piece {
    wire: Vec<u8>,
    plain: usize,
    end: Option<bool>,
}

/// Where a pipelined side spent its time waiting — the bigger number is the
/// bottleneck. Logged once per transfer.
#[derive(Default)]
struct Waits {
    /// sender: on readers (disk + crypto); receiver: on writers (crypto + disk)
    local: Duration,
    /// on the socket
    net: Duration,
}

impl Waits {
    fn log(&self, side: &str, local: &str, wall: Duration) {
        let pct = |d: Duration| 100.0 * d.as_secs_f64() / wall.as_secs_f64().max(0.001);
        log::info!(
            "{side} time split over {:.1}s: waiting on {local} {:.1}s ({:.0}%), on the network {:.1}s ({:.0}%)",
            wall.as_secs_f64(),
            self.local.as_secs_f64(),
            pct(self.local),
            self.net.as_secs_f64(),
            pct(self.net)
        );
    }
}

/// Files `next..` onto the wire. Any error here is a link error — unreadable
/// files are skipped in-band (empty frame), so the receiver stays in step.
///
/// READERS threads open, read and seal files ahead of the socket — file `i`
/// goes to reader `(i - next) % READERS`, so this thread takes them back in
/// manifest order with no reorder buffer. Disk, crypto and network overlap,
/// and the many-small-files case opens READERS files at a time.
#[allow(clippy::too_many_arguments)]
fn send_from<F: Fn(TransferProgress)>(
    stream: &mut EncryptedStream,
    items: &[Item],
    next: usize,
    offs: &[u64],
    failed: &mut BTreeSet<usize>,
    cancel: &AtomicBool,
    emit: &F,
    start: Instant,
) -> io::Result<SendFlow> {
    let sealer = stream.sealer();
    let t0 = Instant::now();
    let mut waits = Waits::default();
    let out = std::thread::scope(|sc| {
        let mut rxs = Vec::with_capacity(READERS);
        for r in 0..READERS {
            let (tx, rx) = mpsc::sync_channel::<Piece>(READ_AHEAD);
            rxs.push(rx);
            let sealer = sealer.clone();
            sc.spawn(move || read_files(items, next + r, &sealer, cancel, tx));
        }
        let out = write_pieces(stream, items, next, offs, failed, cancel, emit, start, &rxs, &mut waits);
        drop(rxs); // readers blocked on a full channel see it closed and exit
        out
    });
    waits.log("send", "disk/encryption", t0.elapsed());
    out
}

/// Reader thread: files `first`, `first + READERS`, … sealed into `tx`.
fn read_files(items: &[Item], first: usize, sealer: &Sealer, cancel: &AtomicBool, tx: SyncSender<Piece>) {
    // one read buffer per thread; each frame is then sized to what was read,
    // so a 40-byte file costs 40 bytes, not a fresh zeroed megabyte
    let mut scratch = vec![0u8; BUF];
    for idx in (first..items.len()).step_by(READERS) {
        let item = &items[idx];
        let file = idx as u32;
        let mut chunk = 0u64;
        let mut ok = true;
        log::trace!("sending {}", item.rel);
        // A file read never yields a 0-byte chunk, so the empty frame is an
        // unambiguous end-of-file marker. That lets an unreadable file (e.g. an
        // un-hydrated OneDrive placeholder failing with os error 362) be skipped
        // without desyncing the receiver, which doesn't rely on manifest sizes.
        match std::fs::File::open(&item.abs) {
            Ok(mut f) => loop {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                let n = match f.read(&mut scratch) {
                    Ok(0) => break,
                    Ok(n) => n,
                    // cloud placeholder / IO fault mid-read: stop this file, keep the stream sane
                    Err(e) => {
                        log::warn!("read failed mid-file, skipping '{}': {e}", item.rel);
                        ok = false;
                        break;
                    }
                };
                let mut buf = Vec::with_capacity(4 + n + TAG_LEN);
                buf.extend_from_slice(&[0; 4]);
                buf.extend_from_slice(&scratch[..n]);
                if sealer.seal(file, chunk, &mut buf).is_err() {
                    ok = false;
                    break;
                }
                chunk += 1;
                if tx.send(Piece { wire: buf, plain: n, end: None }).is_err() {
                    return; // the writer is gone
                }
            },
            // locked/removed/placeholder: skip cleanly
            Err(e) => {
                log::warn!("cannot open, skipping '{}': {e}", item.rel);
                ok = false;
            }
        }
        let mut end = vec![0u8; 4];
        if sealer.seal(file, chunk, &mut end).is_err()
            || tx.send(Piece { wire: end, plain: 0, end: Some(ok) }).is_err()
        {
            return;
        }
    }
}

/// The socket side of `send_from`: pieces back in manifest order, onto the wire.
#[allow(clippy::too_many_arguments)]
fn write_pieces<F: Fn(TransferProgress)>(
    stream: &mut EncryptedStream,
    items: &[Item],
    next: usize,
    offs: &[u64],
    failed: &mut BTreeSet<usize>,
    cancel: &AtomicBool,
    emit: &F,
    start: Instant,
    rxs: &[Receiver<Piece>],
    waits: &mut Waits,
) -> io::Result<SendFlow> {
    let total_bytes = offs[items.len()];
    let total_files = items.len() as u64;
    let mut sent = offs[next];
    let mut last_emit = Instant::now() - Duration::from_secs(1);

    for idx in next..items.len() {
        let rx = &rxs[(idx - next) % READERS];
        loop {
            // per frame, not per file: a multi-GB file must not hold up Stop
            if cancel.load(Ordering::Relaxed) {
                return Ok(SendFlow::Cancelled);
            }
            let t = Instant::now();
            let Ok(piece) = rx.recv() else {
                // readers only quit early on cancel
                return Ok(SendFlow::Cancelled);
            };
            waits.local += t.elapsed();
            let t = Instant::now();
            stream.write_raw(&piece.wire)?;
            waits.net += t.elapsed();
            if let Some(ok) = piece.end {
                if ok {
                    failed.remove(&idx);
                } else {
                    failed.insert(idx);
                }
                break;
            }
            sent += piece.plain as u64;
            if last_emit.elapsed() >= Duration::from_millis(150) {
                let secs = start.elapsed().as_secs_f64().max(0.001);
                emit(TransferProgress::running(
                    sent,
                    total_bytes,
                    idx as u64,
                    total_files,
                    sent as f64 / secs,
                    &items[idx].rel,
                ));
                last_emit = Instant::now();
            }
        }
    }
    let t = Instant::now();
    stream.flush()?;
    waits.net += t.elapsed();
    Ok(SendFlow::Done)
}

/// Data-frame file id for speed-test frames: no manifest reaches it, so their
/// nonces never collide with a real transfer's on the same connection.
const BENCH_FILE: u32 = u32::MAX;

/// Link speed test on a paired, idle connection: sealed in-memory frames for
/// about `dur` (READERS threads sealing, as in a real send); the receiver opens
/// and drops them and confirms. No disk on either side, so this is what cable,
/// TCP and encryption can do. `first` is the first unused bench chunk on this
/// connection. Returns (bytes/sec, next unused chunk).
pub fn bench_send(stream: &mut EncryptedStream, first: u64, dur: Duration) -> io::Result<(f64, u64)> {
    stream.set_write_timeout(Some(WRITE_STALL))?;
    stream.write_json(&Opening::Bench { first })?;
    let sealer = stream.sealer();
    let stop = AtomicBool::new(false);
    let t = Instant::now();
    let mut chunk = first;
    std::thread::scope(|sc| -> io::Result<()> {
        let mut rxs = Vec::with_capacity(READERS);
        for r in 0..READERS as u64 {
            let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(READ_AHEAD);
            rxs.push(rx);
            let (sealer, stop) = (sealer.clone(), &stop);
            sc.spawn(move || {
                let mut c = first + r;
                while !stop.load(Ordering::Relaxed) {
                    let mut buf = vec![0u8; 4 + BUF];
                    buf.reserve(TAG_LEN);
                    if sealer.seal(BENCH_FILE, c, &mut buf).is_err() || tx.send(buf).is_err() {
                        return;
                    }
                    c += READERS as u64;
                }
            });
        }
        let out = (|| {
            while t.elapsed() < dur {
                let frame = rxs[((chunk - first) % READERS as u64) as usize]
                    .recv()
                    .map_err(|_| io::Error::other("speed test stalled"))?;
                stream.write_raw(&frame)?;
                chunk += 1;
            }
            Ok(())
        })();
        stop.store(true, Ordering::Relaxed);
        drop(rxs);
        out
    })?;
    let mut end = vec![0u8; 4];
    sealer.seal(BENCH_FILE, chunk, &mut end)?;
    stream.write_raw(&end)?;
    stream.flush()?;
    await_finished(stream)?;
    stream.set_read_timeout(None)?;
    let bytes = (chunk - first) * BUF as u64;
    Ok((bytes as f64 / t.elapsed().as_secs_f64().max(0.001), chunk + 1))
}

/// Receiver side of `bench_send`: open (to include decryption) and drop.
fn bench_recv(stream: &mut EncryptedStream, first: u64) -> io::Result<()> {
    stream.set_read_timeout(Some(RECV_STALL))?;
    let opener = stream.opener();
    let t = Instant::now();
    let mut bytes = 0u64;
    let damaged = AtomicBool::new(false);
    std::thread::scope(|sc| -> io::Result<()> {
        let mut txs = Vec::with_capacity(WRITERS);
        for _ in 0..WRITERS {
            let (tx, rx) = mpsc::sync_channel::<(u64, Vec<u8>)>(READ_AHEAD);
            txs.push(tx);
            let (opener, damaged) = (opener.clone(), &damaged);
            sc.spawn(move || {
                for (c, mut body) in rx {
                    if opener.open(BENCH_FILE, c, &mut body).is_err() {
                        damaged.store(true, Ordering::Relaxed);
                    }
                }
            });
        }
        let mut c = first;
        loop {
            let mut body = Vec::new();
            stream.read_raw(&mut body)?;
            let end = body.len() == TAG_LEN;
            bytes += (body.len() - TAG_LEN) as u64;
            let _ = txs[((c - first) % WRITERS as u64) as usize].send((c, body));
            c += 1;
            if end {
                return Ok(());
            }
        }
    })?;
    if damaged.load(Ordering::Relaxed) {
        log::error!("speed test: data arrived damaged — the network path is altering bytes");
    }
    stream.write_json(&Finished { ok: !damaged.load(Ordering::Relaxed) })?;
    log::info!(
        "speed test: received {:.2} GB in {:.1}s ({:.0} MB/s)",
        bytes as f64 / 1e9,
        t.elapsed().as_secs_f64(),
        bytes as f64 / t.elapsed().as_secs_f64().max(0.001) / 1e6
    );
    Ok(())
}

/// Stop pressed on this PC. The old link is already closed, so the receiver
/// is about to wait for a resume — reconnect once and say it was a stop, so it
/// ends now instead of waiting out RESUME_WINDOW.
fn stop_sending<F, R>(reconnect: &mut R, transfer_id: u64, emit: &F) -> io::Result<()>
where
    F: Fn(TransferProgress),
    R: FnMut() -> io::Result<EncryptedStream>,
{
    log::warn!("send cancelled — telling the receiver");
    emit(TransferProgress::error("Cancelled"));
    if let Err(e) = reconnect().and_then(|mut s| {
        s.set_write_timeout(Some(Duration::from_secs(5)))?;
        s.write_json(&Opening::Cancel { transfer_id })
    }) {
        log::warn!("could not tell the receiver about the stop: {e}");
    }
    Ok(())
}

/// The receiver confirms the last file landed intact.
fn await_finished(stream: &mut EncryptedStream) -> io::Result<()> {
    stream.set_read_timeout(Some(FINISH_WAIT))?;
    let fin: Finished = stream.read_json()?;
    if !fin.ok {
        return Err(io::Error::new(io::ErrorKind::Other, "receiver did not confirm the transfer"));
    }
    Ok(())
}

/// Reconnect and re-attach to `transfer_id`, retrying with back-off until the
/// receiver answers or RESUME_WINDOW runs out. Returns the new stream and the
/// file index to restart from.
fn resume_sender<F, R>(
    reconnect: &mut R,
    transfer_id: u64,
    cancel: &AtomicBool,
    emit: &F,
    offs: &[u64],
    budget: &ResumeBudget,
) -> io::Result<(EncryptedStream, usize)>
where
    F: Fn(TransferProgress),
    R: FnMut() -> io::Result<EncryptedStream>,
{
    let total_files = (offs.len() - 1) as u64;
    let shown = budget.last_at.unwrap_or(0);
    let deadline = Instant::now() + RESUME_WINDOW;
    let mut attempt = 0u32;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(cancelled());
        }
        if Instant::now() >= deadline {
            log::error!("could not get the transfer back within {}s", RESUME_WINDOW.as_secs());
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "lost the connection to the new PC and couldn't reconnect within 2 minutes",
            ));
        }
        attempt += 1;
        emit(TransferProgress::reconnecting(
            offs[shown],
            offs[offs.len() - 1],
            shown as u64,
            total_files,
            &format!("Connection interrupted — reconnecting (attempt {attempt})…"),
        ));
        // back-off: give the receiver time to return to listening
        let pause = Duration::from_millis(500 * u64::from(attempt.min(8)));
        let until = Instant::now() + pause;
        while Instant::now() < until {
            if cancel.load(Ordering::Relaxed) {
                return Err(cancelled());
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        let tried = reconnect().and_then(|mut s| {
            s.set_write_timeout(Some(WRITE_STALL))?;
            s.write_json(&Opening::Resume { transfer_id })?;
            s.set_read_timeout(Some(Duration::from_secs(30)))?;
            let at: ResumeAt = s.read_json()?;
            s.set_read_timeout(None)?;
            Ok((s, at))
        });
        match tried {
            Ok((_, at)) if at.stopped => {
                log::warn!("the receiver stopped the transfer");
                return Err(io::Error::new(io::ErrorKind::Other, "the transfer was stopped on the new PC"));
            }
            Ok((s, at)) if at.ok && at.next as usize <= total_files as usize => {
                return Ok((s, at.next as usize))
            }
            Ok(_) => {
                log::error!("the receiver refused to resume this transfer");
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "the new PC stopped this transfer (it kept arriving damaged, or was restarted) — see its log",
                ));
            }
            Err(e) => log::warn!("reconnect attempt {attempt} failed: {e}"),
        }
    }
}

/* ---------------- receiver ---------------- */

enum RecvErr {
    /// Disk-side: not resumable.
    Local(io::Error),
    /// Link-side at manifest index `at`: resumable.
    Link { e: io::Error, at: usize },
}

enum RecvFlow {
    Done,
    Cancelled,
}

/// Skip heartbeats until the first real message; give up after a long silence.
fn read_opening(stream: &mut EncryptedStream) -> io::Result<Opening> {
    stream.set_read_timeout(Some(PRE_OPENING_WAIT))?;
    loop {
        let frame = stream.read_frame().map_err(|e| {
            if matches!(e.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock) {
                log::error!(
                    "no word from the sender for {}s before the transfer started",
                    PRE_OPENING_WAIT.as_secs()
                );
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    "lost contact with the old PC before the transfer started (asleep, closed, or network dropped)",
                )
            } else {
                e
            }
        })?;
        if frame.is_empty() {
            log::trace!("sender heartbeat");
            continue;
        }
        stream.set_read_timeout(None)?;
        return serde_json::from_slice(&frame).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e));
    }
}

/// Receive a transfer into `dest`. On link failure, wait (via `reaccept`,
/// given the deadline and the flag that may interrupt it) for the sender to
/// reconnect and resume.
pub fn receive<F, A>(
    stream: EncryptedStream,
    dest: &Path,
    cancel: &AtomicBool,
    reaccept: A,
    emit: F,
) -> io::Result<()>
where
    F: Fn(TransferProgress),
    A: FnMut(Instant, &AtomicBool) -> io::Result<EncryptedStream>,
{
    with_cancel_watch(cancel, |live| receive_watched(stream, dest, cancel, reaccept, emit, live))
}

fn receive_watched<F, A>(
    mut stream: EncryptedStream,
    dest: &Path,
    cancel: &AtomicBool,
    mut reaccept: A,
    emit: F,
    live: &Live,
) -> io::Result<()>
where
    F: Fn(TransferProgress),
    A: FnMut(Instant, &AtomicBool) -> io::Result<EncryptedStream>,
{
    // any number of speed tests may come before the transfer itself
    let (transfer_id, total_bytes, total_files, chunks) = loop {
        match read_opening(&mut stream).inspect_err(|e| log::error!("could not read the manifest: {e}"))? {
            Opening::Start { transfer_id, total_bytes, total_files, chunks } => {
                break (transfer_id, total_bytes, total_files, chunks)
            }
            Opening::Bench { first } => {
                log::info!("speed test requested by the sender");
                bench_recv(&mut stream, first).inspect_err(|e| log::error!("speed test failed: {e}"))?;
            }
            Opening::Resume { .. } | Opening::Cancel { .. } => {
                log::error!("sender referred to a transfer this receiver never started");
                return Err(io::Error::new(io::ErrorKind::InvalidData, "unexpected resume"));
            }
        }
    };
    let mut files: Vec<FileEntry> = Vec::with_capacity(total_files as usize);
    for _ in 0..chunks {
        let part: Vec<FileEntry> = stream
            .read_json()
            .inspect_err(|e| log::error!("could not read the manifest: {e}"))?;
        files.extend(part);
    }
    // Fail now, not 90% of the way in, if the drop folder's drive can't hold it.
    if let Some(free) = crate::settings::free_space(dest) {
        if free < total_bytes {
            let msg = format!(
                "not enough space on the drive holding {} — need {:.1} GB, {:.1} GB free. Pick a bigger drive for incoming data and pair again",
                dest.display(),
                total_bytes as f64 / 1e9,
                free as f64 / 1e9
            );
            log::error!("{msg}");
            return Err(io::Error::new(io::ErrorKind::Other, msg));
        }
    }
    log::info!(
        "manifest received: {} file(s), {:.2} GB",
        total_files,
        total_bytes as f64 / 1e9
    );

    let offs = offsets(files.iter().map(|f| f.size));
    let start = Instant::now();
    let mut next = 0usize;
    let mut budget = ResumeBudget::new();
    watch(live, &stream);
    loop {
        stream.set_read_timeout(Some(RECV_STALL))?;
        match recv_from(&mut stream, dest, &files, next, &offs, cancel, &emit, start) {
            Ok(RecvFlow::Done) => break,
            Ok(RecvFlow::Cancelled) => {
                close(live, stream);
                return stop_receiving(&mut reaccept, transfer_id, &emit);
            }
            Err(RecvErr::Local(e)) => return Err(e),
            // Stop was pressed — the error is the watcher closing the socket
            // under a blocked read, not a break to wait out
            Err(RecvErr::Link { .. }) if cancel.load(Ordering::Relaxed) => {
                close(live, stream);
                return stop_receiving(&mut reaccept, transfer_id, &emit);
            }
            Err(RecvErr::Link { e, at }) => {
                log::warn!("link to the sender broke at file #{at}: {e} — waiting for it to reconnect");
                close(live, stream); // the sender's next write fails fast and it starts reconnecting
                if let Err(give_up) = budget.spend(at, &e) {
                    refuse_one(&mut reaccept, transfer_id, GIVE_UP_NOTICE_WAIT, false);
                    return Err(give_up);
                }
                emit(TransferProgress::reconnecting(
                    offs[at],
                    total_bytes,
                    at as u64,
                    total_files,
                    "Connection interrupted — waiting for the old PC to reconnect…",
                ));
                stream = match resume_receiver(&mut reaccept, transfer_id, at, cancel) {
                    Ok(s) => s,
                    // stopped while waiting: the sender is still trying to get back
                    Err(e) if is_cancel(&e) => return stop_receiving(&mut reaccept, transfer_id, &emit),
                    Err(e) => return Err(e),
                };
                watch(live, &stream);
                next = at;
                log::info!("resumed at file #{at} of {total_files}");
            }
        }
    }

    // Everything is on disk; if this last message is lost the sender will
    // retry for a while and report it, but the data here is complete.
    if let Err(e) = stream.write_json(&Finished { ok: true }) {
        log::warn!("could not confirm completion to the sender: {e}");
    }
    let got = offs[files.len()];
    log::info!(
        "wrote {:.2} GB in {:.1}s ({:.0} MB/s), {} resume(s), to {}",
        got as f64 / 1e9,
        start.elapsed().as_secs_f64(),
        got as f64 / start.elapsed().as_secs_f64().max(0.001) / 1e6,
        budget.total,
        dest.display()
    );
    emit(TransferProgress::done(total_bytes, total_files));
    Ok(())
}

/// Stop pressed on this PC. The old link is already closed, so the sender is
/// about to reconnect to resume — answer that with "stopped" so it ends now
/// instead of retrying for RESUME_WINDOW.
fn stop_receiving<F, A>(reaccept: &mut A, transfer_id: u64, emit: &F) -> io::Result<()>
where
    F: Fn(TransferProgress),
    A: FnMut(Instant, &AtomicBool) -> io::Result<EncryptedStream>,
{
    log::warn!("receive cancelled — telling the sender");
    emit(TransferProgress::error("Cancelled"));
    refuse_one(reaccept, transfer_id, STOP_NOTICE_WAIT, true);
    Ok(())
}

/// Answer the sender's next reconnect with a refusal so it stops at once
/// instead of retrying for the whole resume window. `stopped`: the user ended
/// it here, as opposed to giving up on a damaged link.
fn refuse_one<A>(reaccept: &mut A, transfer_id: u64, wait: Duration, stopped: bool)
where
    A: FnMut(Instant, &AtomicBool) -> io::Result<EncryptedStream>,
{
    let deadline = Instant::now() + wait;
    // a fresh flag: after Stop the session's own flag is set and would end
    // the wait before the sender ever got its answer
    let keep_waiting = AtomicBool::new(false);
    while let Ok(mut s) = reaccept(deadline, &keep_waiting) {
        if let Ok(Opening::Resume { transfer_id: id }) = read_opening(&mut s) {
            if id == transfer_id {
                let _ = s.write_json(&ResumeAt { ok: false, next: 0, stopped });
                return;
            }
        }
    }
}

/// Wait for the sender to come back for `transfer_id` and tell it where to
/// restart. Connections for anything else are turned away.
fn resume_receiver<A>(
    reaccept: &mut A,
    transfer_id: u64,
    at: usize,
    cancel: &AtomicBool,
) -> io::Result<EncryptedStream>
where
    A: FnMut(Instant, &AtomicBool) -> io::Result<EncryptedStream>,
{
    let deadline = Instant::now() + RESUME_WINDOW;
    loop {
        let mut s = match reaccept(deadline, cancel) {
            Ok(s) => s,
            Err(e) if is_cancel(&e) || e.kind() == io::ErrorKind::TimedOut => {
                if e.kind() == io::ErrorKind::TimedOut {
                    log::error!("the sender didn't come back within {}s", RESUME_WINDOW.as_secs());
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "lost the connection to the old PC and it didn't reconnect within 2 minutes",
                    ));
                }
                return Err(e);
            }
            // wrong code / not a WINC peer / handshake hiccup: keep waiting
            Err(e) => {
                log::warn!("reconnect attempt rejected: {e}");
                continue;
            }
        };
        match read_opening(&mut s) {
            Ok(Opening::Resume { transfer_id: id }) if id == transfer_id => {
                s.write_json(&ResumeAt { ok: true, next: at as u64, stopped: false })?;
                return Ok(s);
            }
            Ok(Opening::Cancel { transfer_id: id }) if id == transfer_id => {
                log::warn!("the sender stopped the transfer");
                return Err(io::Error::new(io::ErrorKind::Other, "the transfer was stopped on the old PC"));
            }
            Ok(Opening::Resume { .. }) => {
                log::warn!("a sender tried to resume a different transfer — refused");
                let _ = s.write_json(&ResumeAt { ok: false, next: 0, stopped: false });
            }
            Ok(Opening::Cancel { .. }) => log::warn!("a sender cancelled a different transfer — ignored"),
            Ok(Opening::Bench { .. }) => log::warn!("a sender asked for a speed test mid-resume — refused"),
            Ok(Opening::Start { .. }) => {
                log::warn!("a sender tried to start a new transfer mid-resume — refused");
            }
            Err(e) => log::warn!("reconnect sent nothing usable: {e}"),
        }
    }
}

/// What went wrong on a writer thread, first one wins.
enum WriterErr {
    /// Disk-side: not resumable.
    Local(io::Error),
    /// A frame failed its integrity check — the bytes changed in transit.
    Damaged(io::Error),
}

/// Files `next..` off the wire into `dest`.
///
/// This thread only reads sealed frames off the socket; WRITERS threads open
/// (decrypt) and write them — file `i` goes to writer `(i - next) % WRITERS`,
/// so several small files are created at once (each create pays the
/// antivirus scan on Windows) and decryption is spread over cores.
///
/// On a link error the resume point is the lowest file not yet fully on disk;
/// later files that did finish are simply rewritten.
#[allow(clippy::too_many_arguments)]
fn recv_from<F: Fn(TransferProgress)>(
    stream: &mut EncryptedStream,
    dest: &Path,
    files: &[FileEntry],
    next: usize,
    offs: &[u64],
    cancel: &AtomicBool,
    emit: &F,
    start: Instant,
) -> Result<RecvFlow, RecvErr> {
    let opener = stream.opener();
    let done: Vec<AtomicBool> = (0..files.len()).map(|_| AtomicBool::new(false)).collect();
    let fail: Mutex<Option<WriterErr>> = Mutex::new(None);
    let t0 = Instant::now();
    let mut waits = Waits::default();
    let out = std::thread::scope(|sc| {
        let mut txs = Vec::with_capacity(WRITERS);
        for w in 0..WRITERS {
            let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(READ_AHEAD);
            txs.push(tx);
            let (opener, done, fail) = (opener.clone(), &done, &fail);
            sc.spawn(move || write_files(dest, files, next + w, &opener, rx, done, fail));
        }
        read_frames(stream, files, next, offs, cancel, emit, start, &txs, &mut waits)
        // txs drop here: writers finish what they hold, then exit
    });
    waits.log("receive", "decryption/disk", t0.elapsed());

    // first incomplete file — where a resume picks up
    let at = (next..files.len())
        .find(|&i| !done[i].load(Ordering::Relaxed))
        .unwrap_or(files.len());
    match fail.into_inner().unwrap() {
        Some(WriterErr::Local(e)) => return Err(RecvErr::Local(e)),
        Some(WriterErr::Damaged(e)) => return Err(RecvErr::Link { e, at }),
        None => {}
    }
    match out {
        Ok(RecvFlow::Done) if at < files.len() => Err(RecvErr::Link {
            e: io::Error::other("a file did not finish writing"),
            at,
        }),
        Ok(flow) => Ok(flow),
        Err(e) => Err(RecvErr::Link { e, at }),
    }
}

/// The socket side of `recv_from`: sealed frames, in order, to their writer.
/// Returns Ok(Done) once every frame is handed over; Err on a link error.
#[allow(clippy::too_many_arguments)]
fn read_frames<F: Fn(TransferProgress)>(
    stream: &mut EncryptedStream,
    files: &[FileEntry],
    next: usize,
    offs: &[u64],
    cancel: &AtomicBool,
    emit: &F,
    start: Instant,
    txs: &[SyncSender<Vec<u8>>],
    waits: &mut Waits,
) -> io::Result<RecvFlow> {
    let total_bytes = offs[files.len()];
    let total_files = files.len() as u64;
    let mut got = offs[next];
    let mut last_emit = Instant::now() - Duration::from_secs(1);

    for (idx, entry) in files.iter().enumerate().skip(next) {
        let tx = &txs[(idx - next) % WRITERS];
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Ok(RecvFlow::Cancelled);
            }
            let mut body = Vec::new(); // read_raw sizes it to the frame
            let t = Instant::now();
            stream.read_raw(&mut body)?;
            waits.net += t.elapsed();
            // an empty plaintext seals to exactly a tag: this file's end marker
            let end = body.len() == TAG_LEN;
            let len = body.len() - TAG_LEN;
            let t = Instant::now();
            if tx.send(body).is_err() {
                // the writer stopped on an error it recorded — recv_from reports it
                return Ok(RecvFlow::Done);
            }
            waits.local += t.elapsed();
            if end {
                break;
            }
            got += len as u64;
            if last_emit.elapsed() >= Duration::from_millis(150) {
                let secs = start.elapsed().as_secs_f64().max(0.001);
                emit(TransferProgress::running(
                    got,
                    total_bytes,
                    idx as u64,
                    total_files,
                    got as f64 / secs,
                    &entry.rel,
                ));
                last_emit = Instant::now();
            }
        }
    }
    Ok(RecvFlow::Done)
}

/// Writer thread: files `first`, `first + WRITERS`, … from `rx` onto disk.
/// Stops quietly when `rx` closes (link broke / cancel / done); on its own
/// error, records it in `fail` and stops.
fn write_files(
    dest: &Path,
    files: &[FileEntry],
    first: usize,
    opener: &Opener,
    rx: Receiver<Vec<u8>>,
    done: &[AtomicBool],
    fail: &Mutex<Option<WriterErr>>,
) {
    let record = |e: WriterErr| {
        fail.lock().unwrap().get_or_insert(e);
    };
    for idx in (first..files.len()).step_by(WRITERS) {
        let entry = &files[idx];
        let out_path = safe_join(dest, &entry.rel);
        log::trace!("receiving {}", entry.rel);
        if let Some(parent) = out_path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                log::error!("could not create {}: {e}", parent.display());
                return record(WriterErr::Local(e));
            }
        }
        // create() truncates, so a file that was in flight when the link broke
        // is rewritten whole on resume
        let mut out = match std::fs::File::create(&out_path) {
            Ok(f) => f,
            Err(e) => {
                log::error!("could not create {}: {e}", out_path.display());
                return record(WriterErr::Local(e));
            }
        };
        // reserve the whole file up front: one allocation, less fragmentation
        let _ = out.set_len(entry.size);
        let mut written = 0u64;
        let mut chunk = 0u64;
        loop {
            let Ok(mut body) = rx.recv() else { return };
            if opener.open(idx as u32, chunk, &mut body).is_err() {
                log::error!(
                    "integrity check failed on chunk #{chunk} of '{}' — data was altered in transit",
                    entry.rel
                );
                return record(WriterErr::Damaged(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("data arrived damaged while receiving '{}' (failed the integrity check)", entry.rel),
                )));
            }
            chunk += 1;
            if body.is_empty() {
                break; // end of this file
            }
            if let Err(e) = out.write_all(&body) {
                log::error!("could not write {}: {e}", out_path.display());
                return record(WriterErr::Local(e));
            }
            written += body.len() as u64;
        }
        // skipped or changed on the sender since the scan: trim the reservation
        if written != entry.size {
            if let Err(e) = out.set_len(written) {
                log::error!("could not size {}: {e}", out_path.display());
                return record(WriterErr::Local(e));
            }
        }
        done[idx].store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::{accept_and_verify, connect_and_pair, heartbeat};
    use std::net::{Ipv4Addr, Shutdown, TcpListener, TcpStream};
    use std::path::PathBuf;
    use std::sync::Arc;

    const CODE: &str = "123456";

    /// Deterministic files under a fresh temp dir; `size(i)` picks each size.
    fn make_files(tag: &str, n: usize, size: impl Fn(usize) -> usize) -> (PathBuf, Vec<Item>) {
        let root = std::env::temp_dir().join(format!("winc-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let src = root.join("src");
        std::fs::create_dir_all(&src).unwrap();
        let mut seed = 0x9E37_79B9_7F4A_7C15u64 ^ n as u64;
        let mut items = Vec::with_capacity(n);
        for i in 0..n {
            let len = size(i);
            let mut body = Vec::with_capacity(len);
            while body.len() < len {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                body.extend_from_slice(&seed.to_le_bytes());
            }
            body.truncate(len);
            let p = src.join(format!("f{i}.bin"));
            std::fs::write(&p, &body).unwrap();
            items.push(Item { abs: p, rel: format!("Documents/f{i}.bin"), size: len as u64 });
        }
        (root, items)
    }

    #[derive(Clone, Copy)]
    enum Fault {
        Clean,
        /// flip one byte this far into the sender→receiver stream
        FlipAt(usize),
        /// cut the connection this far in
        CloseAt(usize),
    }

    /// Loopback TCP proxy in front of the receiver; connection `n` (0 = the
    /// pairing connection, then each reconnect) gets `plan(n)`.
    fn proxy(target: u16, plan: impl Fn(usize) -> Fault + Send + 'static) -> u16 {
        let l = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for (n, c) in l.incoming().enumerate() {
                let Ok(c) = c else { return };
                let Ok(up) = TcpStream::connect((Ipv4Addr::LOCALHOST, target)) else { return };
                let fault = plan(n);
                let (mut back_src, mut back_dst) = (up.try_clone().unwrap(), c.try_clone().unwrap());
                std::thread::spawn(move || {
                    let _ = io::copy(&mut back_src, &mut back_dst);
                    let _ = back_dst.shutdown(Shutdown::Both);
                });
                std::thread::spawn(move || {
                    let (mut from, mut to) = (c, up);
                    let mut buf = vec![0u8; 64 * 1024];
                    let mut seen = 0usize;
                    loop {
                        let n = match from.read(&mut buf) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => n,
                        };
                        match fault {
                            Fault::FlipAt(k) if seen <= k && k < seen + n => buf[k - seen] ^= 0xFF,
                            Fault::CloseAt(k) if seen + n > k => break,
                            _ => {}
                        }
                        seen += n;
                        if to.write_all(&buf[..n]).is_err() {
                            break;
                        }
                    }
                    let _ = from.shutdown(Shutdown::Both);
                    let _ = to.shutdown(Shutdown::Both);
                });
            }
        });
        port
    }

    /// Pair through `port` (a proxy or the receiver itself), run a full
    /// transfer both ways round, return (sender result, receiver result, dest).
    fn run(
        tag: &str,
        items: &[Item],
        root: &Path,
        via: impl FnOnce(u16) -> u16,
        idle_pings: usize,
    ) -> (io::Result<()>, io::Result<()>, PathBuf) {
        let dest = root.join(format!("dest-{tag}"));
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = via(listener.local_addr().unwrap().port());
        let cancel = Arc::new(AtomicBool::new(false));
        let (c2, d2) = (cancel.clone(), dest.clone());
        let rx = std::thread::spawn(move || {
            let (s, _) = accept_and_verify(&listener, CODE, &c2, None).unwrap();
            let reaccept = |deadline: Instant, c: &AtomicBool| {
                accept_and_verify(&listener, CODE, c, Some(deadline)).map(|(s, _)| s)
            };
            receive(s, &d2, &c2, reaccept, |_| {})
        });
        let peer = Peer { name: "t".into(), ip: "127.0.0.1".into(), port };
        let mut s = connect_and_pair(&peer, "tester", CODE).unwrap();
        for _ in 0..idle_pings {
            heartbeat(&mut s).unwrap();
        }
        let sent = send(s, items, &cancel, || connect_and_pair(&peer, "tester", CODE), |_| {});
        (sent, rx.join().unwrap(), dest)
    }

    fn assert_same(items: &[Item], dest: &Path) {
        for it in items {
            let a = std::fs::read(&it.abs).unwrap();
            let b = std::fs::read(dest.join(&it.rel)).unwrap_or_else(|_| panic!("missing {}", it.rel));
            assert!(a == b, "content differs: {}", it.rel);
        }
    }

    /// Heartbeats while idle (as during a long scan), then a tree big enough
    /// to need more than one manifest chunk.
    #[test]
    fn heartbeats_then_chunked_manifest() {
        let (root, items) = make_files("chunks", MANIFEST_CHUNK + 500, |i| i % 50);
        let (sent, got, dest) = run("chunks", &items, &root, |p| p, 3);
        sent.unwrap();
        got.unwrap();
        assert_same(&items, &dest);
        std::fs::remove_dir_all(&root).ok();
    }

    /// The link is damaged once and cut once mid-stream; the transfer must
    /// resume each time and still land byte-identical.
    #[test]
    fn resumes_after_damage_and_disconnect() {
        let (root, items) = make_files("resume", 60, |i| 100_000 + (i * 37_013) % 400_000);
        let plan = |n| match n {
            0 => Fault::FlipAt(3_000_000),
            1 => Fault::CloseAt(2_000_000),
            _ => Fault::Clean,
        };
        let (sent, got, dest) = run("resume", &items, &root, |p| proxy(p, plan), 0);
        sent.unwrap();
        got.unwrap();
        assert_same(&items, &dest);
        std::fs::remove_dir_all(&root).ok();
    }

    /// Damage on every connection, always in the same file: both sides must give
    /// up promptly with an error rather than loop or hang.
    #[test]
    fn gives_up_on_systematic_damage() {
        let (root, items) = make_files("giveup", 20, |_| 300_000);
        let t = Instant::now();
        // every reconnect is damaged inside the first file it resends — the
        // same file each time, so no progress is ever made
        let plan = |n| Fault::FlipAt(if n == 0 { 1_000_000 } else { 50_000 });
        let (sent, got, _) = run("giveup", &items, &root, |p| proxy(p, plan), 0);
        assert!(got.is_err(), "receiver should give up");
        assert!(sent.is_err(), "sender should give up");
        assert!(t.elapsed() < Duration::from_secs(60), "took {:?}", t.elapsed());
        std::fs::remove_dir_all(&root).ok();
    }

    /// Two speed tests on the paired connection, then a normal transfer on the
    /// same connection must still land byte-identical.
    #[test]
    fn speed_test_then_transfer() {
        let (root, items) = make_files("bench", 30, |i| 50_000 + i * 70_001);
        let dest = root.join("dest-bench");
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let cancel = Arc::new(AtomicBool::new(false));
        let (c2, d2) = (cancel.clone(), dest.clone());
        let rx = std::thread::spawn(move || {
            let (s, _) = accept_and_verify(&listener, CODE, &c2, None).unwrap();
            let reaccept = |deadline: Instant, c: &AtomicBool| {
                accept_and_verify(&listener, CODE, c, Some(deadline)).map(|(s, _)| s)
            };
            receive(s, &d2, &c2, reaccept, |_| {})
        });
        let peer = Peer { name: "t".into(), ip: "127.0.0.1".into(), port };
        let mut s = connect_and_pair(&peer, "tester", CODE).unwrap();
        let (bps, next) = bench_send(&mut s, 0, Duration::from_millis(300)).unwrap();
        assert!(bps > 0.0 && next > 0);
        let (_, next2) = bench_send(&mut s, next, Duration::from_millis(300)).unwrap();
        assert!(next2 > next);
        send(s, &items, &cancel, || connect_and_pair(&peer, "tester", CODE), |_| {}).unwrap();
        rx.join().unwrap().unwrap();
        assert_same(&items, &dest);
        std::fs::remove_dir_all(&root).ok();
    }

    /// Pair and transfer with a separate Stop flag per side; the side named by
    /// `stop_sender` presses Stop as soon as data is flowing.
    fn run_stopped(tag: &str, items: &[Item], root: &Path, stop_sender: bool) -> (io::Result<()>, io::Result<()>) {
        let dest = root.join(format!("dest-{tag}"));
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx_cancel, rx_cancel) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
        let rc = rx_cancel.clone();
        let rx = std::thread::spawn(move || {
            let (s, _) = accept_and_verify(&listener, CODE, &rc, None).unwrap();
            let reaccept = |deadline: Instant, c: &AtomicBool| {
                accept_and_verify(&listener, CODE, c, Some(deadline)).map(|(s, _)| s)
            };
            let stop = |_| {
                if !stop_sender {
                    rc.store(true, Ordering::Relaxed);
                }
            };
            receive(s, &dest, &rc, reaccept, stop)
        });
        let peer = Peer { name: "t".into(), ip: "127.0.0.1".into(), port };
        let s = connect_and_pair(&peer, "tester", CODE).unwrap();
        let stop = |_| {
            if stop_sender {
                tx_cancel.store(true, Ordering::Relaxed);
            }
        };
        let sent = send(s, items, &tx_cancel, || connect_and_pair(&peer, "tester", CODE), stop);
        (sent, rx.join().unwrap())
    }

    /// Stop on the old PC: the new PC must hear about it and end promptly,
    /// not sit out RESUME_WINDOW waiting for a resume.
    #[test]
    fn sender_stop_reaches_receiver() {
        let (root, items) = make_files("txstop", 40, |_| 2_000_000);
        let t = Instant::now();
        let (sent, got) = run_stopped("txstop", &items, &root, true);
        sent.unwrap();
        let e = got.expect_err("receiver should report the stop");
        assert!(e.to_string().contains("stopped on the old PC"), "{e}");
        assert!(t.elapsed() < Duration::from_secs(15), "took {:?}", t.elapsed());
        std::fs::remove_dir_all(&root).ok();
    }

    /// Stop on the new PC: the old PC must hear about it and end promptly.
    #[test]
    fn receiver_stop_reaches_sender() {
        let (root, items) = make_files("rxstop", 40, |_| 2_000_000);
        let t = Instant::now();
        let (sent, got) = run_stopped("rxstop", &items, &root, false);
        got.unwrap();
        let e = sent.expect_err("sender should report the stop");
        assert!(e.to_string().contains("stopped on the new PC"), "{e}");
        assert!(t.elapsed() < Duration::from_secs(15), "took {:?}", t.elapsed());
        std::fs::remove_dir_all(&root).ok();
    }

    /// Volume: ~30k mostly small files plus some big ones, sizes deliberately
    /// not multiples of BUF. `cargo test --release --lib -- --ignored stress`
    #[test]
    #[ignore]
    fn stress_large_transfer() {
        let (root, items) = make_files("stress", 30_000, |i| match i % 100 {
            0 => 0,
            1..=89 => 200 + (i * 7919) % 60_000,
            90..=97 => BUF - 1 + (i % 3),
            _ => 2_000_000 + (i * 4099) % 3_000_000,
        });
        let total: u64 = items.iter().map(|i| i.size).sum();
        eprintln!("stress: {} files, {:.2} GB", items.len(), total as f64 / 1e9);
        let t = Instant::now();
        let (sent, got, dest) = run("stress", &items, &root, |p| p, 1);
        let secs = t.elapsed().as_secs_f64();
        eprintln!("stress: transferred in {secs:.1}s ({:.0} MB/s)", total as f64 / secs / 1e6);
        sent.unwrap();
        got.unwrap();
        assert_same(&items, &dest);
        std::fs::remove_dir_all(&root).ok();
    }
}

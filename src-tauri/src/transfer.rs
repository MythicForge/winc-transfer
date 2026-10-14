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

use crate::crypto::EncryptedStream;
use crate::model::*;
use crate::net::safe_join;
use crate::sources::Item;
use std::collections::BTreeSet;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const BUF: usize = 256 * 1024;
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

/// Stream `items` over `stream`; on any link failure, get a new connection
/// from `reconnect` and resume where the receiver says.
pub fn send<F, R>(
    mut stream: EncryptedStream,
    items: &[Item],
    cancel: &AtomicBool,
    mut reconnect: R,
    emit: F,
) -> io::Result<()>
where
    F: Fn(TransferProgress),
    R: FnMut() -> io::Result<EncryptedStream>,
{
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
                log::warn!("send cancelled");
                emit(TransferProgress::error("Cancelled"));
                return Ok(());
            }
            Err(e) => {
                log::warn!("link to the receiver broke while sending: {e} — resuming");
                let (s, at) = resume_sender(&mut reconnect, transfer_id, cancel, &emit, &offs, &budget)
                    .inspect_err(|e| {
                        if is_cancel(e) {
                            emit(TransferProgress::error("Cancelled"));
                        }
                    })?;
                budget.spend(at, &e)?;
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

/// Files `next..` onto the wire. Any error here is a link error — unreadable
/// files are skipped in-band (empty frame), so the receiver stays in step.
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
    let total_bytes = offs[items.len()];
    let total_files = items.len() as u64;
    let mut sent = offs[next];
    let mut buf = vec![0u8; BUF];
    let mut last_emit = Instant::now() - Duration::from_secs(1);

    // A file read never yields a 0-byte chunk, so the empty frame is an
    // unambiguous end-of-file marker. That lets an unreadable file (e.g. an
    // un-hydrated OneDrive placeholder failing with os error 362) be skipped
    // without desyncing the receiver, which doesn't rely on manifest sizes.
    for (idx, item) in items.iter().enumerate().skip(next) {
        if cancel.load(Ordering::Relaxed) {
            return Ok(SendFlow::Cancelled);
        }
        log::trace!("sending {}", item.rel);
        let mut ok = true;
        match std::fs::File::open(&item.abs) {
            Ok(mut f) => loop {
                let n = match f.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => n,
                    // cloud placeholder / IO fault mid-read: stop this file, keep the stream sane
                    Err(e) => {
                        log::warn!("read failed mid-file, skipping '{}': {e}", item.rel);
                        ok = false;
                        break;
                    }
                };
                stream.write_frame(&buf[..n])?;
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
            },
            // locked/removed/placeholder: skip cleanly
            Err(e) => {
                log::warn!("cannot open, skipping '{}': {e}", item.rel);
                ok = false;
            }
        }
        stream.write_frame(&[])?; // end of this file
        if ok {
            failed.remove(&idx);
        } else {
            failed.insert(idx);
        }
    }
    stream.flush()?;
    Ok(SendFlow::Done)
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
/// given the deadline) for the sender to reconnect and resume.
pub fn receive<F, A>(
    mut stream: EncryptedStream,
    dest: &Path,
    cancel: &AtomicBool,
    mut reaccept: A,
    emit: F,
) -> io::Result<()>
where
    F: Fn(TransferProgress),
    A: FnMut(Instant) -> io::Result<EncryptedStream>,
{
    let (transfer_id, total_bytes, total_files, chunks) = match read_opening(&mut stream)
        .inspect_err(|e| log::error!("could not read the manifest: {e}"))?
    {
        Opening::Start { transfer_id, total_bytes, total_files, chunks } => {
            (transfer_id, total_bytes, total_files, chunks)
        }
        Opening::Resume { .. } => {
            log::error!("sender tried to resume a transfer this receiver never started");
            return Err(io::Error::new(io::ErrorKind::InvalidData, "unexpected resume"));
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
    loop {
        stream.set_read_timeout(Some(RECV_STALL))?;
        match recv_from(&mut stream, dest, &files, next, &offs, cancel, &emit, start) {
            Ok(RecvFlow::Done) => break,
            Ok(RecvFlow::Cancelled) => {
                log::warn!("receive cancelled");
                emit(TransferProgress::error("Cancelled"));
                return Ok(());
            }
            Err(RecvErr::Local(e)) => return Err(e),
            Err(RecvErr::Link { e, at }) => {
                log::warn!("link to the sender broke at file #{at}: {e} — waiting for it to reconnect");
                drop(stream); // the sender's next write fails fast and it starts reconnecting
                if let Err(give_up) = budget.spend(at, &e) {
                    refuse_one(&mut reaccept, transfer_id);
                    return Err(give_up);
                }
                emit(TransferProgress::reconnecting(
                    offs[at],
                    total_bytes,
                    at as u64,
                    total_files,
                    "Connection interrupted — waiting for the old PC to reconnect…",
                ));
                stream = resume_receiver(&mut reaccept, transfer_id, at).inspect_err(|e| {
                    if is_cancel(e) {
                        emit(TransferProgress::error("Cancelled"));
                    }
                })?;
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

/// After giving up, answer the sender's next reconnect with a refusal so it
/// stops at once instead of retrying for the whole resume window.
fn refuse_one<A>(reaccept: &mut A, transfer_id: u64)
where
    A: FnMut(Instant) -> io::Result<EncryptedStream>,
{
    let deadline = Instant::now() + Duration::from_secs(20);
    while let Ok(mut s) = reaccept(deadline) {
        if let Ok(Opening::Resume { transfer_id: id }) = read_opening(&mut s) {
            if id == transfer_id {
                let _ = s.write_json(&ResumeAt { ok: false, next: 0 });
                return;
            }
        }
    }
}

/// Wait for the sender to come back for `transfer_id` and tell it where to
/// restart. Connections for anything else are turned away.
fn resume_receiver<A>(reaccept: &mut A, transfer_id: u64, at: usize) -> io::Result<EncryptedStream>
where
    A: FnMut(Instant) -> io::Result<EncryptedStream>,
{
    let deadline = Instant::now() + RESUME_WINDOW;
    loop {
        let mut s = match reaccept(deadline) {
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
                s.write_json(&ResumeAt { ok: true, next: at as u64 })?;
                return Ok(s);
            }
            Ok(Opening::Resume { .. }) => {
                log::warn!("a sender tried to resume a different transfer — refused");
                let _ = s.write_json(&ResumeAt { ok: false, next: 0 });
            }
            Ok(Opening::Start { .. }) => {
                log::warn!("a sender tried to start a new transfer mid-resume — refused");
            }
            Err(e) => log::warn!("reconnect sent nothing usable: {e}"),
        }
    }
}

/// A decrypt failure AFTER pairing isn't a wrong code (the manifest already
/// decrypted): the bytes changed in transit. Name it, and where it happened.
fn link_error(e: io::Error, stream: &EncryptedStream, rel: &str, got: u64) -> io::Error {
    if e.kind() != io::ErrorKind::PermissionDenied {
        return e;
    }
    log::error!(
        "integrity check failed on frame #{} while receiving '{rel}', {:.2} GB in — data was altered in transit",
        stream.frames_read(),
        got as f64 / 1e9
    );
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("data arrived damaged while receiving '{rel}' (failed the integrity check)"),
    )
}

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
    let total_bytes = offs[files.len()];
    let total_files = files.len() as u64;
    let mut got = offs[next];
    let mut last_emit = Instant::now() - Duration::from_secs(1);

    for (idx, entry) in files.iter().enumerate().skip(next) {
        if cancel.load(Ordering::Relaxed) {
            return Ok(RecvFlow::Cancelled);
        }
        let out_path = safe_join(dest, &entry.rel);
        log::trace!("receiving {}", entry.rel);
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                log::error!("could not create {}: {e}", parent.display());
                RecvErr::Local(e)
            })?;
        }
        // create() truncates, so a file that was in flight when the link broke
        // is rewritten whole on resume
        let mut out = std::fs::File::create(&out_path).map_err(|e| {
            log::error!("could not create {}: {e}", out_path.display());
            RecvErr::Local(e)
        })?;
        loop {
            let chunk = stream.read_frame().map_err(|e| RecvErr::Link {
                e: link_error(e, stream, &entry.rel, got),
                at: idx,
            })?;
            if chunk.is_empty() {
                break; // end of this file
            }
            out.write_all(&chunk).map_err(|e| {
                log::error!("could not write {}: {e}", out_path.display());
                RecvErr::Local(e)
            })?;
            got += chunk.len() as u64;
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
            let reaccept = |deadline| accept_and_verify(&listener, CODE, &c2, Some(deadline)).map(|(s, _)| s);
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
        let (sent, got, dest) = run("stress", &items, &root, |p| p, 1);
        sent.unwrap();
        got.unwrap();
        assert_same(&items, &dest);
        std::fs::remove_dir_all(&root).ok();
    }
}

//! Encrypted transport: SPAKE2 (pairing code -> strong session key) + per-direction
//! ChaCha20-Poly1305 frames. The code itself is never sent on the wire — a
//! successful decrypt of the first frame is the proof both sides knew it.

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use hkdf::Hkdf;
use serde::de::DeserializeOwned;
use serde::Serialize;
use sha2::Sha256;
use spake2::{Ed25519Group, Identity, Password, Spake2};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// Hard cap on one frame — a manifest for a very large tree can run to
/// megabytes; anything bigger than this is garbage or an attack, not data.
const MAX_FRAME: usize = 64 * 1024 * 1024;
/// SPAKE2 messages are ~33 bytes; anything near this cap is not a peer.
const MAX_HANDSHAKE_MSG: usize = 1024;

/// Run the SPAKE2 handshake over a fresh TCP connection and return the
/// encrypted framing layer. Both sides call this (symmetric PAKE); `initiator`
/// only decides which derived key encrypts which direction.
pub fn pake_handshake(stream: &TcpStream, code: &str, initiator: bool) -> io::Result<EncryptedStream> {
    // Bound the handshake so a stalled or hostile peer (or a firewall that
    // allows the SYN but drops data) can't hang the UI forever.
    let prev_timeout = stream.read_timeout()?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;

    let (state, outbound) = Spake2::<Ed25519Group>::start_symmetric(
        &Password::new(code.as_bytes()),
        &Identity::new(b"winc-v1"),
    );

    let mut w = stream.try_clone()?;
    let mut r = stream.try_clone()?;
    w.write_all(&(outbound.len() as u32).to_be_bytes())?;
    w.write_all(&outbound)?;
    w.flush()?;

    let mut lenb = [0u8; 4];
    r.read_exact(&mut lenb)?;
    let len = u32::from_be_bytes(lenb) as usize;
    if len == 0 || len > MAX_HANDSHAKE_MSG {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "bad handshake"));
    }
    let mut inbound = vec![0u8; len];
    r.read_exact(&mut inbound)?;

    let shared = state
        .finish(&inbound)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "handshake failed"))?;

    // Split the shared key into one key per direction so the two counter-based
    // nonce streams can never collide on the same key.
    let hk = Hkdf::<Sha256>::new(None, &shared);
    let mut a2b = [0u8; 32];
    let mut b2a = [0u8; 32];
    hk.expand(b"winc a2b", &mut a2b)
        .map_err(|_| io::Error::new(io::ErrorKind::Other, "hkdf failed"))?;
    hk.expand(b"winc b2a", &mut b2a)
        .map_err(|_| io::Error::new(io::ErrorKind::Other, "hkdf failed"))?;
    let (send_key, recv_key) = if initiator { (a2b, b2a) } else { (b2a, a2b) };

    stream.set_read_timeout(prev_timeout)?;
    Ok(EncryptedStream {
        reader: BufReader::new(r),
        writer: BufWriter::new(w),
        send_cipher: ChaCha20Poly1305::new(Key::from_slice(&send_key)),
        recv_cipher: ChaCha20Poly1305::new(Key::from_slice(&recv_key)),
        send_n: 0,
        recv_n: 0,
    })
}

/// AEAD-framed transport over TCP. Frames are `[u32 len][ciphertext+tag]`;
/// nonces are per-direction 64-bit counters, so a (key, nonce) pair is never
/// reused. An empty plaintext is a valid frame (used as an end-of-file marker).
pub struct EncryptedStream {
    reader: BufReader<TcpStream>,
    writer: BufWriter<TcpStream>,
    send_cipher: ChaCha20Poly1305,
    recv_cipher: ChaCha20Poly1305,
    send_n: u64,
    recv_n: u64,
}

fn nonce_for(counter: u64) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[4..].copy_from_slice(&counter.to_be_bytes());
    n
}

impl EncryptedStream {
    pub fn write_frame(&mut self, plaintext: &[u8]) -> io::Result<()> {
        let nonce = nonce_for(self.send_n);
        self.send_n += 1;
        let ct = self
            .send_cipher
            .encrypt(Nonce::from_slice(&nonce), plaintext)
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "encrypt failed"))?;
        self.writer.write_all(&(ct.len() as u32).to_be_bytes())?;
        self.writer.write_all(&ct)?;
        Ok(())
    }

    /// A decrypt failure means the peer used a different code (or the data was
    /// tampered with) — surfaced as `PermissionDenied` so callers can show a
    /// clean "wrong code" instead of a byte-level error.
    pub fn read_frame(&mut self) -> io::Result<Vec<u8>> {
        let mut lenb = [0u8; 4];
        self.reader.read_exact(&mut lenb)?;
        let len = u32::from_be_bytes(lenb) as usize;
        if len < 16 || len > MAX_FRAME {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "bad frame length"));
        }
        let mut ct = vec![0u8; len];
        self.reader.read_exact(&mut ct)?;
        let nonce = nonce_for(self.recv_n);
        self.recv_n += 1;
        self.recv_cipher
            .decrypt(Nonce::from_slice(&nonce), ct.as_slice())
            .map_err(|_| io::Error::new(io::ErrorKind::PermissionDenied, "code mismatch"))
    }

    pub fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }

    pub fn write_json<T: Serialize>(&mut self, v: &T) -> io::Result<()> {
        let bytes = serde_json::to_vec(v)?;
        self.write_frame(&bytes)?;
        self.flush()
    }

    pub fn read_json<T: DeserializeOwned>(&mut self) -> io::Result<T> {
        let bytes = self.read_frame()?;
        serde_json::from_slice(&bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }
}

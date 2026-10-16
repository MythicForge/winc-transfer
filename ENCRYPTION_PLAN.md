# Plan — End-to-end encryption for WINC transfers (PAKE + AEAD)

> Status: **implemented** (v0.2.0) — pending Windows build + two-PC verification below.
> Implementation notes vs. this plan: file bytes keep the chunked framing from the
> OneDrive patch (one encrypted frame per chunk, empty frame = end-of-file, so
> unreadable placeholders still skip cleanly); handshake has a 10 s read timeout;
> frame length is capped (64 MB); sender maps the post-handshake reset a Windows
> peer produces on a wrong code (os error 10054) to a clean "wrong code".

## Context

WINC now supports transfers over any IP network (Wi-Fi / Ethernet), not just a
physically-private Thunderbolt/USB4 cable. On a shared network the current wire
protocol is **plaintext**, and the pairing code is sent in the clear
(`Hello{code}` in `net.rs`). A passive eavesdropper can read all transferred
files and the code itself.

Goal: make every transfer **end-to-end encrypted**, authenticated by the
existing 6-digit pairing code, with no offline brute-force weakness.

Decisions (confirmed with user):
- **Always on** — encrypt on cable *and* network. Single code path, no downgrade.
- **Keep the 6-digit code** — safe here because PAKE removes the offline attack;
  only online guessing remains, and it's naturally rate-limited (one guess per
  connection, then the user restarts).

Why not the naive approach: `key = KDF(code)` + AES is broken for the network
case — 6 digits = 10^6 keys, brute-forced offline in seconds from a captured
handshake. The correct tool is a **PAKE (SPAKE2)**, exactly the Magic Wormhole
design: a weak shared code yields a strong session key that resists offline
attack.

## Approach

Add a thin encrypted-transport layer under the existing protocol. The
**manifest / file-stream / progress logic and the entire frontend are
unchanged** — only the byte framing in `net.rs` swaps from newline-JSON + raw
bytes to length-prefixed AEAD frames, preceded by a SPAKE2 handshake.

### New crates (`src-tauri/Cargo.toml`)
```
spake2 = "0.4"              # PAKE key agreement
chacha20poly1305 = "0.10"   # AEAD stream cipher
hkdf = "0.12"               # split shared key into per-direction keys
sha2 = "0.10"               # hash for HKDF
```

### New module: `src-tauri/src/crypto.rs`
- `pake_handshake(stream: &TcpStream, code: &str, initiator: bool) -> io::Result<EncryptedStream>`
  1. `Spake2::<Ed25519Group>::start_symmetric(Password::new(code.as_bytes()), Identity::new(b"winc-v1"))`.
  2. Exchange the 33-byte messages: write ours length-prefixed, read theirs. Both
     sides run identical code (symmetric mode).
  3. `state.finish(&inbound)` → 32-byte shared key.
  4. HKDF-SHA256 → two 32-byte keys with fixed info labels `b"winc a2b"` /
     `b"winc b2a"`. Assign by role: `initiator` (sender / TCP client) sends with
     a2b and receives with b2a; receiver inverts. This prevents any nonce/key
     reuse across directions.
- `struct EncryptedStream` — owns `TcpStream::try_clone` handles, a
  `ChaCha20Poly1305` cipher per direction, and 64-bit send/recv counters.
  - `write_frame(&mut self, plaintext: &[u8]) -> io::Result<()>`: nonce = 12-byte
    (counter, big-endian, zero-padded); `cipher.encrypt`; write `[u32 len][ciphertext+tag]`.
  - `read_frame(&mut self) -> io::Result<Vec<u8>>`: read len, read ciphertext,
    `cipher.decrypt`; **decrypt failure ⇒ wrong code / tampering** →
    `io::ErrorKind::PermissionDenied`.
  - Helpers `write_json<T: Serialize>` / `read_json<T: DeserializeOwned>` wrapping
    a single frame (replaces the current `write_msg`/`read_msg`).

### Changes in `src-tauri/src/net.rs`
- `connect_and_pair(peer, name, code)`:
  1. `TcpStream::connect_timeout` (unchanged).
  2. `crypto::pake_handshake(&stream, code, initiator=true)` → `EncryptedStream`.
  3. Send `Hello{ magic, name }` (**drop the `code` field**) and read
     `HelloAck{ok}` — now over encrypted frames. Successful decrypt *is* the
     proof both sides knew the code.
  - Return the `EncryptedStream` (not the raw `TcpStream`).
- `accept_and_verify(listener, code)`:
  1. `accept()` (unchanged).
  2. `pake_handshake(&stream, code, initiator=false)`.
  3. `read_json::<Hello>()` — a decrypt error here means wrong code → return
     `PermissionDenied` (clean "wrong code" instead of leaking bytes). Send
     `HelloAck{ok:true}`.
  - Return `(EncryptedStream, Peer)`.
- `send_files` / `receive_files`: operate on `&mut EncryptedStream`.
  - Manifest: one JSON frame.
  - File bytes: sender writes each ≤256 KB chunk as one `write_frame`; receiver
    `read_frame`s and writes bytes until each manifest entry's `size` is met.
    (Byte-count boundaries as today; framing just moves inside the cipher.)
  - Progress emission, throttling, cancel checks, `safe_join` — all unchanged.
- Remove the newline-JSON `read_msg`/`write_msg` and the `BufReader` file-read
  path; the frame layer replaces them.
- `Hello` struct in `model.rs`: remove the `code` field.

### Changes in `src-tauri/src/commands.rs`
- `Session.send_stream`: type becomes `Mutex<Option<crypto::EncryptedStream>>`
  (was `TcpStream`) — `pair` builds it, `start_send` takes it. `EncryptedStream`
  must be `Send`.
- `pair` / `start_send` / `receive`: same control flow; just the stored/return
  type changes. Wrong-code errors from the handshake surface through the existing
  `.map_err(|e| e.to_string())` → the frontend `PairStep` already shows a
  "code didn't match" message.

### Frontend
**No changes.** `api.ts`, steps, store untouched. Encryption is invisible above
the transport.

## Files
- `src-tauri/Cargo.toml` — add 4 deps.
- `src-tauri/src/crypto.rs` — **new**: `pake_handshake`, `EncryptedStream`.
- `src-tauri/src/net.rs` — swap framing + handshake in connect/accept/send/receive.
- `src-tauri/src/model.rs` — drop `Hello.code`.
- `src-tauri/src/commands.rs` — `Session.send_stream` type change.

## Security notes to preserve
- Code is never transmitted (plaintext or hashed) — knowledge is proven via
  SPAKE2. Do not add it back to `Hello`.
- Per-direction keys + counter nonces: never reuse a (key, nonce) pair.
- One wrong online guess kills the connection; the receiver returns and the user
  restarts to retry — that is the rate limit. Optionally add a short sleep on
  decrypt failure (cheap defense-in-depth); not required.
- Still not a defense against an active MITM who *also* knows the code — out of
  scope and irrelevant for this use case.

## Verification (on Windows — only place the Rust core compiles)
1. `npm run tauri build` (or `dev`) — confirm the 4 crates compile and link.
2. Two PCs (or two instances + loopback/manual IP):
   - **Correct code** → full transfer completes; files land in
     `Documents\WINC Received\...` and match source (spot-check a few, compare
     sizes/hashes).
   - **Wrong code** → pairing fails fast on the new PC with a clean
     "code didn't match", no partial files written.
3. **Over Wi-Fi** with manual IP entry: run a capture (Wireshark) on the transfer
   port `50738` and confirm payloads are ciphertext, and the digits never appear
   on the wire (including during the handshake).
4. Sanity: a large folder (multi-GB) still streams with correct progress/throughput
   and honest final byte/file totals.

## Effort
Medium — ~150–250 lines of Rust in one new module plus the `net.rs` framing swap.
Frontend and higher-level protocol untouched. Main cost is testing the frame
boundaries and wrong-code path on Windows.

# WINC — Direct-Cable PC Data Crossing

Move files and browser data from an **old Windows PC** to a **new one** over a
single Thunderbolt / USB4 cable. No cloud, no accounts, no router — the data
crosses the wire directly.

Built with **Tauri v2** (Rust core + a React/TypeScript UI).

---

## How it works

"Windows Direct Cable Networking" (Thunderbolt Networking / USB4NET) exposes the
cable as an ordinary network adapter. Both PCs land on the same link-local
segment (usually `169.254.x.x`). WINC rides that link:

```
  OLD PC (send)                         NEW PC (receive)
  ┌───────────────┐   UDP beacon  ◄──── broadcasts { name, tcp_port }
  │ discover_peer │
  │      ↓        │   TCP connect  ────► accept
  │  pair (code)  │   Hello{code}  ────► verify, HelloAck
  │      ↓        │   Manifest     ────► create dest tree
  │  start_send   │   file bytes   ────► write files, emit progress
  └───────────────┘                     Documents\WINC Received\crossing-<ts>\
```

- **Discover** — receiver broadcasts a UDP beacon on port `50737`; sender listens.
- **Pair** — new PC shows a 6-digit code; old PC types it. Verified over TCP
  before any data moves. The connection stays open from pairing through transfer.
- **No cable?** The sender can skip discovery and **enter the new PC's IP** by
  hand (any Wi-Fi / Ethernet network). The receiver shows its IP(s); the TCP port
  is fixed at `50738`, so only the IP is needed.
- **Transfer** — length-declared manifest, then raw file streams. Progress is
  emitted to the UI (`winc://progress`) from whichever side is doing the I/O.

Source of truth for the wire protocol: `src-tauri/src/model.rs` and `net.rs`.

## What transfers

- **Folders** — Documents, Desktop, Pictures, Downloads, Music, Videos, plus any
  folder you add. Sizes are measured live.
- **Browser data** — Chrome / Edge / Firefox bookmarks & history, and (opt-in)
  saved passwords.

> ⚠ **Saved passwords are DPAPI-bound.** Chrome/Edge encrypt them against the
> Windows account. The files copy fine but only decrypt if you sign into the new
> PC with the **same Microsoft account**. The UI states this at the point of
> selection — don't remove that warning.
>
> ⚠ **Close the browser before sending its data** — Chrome/Edge lock their SQLite
> files while running, and locked files are skipped.

---

## Project layout

```
src/                     React UI (the "Signal Bench" design)
  lib/api.ts             backend abstraction: real Tauri calls OR a browser mock
  lib/types.ts,format.ts shared types + number formatting
  store.tsx              app state machine (role -> step)
  components/Cable.tsx   the signature live-cable visualization
  components/...         rail, role picker, session controller
  steps/...              one file per wizard step
  styles/                tokens.css - global.css - app.css
src-tauri/src/
  model.rs               wire types + progress
  net.rs                 link detection, UDP discovery, TCP transfer
  sources.rs             enumerate + expand folders / browser data
  commands.rs            #[tauri::command]s + session state
  lib.rs                 app builder + handler registration
```

### Mock mode
Opened in a plain browser (no Tauri), `api.ts` falls back to a scripted mock so
the whole flow is drivable — useful for design work on non-Windows machines. A
"Demo mode" chip shows bottom-right. The real backend is used automatically
inside the packaged app.

---

## Build & run

### Prerequisites (on the Windows dev machine)
- **Node 18+**
- **Rust** (stable) via <https://rustup.rs>
- **Microsoft C++ Build Tools** (MSVC) + **WebView2** runtime (ships with Win 11)

```powershell
npm install

# hot-reload dev (opens the native window)
npm run tauri dev

# release installer (.msi / .exe in src-tauri/target/release/bundle/)
npm run tauri build
```

> The Rust core targets standard `std::net` + `if-addrs`/`walkdir`/`dirs`. It is
> written on Linux but **must be compiled and tested on Windows** — that's the
> only place the Thunderbolt/USB4 adapter and browser paths exist.

### UI-only preview (any OS)
```bash
npm run dev      # http://localhost:5173  -> runs in mock mode
```

---

## Windows setup for the direct cable

1. Connect both PCs with a **data-rated** Thunderbolt 3/4 or USB4 cable (not
   charge-only).
2. Windows brings up a **Thunderbolt Networking** / **USB4 Net** adapter
   automatically. If not, enable it in the Thunderbolt Control Center / network
   adapter settings.
3. Allow WINC through **Windows Firewall** on Private networks (UDP `50737` for
   discovery, TCP `50738` for transfer). Approve the prompt on first run.
4. Run WINC on both PCs — **Send** on the old one, **Receive** on the new one.

---

## Security notes

- Traffic is unencrypted — acceptable because it never leaves a physical
  point-to-point cable. If you later route this over shared networks, add TLS
  (e.g. `rustls`) and derive a key from the pairing code.
- The 6-digit code prevents pairing with the wrong machine; it is not a
  cryptographic secret.
- Incoming paths are sanitized (`net::safe_join`) so a manifest cannot write
  outside the destination folder.

## Status

Frontend: complete, typechecks, builds. Rust core: complete and written to
standard APIs, **pending a compile/run on Windows** (no Rust toolchain on the
authoring box). Next: `npm run tauri dev` on two Windows PCs wired together.

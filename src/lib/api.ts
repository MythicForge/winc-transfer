/* Backend abstraction.
 * In the packaged Tauri app -> calls Rust commands + listens to real events.
 * In a plain browser (dev / design preview on non-Windows) -> a mock so the
 * whole flow is drivable without two PCs. Detected via window.__TAURI_INTERNALS__.
 */
import type {
  AdapterInfo,
  ImportEntry,
  ImportReport,
  LinkStatus,
  Peer,
  SourceGroup,
  TransferProgress,
} from "./types";

const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/** Fixed TCP port the receiver prefers, so manual IP entry needs no port. */
export const TRANSFER_PORT = 50738;

type ProgressCb = (p: TransferProgress) => void;
type LinkCb = (l: LinkStatus) => void;
type PeerCb = (p: Peer) => void;

export interface Backend {
  /** Poll/subscribe the direct-cable network link. */
  watchLink(cb: LinkCb): () => void;
  /** All network adapters this PC sees, classified — for the connect diagnostics. */
  listAdapters(): Promise<AdapterInfo[]>;
  /** RECEIVE side: open listener, return the 6-digit pairing code + our address. */
  startReceiver(name: string): Promise<{ code: string; peer: Peer }>;
  /** SEND side: look for a receiver beacon on the link-local subnet. */
  discoverPeer(): Promise<Peer | null>;
  /** Every IPv4 on this PC (direct-cable link first), for manual pairing. */
  listLocalIps(): Promise<string[]>;
  /** SEND side: connect to peer and verify the code. */
  pair(peer: Peer, code: string): Promise<void>;
  /** SEND side: the default + browser data groups discovered on this pc. */
  listSources(): Promise<SourceGroup[]>;
  /** SEND side: begin streaming selected groups; progress via cb. */
  startSend(
    peer: Peer,
    groupIds: string[],
    onProgress: ProgressCb,
  ): Promise<void>;
  /** RECEIVE side: wait for + accept an incoming transfer; progress via cb.
   *  Resolves with the folder the crossing landed in. */
  receive(onPeer: PeerCb, onProgress: ProgressCb): Promise<string>;
  /** RECEIVE side: move a finished crossing into the real folders / browsers. */
  importReceived(dir: string): Promise<ImportReport>;
  /** RECEIVE side: force one browser's import ("Overwrite?"), backing up originals first. */
  importBrowserOverwrite(dir: string, label: string): Promise<ImportEntry>;
  cancel(): Promise<void>;
  /** Add a Windows Firewall allow-rule for WINCI on all profiles (UAC prompt). */
  allowFirewall(): Promise<void>;
}

/* ---------------- Tauri backend ---------------- */

function tauriBackend(): Backend {
  // dynamic import keeps mock-only browsers from choking on the module
  const invoke = async <T>(cmd: string, args?: Record<string, unknown>) => {
    const { invoke } = await import("@tauri-apps/api/core");
    return invoke<T>(cmd, args);
  };
  const listen = async <T>(event: string, cb: (p: T) => void) => {
    const { listen } = await import("@tauri-apps/api/event");
    return listen<T>(event, (e) => cb(e.payload));
  };

  return {
    watchLink(cb) {
      // APIPA (169.254) can take several seconds to self-assign after the cable
      // links, so poll — a one-shot check misses the cable and latches onto Wi-Fi.
      const tick = () => invoke<LinkStatus>("get_link_status").then(cb).catch(() => {});
      tick();
      const id = setInterval(tick, 1500);
      return () => clearInterval(id);
    },
    listAdapters() {
      return invoke("list_adapters");
    },
    startReceiver(name) {
      return invoke("start_receiver", { name });
    },
    discoverPeer() {
      return invoke("discover_peer");
    },
    listLocalIps() {
      return invoke("list_local_ips");
    },
    pair(peer, code) {
      return invoke("pair", { peer, code });
    },
    listSources() {
      return invoke("list_sources");
    },
    async startSend(peer, groupIds, onProgress) {
      const un = await listen<TransferProgress>("winc://progress", onProgress);
      void peer; // pairing already bound the connection server-side
      try {
        await invoke("start_send", { groupIds });
      } finally {
        un();
      }
    },
    async receive(onPeer, onProgress) {
      const unP = await listen<Peer>("winc://paired", onPeer);
      const unG = await listen<TransferProgress>("winc://progress", onProgress);
      try {
        return await invoke<string>("receive");
      } finally {
        unP();
        unG();
      }
    },
    importReceived(dir) {
      return invoke("import_received", { dir });
    },
    importBrowserOverwrite(dir, label) {
      return invoke("import_browser_overwrite", { dir, label });
    },
    cancel() {
      return invoke("cancel");
    },
    allowFirewall() {
      return invoke("allow_firewall");
    },
  };
}

/* ---------------- Mock backend ---------------- */

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

function mockBackend(): Backend {
  let cancelled = false;
  const sources: SourceGroup[] = [
    { id: "docs", label: "Documents", hint: "C:\\Users\\you\\Documents", kind: "folder", path: "Documents", bytes: 4_820_000_000, items: 12840, selected: true },
    { id: "desktop", label: "Desktop", hint: "C:\\Users\\you\\Desktop", kind: "folder", path: "Desktop", bytes: 640_000_000, items: 214, selected: true },
    { id: "pictures", label: "Pictures", hint: "C:\\Users\\you\\Pictures", kind: "folder", path: "Pictures", bytes: 18_200_000_000, items: 9021, selected: true },
    { id: "downloads", label: "Downloads", hint: "C:\\Users\\you\\Downloads", kind: "folder", path: "Downloads", bytes: 7_100_000_000, items: 512, selected: false },
    { id: "chrome", label: "Chrome — bookmarks & history", hint: "Default profile", kind: "browser", path: null, bytes: 210_000_000, items: 6, selected: true },
    { id: "edge", label: "Edge — bookmarks & history", hint: "Default profile", kind: "browser", path: null, bytes: 88_000_000, items: 6, selected: false },
    { id: "chrome-pw", label: "Chrome — saved passwords", hint: "Default profile", kind: "browser", path: null, bytes: 400_000, items: 74, caveat: "Passwords are locked to this Windows account (DPAPI). They transfer but only unlock if you sign in with the same Microsoft account on the new PC.", selected: false },
  ];

  return {
    watchLink(cb) {
      cb({ up: false, adapter: null, localIp: null, kind: null });
      const t = setTimeout(
        () => cb({ up: true, adapter: "Thunderbolt Bridge", localIp: "169.254.42.7", kind: "thunderbolt" }),
        2600,
      );
      return () => clearTimeout(t);
    },
    async listAdapters() {
      await sleep(150);
      return [
        { name: "Thunderbolt Bridge", ip: "169.254.42.7", linkLocal: true, cable: true, kind: "thunderbolt" as const },
        { name: "Wi-Fi", ip: "192.168.1.24", linkLocal: false, cable: false, kind: "network" as const },
      ];
    },
    async startReceiver() {
      await sleep(400);
      return { code: "418 620", peer: { name: "SURFACE-9", ip: "169.254.42.9", port: TRANSFER_PORT } };
    },
    async discoverPeer() {
      await sleep(1800);
      return { name: "SURFACE-9", ip: "169.254.42.9", port: TRANSFER_PORT };
    },
    async listLocalIps() {
      await sleep(200);
      return ["169.254.42.9", "192.168.1.24"];
    },
    async pair(_peer, code) {
      await sleep(900);
      if (code.replace(/\s/g, "").length !== 6) throw new Error("bad code");
    },
    async listSources() {
      await sleep(600);
      return sources.map((s) => ({ ...s }));
    },
    async startSend(_peer, groupIds, onProgress) {
      cancelled = false;
      const chosen = sources.filter((s) => groupIds.includes(s.id));
      const bytesTotal = chosen.reduce((a, s) => a + s.bytes, 0);
      const filesTotal = chosen.reduce((a, s) => a + s.items, 0);
      let bytesSent = 0;
      let filesSent = 0;
      const start = performance.now();
      for (const g of chosen) {
        const steps = 22;
        for (let i = 0; i < steps; i++) {
          if (cancelled) {
            onProgress({ state: "error", bytesSent, bytesTotal, filesSent, filesTotal, bytesPerSec: 0, currentFile: null, error: "Cancelled" });
            return;
          }
          await sleep(90);
          bytesSent += g.bytes / steps;
          filesSent += g.items / steps;
          const secs = (performance.now() - start) / 1000;
          onProgress({
            state: "running",
            bytesSent: Math.min(bytesSent, bytesTotal),
            bytesTotal,
            filesSent: Math.round(Math.min(filesSent, filesTotal)),
            filesTotal,
            bytesPerSec: bytesSent / secs,
            currentFile: `${g.label} — item ${Math.round((i / steps) * g.items)}`,
          });
        }
      }
      onProgress({ state: "done", bytesSent: bytesTotal, bytesTotal, filesSent: filesTotal, filesTotal, bytesPerSec: 0, currentFile: null });
    },
    async receive(onPeer, onProgress) {
      await sleep(2000);
      onPeer({ name: "OLD-DELL-7490", ip: "169.254.42.9", port: 52128 });
      await this.startSend({ name: "x", ip: "x", port: 0 }, ["docs", "desktop", "pictures", "chrome"], onProgress);
      return "C:\\Users\\you\\Documents\\WINC Received\\crossing-1721000000";
    },
    async importReceived() {
      await sleep(1500);
      return {
        entries: [
          { label: "Documents", action: "imported" as const, count: 12840, detail: "3 kept both", browserLabel: null },
          { label: "Desktop", action: "imported" as const, count: 214, detail: null, browserLabel: null },
          { label: "Pictures", action: "imported" as const, count: 9021, detail: null, browserLabel: null },
          { label: "Tax Stuff", action: "imported" as const, count: 96, detail: "→ Documents\\Tax Stuff", browserLabel: null },
          { label: "Chrome (browser)", action: "skipped-not-fresh" as const, count: 0, detail: "Chrome already has data on this PC — sign in and sync instead.", browserLabel: "Chrome" },
        ],
      };
    },
    async importBrowserOverwrite(_dir, label) {
      await sleep(1200);
      return {
        label: `${label} (browser)`,
        action: "imported" as const,
        count: 6,
        detail: `overwrote 6 — originals in C:\\Users\\you\\Documents\\WINC Received\\crossing-1721000000\\Backup\\${label}`,
        browserLabel: label,
      };
    },
    async cancel() {
      cancelled = true;
    },
    async allowFirewall() {
      await sleep(600);
    },
  };
}

export const backend: Backend = isTauri ? tauriBackend() : mockBackend();
export const MOCK = !isTauri;

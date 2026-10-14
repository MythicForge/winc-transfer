/* Frontend end of the log bus.
 *
 * In the packaged app the authority is Rust: UI lines are pushed over `ui_log`
 * and come back on the `winc://log` event, so both windows see one stream in
 * one order, and everything also lands in the on-disk log file.
 * In a plain browser (mock mode) the ring lives here so the view still works
 * for design/demo without a cable.
 */

const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export type LogLevel = "error" | "warn" | "info" | "debug" | "trace";

export type LogEntry = {
  seq: number;
  /** ms since epoch */
  ts: number;
  level: LogLevel;
  /** rust module path, or a UI area like "ui::pair" */
  target: string;
  source: "rust" | "ui";
  msg: string;
};

export const LEVELS: LogLevel[] = ["error", "warn", "info", "debug", "trace"];
/** Lower = more severe, so `rank[a] <= rank[b]` means "at least as severe as b". */
const rank: Record<LogLevel, number> = { error: 0, warn: 1, info: 2, debug: 3, trace: 4 };
export const atLeast = (e: LogLevel, floor: LogLevel) => rank[e] <= rank[floor];

const CAP = 3000;

/** Mutated in place — copied once per frame in notify(), never per entry, so a
 *  burst of a few hundred lines doesn't cost a few hundred array clones. */
let live: LogEntry[] = [];
let snap: LogEntry[] = [];
let seen = new Set<number>();
let localSeq = 1_000_000_000; // mock-only ids, far from any Rust seq
const subs = new Set<(entries: LogEntry[]) => void>();

let notifyQueued = false;
function notify() {
  // A transfer can emit in bursts; coalesce so React re-renders once a frame.
  if (notifyQueued) return;
  notifyQueued = true;
  requestAnimationFrame(() => {
    notifyQueued = false;
    snap = live.slice();
    subs.forEach((fn) => fn(snap));
  });
}

function push(e: LogEntry) {
  if (seen.has(e.seq)) return; // hydration overlapping the live stream
  seen.add(e.seq);
  live.push(e);
  if (live.length > CAP) {
    const dropped = live.splice(0, live.length - CAP);
    dropped.forEach((d) => seen.delete(d.seq));
  }
  notify();
}

export function subscribe(fn: (entries: LogEntry[]) => void) {
  subs.add(fn);
  fn(snap);
  return () => void subs.delete(fn);
}

export const snapshot = () => snap;

/* ---------------- invoke helpers ---------------- */

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(cmd, args);
}

/** Emit a UI-side line. Fire-and-forget: logging must never break a flow. */
export function ui(level: LogLevel, target: string, msg: string) {
  if (!isTauri) {
    push({ seq: localSeq++, ts: Date.now(), level, target, source: "ui", msg });
    return;
  }
  call("ui_log", { level, target, msg }).catch(() => {
    // Rust unreachable — keep the line locally so it isn't simply lost.
    push({ seq: localSeq++, ts: Date.now(), level, target, source: "ui", msg });
  });
}

export const uiLog = {
  error: (target: string, msg: string) => ui("error", target, msg),
  warn: (target: string, msg: string) => ui("warn", target, msg),
  info: (target: string, msg: string) => ui("info", target, msg),
  debug: (target: string, msg: string) => ui("debug", target, msg),
};

/** Normalize whatever a rejected promise carried into one line. */
export function errText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  try {
    return JSON.stringify(e);
  } catch {
    return String(e);
  }
}

export async function clearLogs() {
  live = [];
  seen = new Set();
  notify();
  if (isTauri) await call("clear_logs").catch(() => {});
}

export async function setLevel(level: LogLevel) {
  if (isTauri) await call("set_log_level", { level }).catch(() => {});
}

export async function getLevel(): Promise<LogLevel> {
  if (!isTauri) return "debug";
  return call<LogLevel>("get_log_level").catch(() => "info" as LogLevel);
}

export async function logFilePath(): Promise<string> {
  if (!isTauri) return "(mock mode — no log file)";
  return call<string>("log_file_path").catch(() => "");
}

export async function openLogDir() {
  if (!isTauri) {
    ui("warn", "ui::log", "no log file in mock mode — run the packaged app");
    return;
  }
  await call("open_log_dir").catch((e) =>
    ui("error", "ui::log", `could not reveal the log file: ${errText(e)}`),
  );
}

/** Open (or focus) the detachable log window. Built on the Rust side. */
export async function openLogWindow() {
  if (!isTauri) {
    ui("warn", "ui::log", "pop-out needs the packaged app (mock mode is one browser tab)");
    return;
  }
  await call("open_log_window").catch((e) =>
    ui("error", "ui::log", `could not open the log window: ${errText(e)}`),
  );
}

/** True when this document is the popout, not the main UI. */
export const isLogWindow =
  typeof window !== "undefined" &&
  new URLSearchParams(window.location.search).get("view") === "log";

/* ---------------- startup ---------------- */

let started = false;

/** Attach to the Rust stream and backfill history. Safe to call more than once. */
export async function initLogBus() {
  if (started) return;
  started = true;
  if (!isTauri) {
    ui("info", "ui::boot", "mock mode — no backend; log shows UI activity only");
    return;
  }
  try {
    // Listen BEFORE hydrating: anything emitted during the round-trip still
    // arrives, and `seen` drops the overlap.
    const { listen } = await import("@tauri-apps/api/event");
    await listen<LogEntry>("winc://log", (e) => push(e.payload));
    const history = await call<LogEntry[]>("get_logs");
    history.forEach(push);
  } catch (e) {
    push({
      seq: localSeq++,
      ts: Date.now(),
      level: "error",
      target: "ui::log",
      source: "ui",
      msg: `log bus failed to attach: ${errText(e)}`,
    });
  }
}

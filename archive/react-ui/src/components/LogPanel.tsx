import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  atLeast,
  clearLogs,
  getLevel,
  logFilePath,
  openLogDir,
  openLogWindow,
  setLevel,
  subscribe,
  type LogEntry,
  type LogLevel,
} from "../lib/log";

/** Rendering every one of 3000 rows on each burst is what makes a log view
 *  janky; the tail is what anyone actually reads. */
const MAX_ROWS = 800;

const LEVEL_CHOICES: { value: LogLevel; label: string }[] = [
  { value: "error", label: "Errors" },
  { value: "warn", label: "Warnings" },
  { value: "info", label: "Info" },
  { value: "debug", label: "Debug" },
  { value: "trace", label: "Trace (every file)" },
];

function clock(ts: number) {
  const d = new Date(ts);
  const p = (n: number, w = 2) => String(n).padStart(w, "0");
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}.${p(d.getMilliseconds(), 3)}`;
}

export default function LogPanel({ popout = false }: { popout?: boolean }) {
  const [entries, setEntries] = useState<LogEntry[]>([]);
  const [floor, setFloor] = useState<LogLevel>("info");
  const [needle, setNeedle] = useState("");
  const [follow, setFollow] = useState(true);
  const [path, setPath] = useState("");
  const [copied, setCopied] = useState(false);
  const bodyRef = useRef<HTMLDivElement>(null);

  useEffect(() => subscribe(setEntries), []);
  useEffect(() => {
    getLevel().then(setFloor);
    logFilePath().then(setPath);
  }, []);

  const shown = useMemo(() => {
    const q = needle.trim().toLowerCase();
    const hits = entries.filter(
      (e) =>
        atLeast(e.level, floor) &&
        (!q || e.msg.toLowerCase().includes(q) || e.target.toLowerCase().includes(q)),
    );
    return hits.length > MAX_ROWS ? hits.slice(hits.length - MAX_ROWS) : hits;
  }, [entries, floor, needle]);

  const errors = useMemo(() => entries.filter((e) => e.level === "error").length, [entries]);

  // Stick to the bottom while following; the scroll handler below drops follow
  // the moment the user scrolls up to read something.
  useEffect(() => {
    if (!follow) return;
    const el = bodyRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [shown, follow]);

  const onScroll = useCallback(() => {
    const el = bodyRef.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
    setFollow(atBottom);
  }, []);

  const onLevel = (v: LogLevel) => {
    setFloor(v);
    // Debug/trace must also be raised in Rust, or those records are never sent.
    setLevel(v);
  };

  const copy = async () => {
    const text = shown
      .map((e) => `${clock(e.ts)} ${e.level.toUpperCase().padEnd(5)} [${e.target}] ${e.msg}`)
      .join("\n");
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      setTimeout(() => setCopied(false), 1600);
    } catch {
      setCopied(false);
    }
  };

  return (
    <div className="logs">
      <div className="logs__bar">
        <select
          className="logs__select"
          value={floor}
          onChange={(e) => onLevel(e.target.value as LogLevel)}
          aria-label="Minimum level"
        >
          {LEVEL_CHOICES.map((c) => (
            <option key={c.value} value={c.value}>
              {c.label}
            </option>
          ))}
        </select>

        <input
          className="logs__find"
          placeholder="Filter…"
          value={needle}
          onChange={(e) => setNeedle(e.target.value)}
        />

        <span className="logs__count">
          {shown.length} shown{errors > 0 && <b className="logs__errs"> · {errors} error{errors > 1 ? "s" : ""}</b>}
        </span>

        <span className="logs__spacer" />

        <button
          className={`btn btn--ghost logs__btn${follow ? " is-on" : ""}`}
          onClick={() => setFollow((f) => !f)}
          title="Keep the newest line in view"
        >
          {follow ? "Following" : "Follow"}
        </button>
        <button className="btn btn--ghost logs__btn" onClick={copy}>
          {copied ? "Copied" : "Copy"}
        </button>
        <button className="btn btn--ghost logs__btn" onClick={() => clearLogs()}>
          Clear
        </button>
        <button className="btn btn--ghost logs__btn" onClick={() => openLogDir()} title={path}>
          Log file
        </button>
        {!popout && (
          <button className="btn btn--ghost logs__btn" onClick={() => openLogWindow()}>
            Pop out ↗
          </button>
        )}
      </div>

      <div className="logs__body" ref={bodyRef} onScroll={onScroll}>
        {shown.length === 0 ? (
          <p className="logs__empty">
            Nothing at this level yet
            {needle && " matching that filter"}.
          </p>
        ) : (
          shown.map((e) => (
            <div key={e.seq} className={`logrow logrow--${e.level}`}>
              <span className="logrow__t">{clock(e.ts)}</span>
              <span className="logrow__lvl">{e.level}</span>
              <span className="logrow__src">{e.target}</span>
              <span className="logrow__msg">{e.msg}</span>
            </div>
          ))
        )}
      </div>
    </div>
  );
}

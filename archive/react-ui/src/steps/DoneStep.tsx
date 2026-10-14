import { useState } from "react";
import { useStore, useReset } from "../store";
import { backend } from "../lib/api";
import { bytes, count } from "../lib/format";
import type { ImportAction, ImportEntry, ImportReport } from "../lib/types";

const ACTION_BADGE: Record<ImportAction, { text: string; cls: string }> = {
  imported: { text: "✓ imported", cls: "import__badge--ok" },
  "skipped-not-fresh": { text: "skipped", cls: "import__badge--warn" },
  "skipped-not-installed": { text: "not installed", cls: "import__badge--warn" },
  error: { text: "failed", cls: "import__badge--err" },
};

export default function DoneStep() {
  const { state } = useStore();
  const reset = useReset();
  const p = state.progress;
  const sending = state.role === "send";
  const [report, setReport] = useState<ImportReport | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [pending, setPending] = useState<string | null>(null);

  function runImport() {
    if (!state.receivedDir) return;
    setBusy(true);
    setErr(null);
    backend
      .importReceived(state.receivedDir)
      .then(setReport)
      .catch((e) => setErr(String(e)))
      .finally(() => setBusy(false));
  }

  // Re-run one browser's import (Overwrite? or Open First) and swap its row in
  // place with whatever the backend now reports.
  function runBrowserAction(
    label: string,
    fn: (dir: string, label: string) => Promise<ImportEntry>,
  ) {
    if (!state.receivedDir || pending) return;
    setPending(label);
    setErr(null);
    fn(state.receivedDir, label)
      .then((updated) =>
        setReport((r) =>
          r
            ? { entries: r.entries.map((en) => (en.browserLabel === label ? updated : en)) }
            : r,
        ),
      )
      .catch((e) => setErr(String(e)))
      .finally(() => setPending(null));
  }

  return (
    <div>
      <div className="done-hero">
        <div className="done-hero__seal" aria-hidden="true">
          <svg width="30" height="30" viewBox="0 0 24 24" fill="none">
            <path d="M5 13l4 4L19 7" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
        </div>
        <h2>{sending ? "Everything sent" : "Everything landed"}</h2>
        <p style={{ color: "var(--ink-2)", marginTop: 8 }}>
          {bytes(p.bytesTotal)} across {count(p.filesTotal)} items crossed the cable.
        </p>
      </div>

      <div className="card" style={{ padding: 20 }}>
        <div className="eyebrow" style={{ marginBottom: 10 }}>What next</div>
        <ul style={{ color: "var(--ink-2)", lineHeight: 1.8, margin: 0, paddingLeft: 18 }}>
          <li>Open the new PC and check a few folders landed where you expect.</li>
          <li>Sign into your browser to finish syncing bookmarks and history.</li>
          <li>Saved passwords only unlock under the same Windows / Microsoft account.</li>
          <li>You can now safely unplug the cable.</li>
        </ul>
      </div>

      {!sending && (
        <div className="card" style={{ padding: 20, marginTop: 14 }}>
          <div className="eyebrow" style={{ marginBottom: 10 }}>Import into place</div>
          {!report ? (
            <>
              <p style={{ color: "var(--ink-2)", margin: "0 0 12px", lineHeight: 1.6 }}>
                Files landed in <span className="mono">{state.receivedDir ?? "Documents\\WINC Received"}</span>.
                Import moves them into your real folders — nothing on this PC is overwritten;
                incoming duplicates are kept as “name (from old PC)”. Browser data imports
                only into browsers with no existing data; ones that already have data show
                an <b>Overwrite?</b> option, and ones not yet opened show <b>Open First</b>.
              </p>
              <button className="btn btn--primary" disabled={busy || !state.receivedDir} onClick={runImport}>
                {busy ? "Importing…" : "Import into place"}
              </button>
              {err && (
                <p className="field-warn" style={{ marginTop: 8 }}>
                  Import failed — {err}
                </p>
              )}
            </>
          ) : (
            <div className="import-report">
              {report.entries.length === 0 && (
                <p style={{ color: "var(--ink-2)", margin: 0 }}>Nothing to import.</p>
              )}
              {report.entries.map((en) => (
                <div
                  key={en.label}
                  style={{ display: "flex", gap: 10, alignItems: "baseline", padding: "6px 0", borderBottom: "1px solid var(--line, rgba(0,0,0,.06))" }}
                >
                  <b style={{ minWidth: 140 }}>{en.label}</b>
                  {en.action === "skipped-not-fresh" && en.browserLabel ? (
                    <button
                      className="btn btn--ghost import__overwrite"
                      disabled={pending !== null}
                      onClick={() => runBrowserAction(en.browserLabel!, backend.importBrowserOverwrite)}
                      title={`Replace ${en.browserLabel}'s data with the old PC's. Current files are backed up to the WINC Received folder first.`}
                    >
                      {pending === en.browserLabel ? "Overwriting…" : "Overwrite?"}
                    </button>
                  ) : en.action === "skipped-not-installed" && en.browserLabel ? (
                    <button
                      className="btn btn--ghost import__overwrite"
                      disabled={pending !== null}
                      onClick={() => runBrowserAction(en.browserLabel!, backend.importBrowserRetry)}
                      title={`Open ${en.browserLabel} once so it creates its profile, then click to finish importing.`}
                    >
                      {pending === en.browserLabel ? "Checking…" : "Open First"}
                    </button>
                  ) : (
                    <span className={`import__badge ${ACTION_BADGE[en.action].cls}`}>
                      {ACTION_BADGE[en.action].text}
                    </span>
                  )}
                  {en.count > 0 && <span style={{ color: "var(--ink-2)" }}>{count(en.count)} files</span>}
                  {en.detail && <span style={{ color: "var(--ink-2)", fontSize: 13 }}>{en.detail}</span>}
                </div>
              ))}
              <p style={{ color: "var(--slate, var(--ink-2))", fontSize: 12, margin: "10px 0 0" }}>
                A snapshot log of every file (source → destination) was saved as{" "}
                <span className="mono">import-log-*.json</span> in the WINC Received folder.
              </p>
            </div>
          )}
        </div>
      )}

      <div className="actions">
        <button className="btn" onClick={reset}>
          Transfer something else
        </button>
      </div>
    </div>
  );
}

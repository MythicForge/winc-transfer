import { useState } from "react";
import { useStore, useReset } from "../store";
import { backend } from "../lib/api";
import { bytes, count } from "../lib/format";
import type { ImportAction, ImportReport } from "../lib/types";

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
                incoming duplicates are kept as “name (from old PC)”. Browser data is only
                imported into browsers with no existing data.
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
                  <span className={`import__badge ${ACTION_BADGE[en.action].cls}`}>
                    {ACTION_BADGE[en.action].text}
                  </span>
                  {en.count > 0 && <span style={{ color: "var(--ink-2)" }}>{count(en.count)} files</span>}
                  {en.detail && <span style={{ color: "var(--ink-2)", fontSize: 13 }}>{en.detail}</span>}
                </div>
              ))}
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

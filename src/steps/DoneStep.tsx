import { useStore } from "../store";
import { bytes, count } from "../lib/format";

export default function DoneStep() {
  const { state, dispatch } = useStore();
  const p = state.progress;
  const sending = state.role === "send";

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

      <div className="actions">
        <button className="btn" onClick={() => dispatch({ t: "reset" })}>
          Transfer something else
        </button>
      </div>
    </div>
  );
}

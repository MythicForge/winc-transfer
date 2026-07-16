import { useStore } from "../store";
import { bytes, count } from "../lib/format";
import { open } from "@tauri-apps/plugin-dialog";
import { MOCK } from "../lib/api";

export default function SelectStep() {
  const { state, dispatch } = useStore();
  const chosen = state.sources.filter((s) => s.selected);
  const allOn = state.sources.length > 0 && state.sources.every((s) => s.selected);
  const totalBytes = chosen.reduce((a, s) => a + s.bytes, 0);
  const totalItems = chosen.reduce((a, s) => a + s.items, 0);

  async function addFolder() {
    if (MOCK) return; // dialog only in the packaged app
    const dir = await open({ directory: true, multiple: false });
    if (typeof dir === "string") {
      dispatch({
        t: "sources",
        sources: [
          ...state.sources,
          {
            id: `custom-${dir}`,
            label: dir.split(/[\\/]/).pop() || dir,
            hint: dir,
            kind: "folder",
            path: dir,
            bytes: 0,
            items: 0,
            selected: true,
          },
        ],
      });
    }
  }

  return (
    <div>
      <div className="panel-head">
        <div className="eyebrow">Step 3 · Payload</div>
        <h2>Choose what to carry over</h2>
        <p>WINCI found these on the old PC. Tick what should land on the new machine.</p>
      </div>

      <div className="sources">
        {state.sources.map((s) => (
          <label key={s.id} className={`source${s.selected ? " source--on" : ""}`}>
            <input
              type="checkbox"
              checked={s.selected}
              onChange={() => dispatch({ t: "toggle", id: s.id })}
              style={{ position: "absolute", opacity: 0, width: 0, height: 0 }}
            />
            <span className="source__check" aria-hidden="true">
              {s.selected ? "✓" : ""}
            </span>
            <span>
              <span className="source__label">
                {s.label}
                {s.kind === "browser" && <span className="badge-copper">browser</span>}
              </span>
              <span className="source__hint">{s.hint}</span>
            </span>
            <span className="source__meta">
              <b>{bytes(s.bytes)}</b>
              <br />
              {count(s.items)} items
            </span>
            {s.caveat && s.selected && <span className="source__caveat">⚠ {s.caveat}</span>}
          </label>
        ))}
      </div>

      <button className="btn btn--ghost" style={{ marginTop: 10 }} onClick={addFolder} disabled={MOCK}>
        + Add a folder{MOCK ? " (packaged app only)" : ""}
      </button>

      <div className="totals">
        <button
          className="btn btn--ghost"
          disabled={state.sources.length === 0}
          onClick={() => dispatch({ t: "select-all", on: !allOn })}
        >
          {allOn ? "Deselect all" : "Select all"}
        </button>
        <div className="totals__n">
          <b>{bytes(totalBytes)}</b> across {count(totalItems)} items
        </div>
        <button
          className="btn btn--primary"
          disabled={chosen.length === 0}
          onClick={() => dispatch({ t: "step", step: "transfer" })}
        >
          Start transfer →
        </button>
      </div>
    </div>
  );
}

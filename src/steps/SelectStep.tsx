import { useState } from "react";
import { useStore } from "../store";
import { bytes, count } from "../lib/format";
import { open } from "@tauri-apps/plugin-dialog";
import { MOCK, backend } from "../lib/api";
import { errText, uiLog } from "../lib/log";
import type { SourceGroup, TransferMode } from "../lib/types";

const MODES: { id: TransferMode; title: string; sub: string }[] = [
  { id: "user", title: "User Transfer", sub: "Your folders & browser data, found for you" },
  { id: "custom", title: "Custom Transfer", sub: "Pick any files and folders in Explorer" },
];

/** Open Explorer and size up whatever comes back. Null = cancelled. */
async function pick(directory: boolean): Promise<SourceGroup[] | null> {
  const got = MOCK
    ? directory
      ? ["C:\\Projects\\Thesis", "D:\\Photos 2019"]
      : ["C:\\Users\\you\\notes.txt"]
    : await open({ directory, multiple: true });
  const paths = Array.isArray(got) ? got : typeof got === "string" ? [got] : [];
  if (paths.length === 0) return null;
  return backend.measurePaths(paths);
}

export default function SelectStep() {
  const { state, dispatch } = useStore();
  const custom = state.mode === "custom";
  const chosen = custom ? state.picks : state.sources.filter((s) => s.selected);
  const totalBytes = chosen.reduce((a, s) => a + s.bytes, 0);
  const totalItems = chosen.reduce((a, s) => a + s.items, 0);
  const [busy, setBusy] = useState(false);

  async function run(directory: boolean, into: "picks" | "sources") {
    setBusy(true);
    try {
      const found = await pick(directory);
      if (!found) return;
      if (into === "picks") {
        dispatch({ t: "picks-add", picks: found });
      } else {
        const have = new Set(state.sources.map((s) => s.id));
        dispatch({ t: "sources", sources: [...state.sources, ...found.filter((f) => !have.has(f.id))] });
      }
    } catch (e) {
      uiLog.error("ui::select", `could not add from Explorer: ${errText(e)}`);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div>
      <div className="panel-head">
        <div className="eyebrow">Step 3 · Payload</div>
        <h2>Choose what to carry over</h2>
        <p>
          {custom
            ? "Pick exactly what to send. Everything in the list crosses the cable."
            : "WINCI found these on the old PC. Tick what should land on the new machine."}
        </p>
      </div>

      <div className="modes" role="radiogroup" aria-label="Transfer type">
        {MODES.map((m) => (
          <button
            key={m.id}
            role="radio"
            aria-checked={state.mode === m.id}
            className={`mode${state.mode === m.id ? " mode--on" : ""}`}
            onClick={() => dispatch({ t: "mode", mode: m.id })}
          >
            <span className="mode__title">{m.title}</span>
            <span className="mode__sub">{m.sub}</span>
          </button>
        ))}
      </div>

      {custom ? (
        <>
          {state.picks.length === 0 ? (
            <div className="picks-empty">
              Nothing picked yet. Add folders or files from anywhere on this PC — other drives
              included. They land in <span className="mono">Documents</span> on the new PC.
            </div>
          ) : (
            <div className="sources">
              {state.picks.map((s) => (
                <div key={s.id} className="source source--on source--pick">
                  <span className="source__glyph" aria-hidden="true">
                    {s.kind === "file" ? "▤" : "▣"}
                  </span>
                  <span>
                    <span className="source__label">{s.label}</span>
                    <span className="source__hint">{s.hint}</span>
                  </span>
                  <span className="source__meta">
                    <b>{bytes(s.bytes)}</b>
                    <br />
                    {s.kind === "file" ? "file" : `${count(s.items)} items`}
                  </span>
                  <button
                    className="source__remove"
                    title={`Remove ${s.label}`}
                    aria-label={`Remove ${s.label}`}
                    onClick={() => dispatch({ t: "pick-remove", id: s.id })}
                  >
                    ✕
                  </button>
                </div>
              ))}
            </div>
          )}
          <div className="pick-actions">
            <button className="btn btn--ghost" disabled={busy} onClick={() => run(true, "picks")}>
              + Add folders
            </button>
            <button className="btn btn--ghost" disabled={busy} onClick={() => run(false, "picks")}>
              + Add files
            </button>
            {busy && <span className="pick-busy">Measuring…</span>}
          </div>
        </>
      ) : (
        <>
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

          <div className="pick-actions">
            <button className="btn btn--ghost" disabled={busy} onClick={() => run(true, "sources")}>
              + Add a folder
            </button>
            {busy && <span className="pick-busy">Measuring…</span>}
          </div>
        </>
      )}

      <div className="totals">
        {custom ? (
          <button
            className="btn btn--ghost"
            disabled={state.picks.length === 0}
            onClick={() => dispatch({ t: "picks-clear" })}
          >
            Clear list
          </button>
        ) : (
          <button
            className="btn btn--ghost"
            disabled={state.sources.length === 0}
            onClick={() =>
              dispatch({ t: "select-all", on: !state.sources.every((s) => s.selected) })
            }
          >
            {state.sources.length > 0 && state.sources.every((s) => s.selected)
              ? "Deselect all"
              : "Select all"}
          </button>
        )}
        <div className="totals__n">
          <b>{bytes(totalBytes)}</b> across {count(totalItems)} items
        </div>
        <button
          className="btn btn--primary"
          disabled={chosen.length === 0 || busy}
          onClick={() => dispatch({ t: "step", step: "transfer" })}
        >
          Start transfer →
        </button>
      </div>
    </div>
  );
}

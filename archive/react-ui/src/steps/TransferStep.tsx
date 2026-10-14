import { useStore, useReset } from "../store";
import { backend } from "../lib/api";
import { bytes, count, eta, rate } from "../lib/format";

export default function TransferStep() {
  const { state } = useStore();
  const reset = useReset();
  const p = state.progress;
  const pct = p.bytesTotal > 0 ? Math.min(100, (p.bytesSent / p.bytesTotal) * 100) : 0;
  const sending = state.role === "send";

  return (
    <div>
      <div className="panel-head">
        <div className="eyebrow">Step {sending ? 4 : 3} · Crossing</div>
        <h2>{sending ? "Sending to the new PC" : "Receiving from the old PC"}</h2>
        <p>
          Keep the cable connected. Data moves directly between the two machines over the wire
          above — nothing touches the internet.
        </p>
      </div>

      <div className="meters">
        <div className="meter">
          <div className="meter__k">Throughput</div>
          <div className="meter__v mono">{rate(p.bytesPerSec)}</div>
        </div>
        <div className="meter">
          <div className="meter__k">Moved</div>
          <div className="meter__v mono">
            {bytes(p.bytesSent)} <small>/ {bytes(p.bytesTotal)}</small>
          </div>
        </div>
        <div className="meter">
          <div className="meter__k">Items</div>
          <div className="meter__v mono">
            {count(p.filesSent)} <small>/ {count(p.filesTotal)}</small>
          </div>
        </div>
        <div className="meter">
          <div className="meter__k">Time left</div>
          <div className="meter__v mono">{eta(p.bytesTotal - p.bytesSent, p.bytesPerSec)}</div>
        </div>
      </div>

      <div className="progressbar">
        <div className="progressbar__fill" style={{ width: `${pct}%` }} />
      </div>
      <div className="nowfile">{p.currentFile ?? (p.state === "idle" ? "Preparing…" : " ")}</div>

      {p.state === "error" && (
        <p style={{ color: "var(--bad)", fontFamily: "var(--mono)", fontSize: "var(--step--1)", marginTop: 12 }}>
          {p.error ?? "Transfer stopped."}
        </p>
      )}

      <div className="actions">
        {p.state === "running" ? (
          <button className="btn" onClick={() => backend.cancel()}>
            Stop transfer
          </button>
        ) : (
          <button className="btn btn--ghost" onClick={reset}>
            ← Start over
          </button>
        )}
      </div>
    </div>
  );
}

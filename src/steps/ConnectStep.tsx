import { useEffect } from "react";
import { useStore } from "../store";
import { backend } from "../lib/api";

export default function ConnectStep() {
  const { state, dispatch } = useStore();
  const next = state.role === "receive" ? "code" : "pair";

  useEffect(() => {
    return backend.watchLink((link) => dispatch({ t: "link", link }));
  }, [dispatch]);

  const up = state.link.up;

  return (
    <div>
      <div className="panel-head">
        <div className="eyebrow">Step 1 · Physical link</div>
        <h2>Connect the cable</h2>
        <p>
          Join the two PCs with a Thunderbolt or USB4 cable. Windows brings up a direct network
          bridge automatically — no router, no Wi-Fi. WINC watches for the link.
        </p>
      </div>

      <div className="card" style={{ padding: 22 }}>
        <div className="waiting">
          {up ? (
            <>
              <span className="readout__dot readout__dot--linked" />
              <span>
                Link up on <b>{state.link.adapter}</b> · this PC is{" "}
                <span className="mono">{state.link.localIp}</span>
              </span>
            </>
          ) : (
            <>
              <span className="spinner" />
              <span>Waiting for a direct cable link…</span>
            </>
          )}
        </div>

        {!up && (
          <ul style={{ color: "var(--ink-2)", marginTop: 16, lineHeight: 1.7, paddingLeft: 18 }}>
            <li>Use a cable rated for data (Thunderbolt 3/4 or USB4), not charge-only.</li>
            <li>On the other PC, launch WINC too and choose the opposite role.</li>
            <li>If nothing appears, enable “Thunderbolt Networking” in Windows settings.</li>
          </ul>
        )}
      </div>

      <div className="actions">
        <button className="btn btn--primary" disabled={!up} onClick={() => dispatch({ t: "step", step: next })}>
          Cable is connected →
        </button>
        <button className="btn btn--ghost" onClick={() => dispatch({ t: "step", step: next })}>
          No cable — {state.role === "receive" ? "receive over Wi-Fi / LAN" : "connect by IP instead"} →
        </button>
      </div>
    </div>
  );
}

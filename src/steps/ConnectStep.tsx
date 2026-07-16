import { useCallback, useEffect, useState } from "react";
import { useStore } from "../store";
import { backend } from "../lib/api";
import type { AdapterInfo } from "../lib/types";

export default function ConnectStep() {
  const { state, dispatch } = useStore();
  const next = state.role === "receive" ? "code" : "pair";
  const [adapters, setAdapters] = useState<AdapterInfo[]>([]);
  const [fw, setFw] = useState<"idle" | "working" | "done" | "error">("idle");

  const allowFirewall = useCallback(() => {
    setFw("working");
    backend
      .allowFirewall()
      .then(() => setFw("done"))
      .catch(() => setFw("error"));
  }, []);

  useEffect(() => {
    return backend.watchLink((link) => dispatch({ t: "link", link }));
  }, [dispatch]);

  const refreshAdapters = useCallback(() => {
    backend.listAdapters().then(setAdapters).catch(() => {});
  }, []);

  // refresh the adapter list on mount and every 2s while waiting
  useEffect(() => {
    refreshAdapters();
    const id = setInterval(refreshAdapters, 2000);
    return () => clearInterval(id);
  }, [refreshAdapters]);

  const up = state.link.up;
  const cableAdapters = adapters.filter((a) => a.cable);

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
              <span>Waiting for a direct cable link… (the address can take a few seconds)</span>
            </>
          )}
        </div>

        {!up && (
          <ul style={{ color: "var(--ink-2)", marginTop: 16, lineHeight: 1.7, paddingLeft: 18 }}>
            <li>Use a cable rated for data (Thunderbolt 3/4 or USB4), not charge-only.</li>
            <li>On the other PC, launch WINC too and choose the opposite role.</li>
            <li>
              The cable shows up as an <b>Ethernet</b> adapter with a{" "}
              <span className="mono">169.254.x.x</span> address — that's normal.
            </li>
          </ul>
        )}
      </div>

      {/* live diagnostics: what the app actually sees */}
      <div className="card ipcard">
        <div className="diag-head">
          <div className="eyebrow">Detected adapters</div>
          <button className="btn btn--ghost" onClick={refreshAdapters}>
            ↻ Refresh
          </button>
        </div>
        {adapters.length === 0 ? (
          <p className="mono" style={{ color: "var(--slate)", margin: 0 }}>
            No IPv4 adapters found yet…
          </p>
        ) : (
          <div className="adapters">
            {adapters.map((a) => (
              <div key={a.name + a.ip} className={`adapter${a.cable ? " adapter--cable" : ""}`}>
                <span className="adapter__name">{a.name}</span>
                <span className="mono adapter__ip">{a.ip}</span>
                <span className={`adapter__tag${a.cable ? " adapter__tag--cable" : ""}`}>
                  {a.linkLocal ? "direct cable" : a.cable ? "cable" : "network"}
                </span>
              </div>
            ))}
          </div>
        )}
        {!up && cableAdapters.length === 0 && adapters.length > 0 && (
          <p className="field-warn" style={{ marginTop: 12 }}>
            No cable adapter yet. If the cable is plugged in, wait a moment for its{" "}
            <span className="mono">169.254</span> address, or enable “Thunderbolt Networking” in
            Windows settings.
          </p>
        )}

        {/* Windows treats the cable link as an "Unidentified network" (Public
            profile) and silently blocks WINC's discovery + listener there —
            the standard firewall prompt only covers Private networks. */}
        <div style={{ marginTop: 16 }}>
          <p style={{ color: "var(--ink-2)", margin: "0 0 8px" }}>
            If the PCs never find each other, Windows Firewall is usually blocking WINC on
            “Public” networks — the cable link counts as one.
          </p>
          <button className="btn btn--ghost" onClick={allowFirewall} disabled={fw === "working"}>
            {fw === "working"
              ? "Waiting for admin approval…"
              : fw === "done"
                ? "✓ Firewall rule added"
                : "Allow WINC through the firewall"}
          </button>
          {fw === "error" && (
            <p className="field-warn" style={{ marginTop: 8 }}>
              Couldn’t add the rule — the admin prompt was declined or blocked. You can also allow
              WINC manually in “Windows Security → Firewall → Allow an app”.
            </p>
          )}
        </div>
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

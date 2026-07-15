import { useEffect, useState } from "react";
import { useStore } from "../store";
import { backend } from "../lib/api";

export default function CodeStep() {
  const { state } = useStore();
  const code = state.code || "······";
  const port = state.selfPeer?.port;
  const [ips, setIps] = useState<string[]>([]);

  useEffect(() => {
    backend.listLocalIps().then(setIps).catch(() => {});
  }, []);

  return (
    <div>
      <div className="panel-head">
        <div className="eyebrow">Step 2 · Handshake</div>
        <h2>Read this code to the old PC</h2>
        <p>
          On the old computer, WINC is asking for a 6-digit code. Type the numbers below over
          there to confirm the two machines are paired.
        </p>
      </div>

      <div className="codebox">
        <div className="codebox__digits">{code}</div>
        <div className="codebox__cap">Enter this on the old PC to pair</div>
      </div>

      <div className="card ipcard">
        <div className="eyebrow" style={{ marginBottom: 10 }}>No cable? Pair by IP address</div>
        <p style={{ color: "var(--ink-2)", margin: "0 0 14px", fontSize: "var(--step-0)" }}>
          If the old PC can’t find this one automatically, choose “Enter IP” over there and type one
          of these addresses. Port is <span className="mono">{port ?? "—"}</span>.
        </p>
        <div className="iplist">
          {ips.length === 0 ? (
            <span className="mono" style={{ color: "var(--slate)" }}>Reading addresses…</span>
          ) : (
            ips.map((ip, i) => (
              <span key={ip} className={`ipchip${i === 0 ? " ipchip--primary" : ""}`}>
                <span className="mono">{ip}</span>
                {i === 0 && <em>direct link</em>}
              </span>
            ))
          )}
        </div>
      </div>

      <div className="waiting" style={{ marginTop: 22 }}>
        <span className="spinner" />
        <span>Listening for the old PC to connect…</span>
      </div>
    </div>
  );
}

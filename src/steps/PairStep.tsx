import { useEffect, useRef, useState } from "react";
import { useStore } from "../store";
import { backend, TRANSFER_PORT } from "../lib/api";

type Mode = "auto" | "manual";

export default function PairStep() {
  const { state, dispatch } = useStore();
  const [mode, setMode] = useState<Mode>("auto");
  const [entered, setEntered] = useState("");
  const [addr, setAddr] = useState("");
  const [status, setStatus] = useState<"finding" | "found" | "pairing" | "error">("finding");
  const [err, setErr] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  // Auto mode: listen for the receiver beacon on the link.
  useEffect(() => {
    if (mode !== "auto") return;
    let alive = true;
    setStatus("finding");
    backend
      .discoverPeer()
      .then((peer) => {
        if (!alive) return;
        if (peer) {
          dispatch({ t: "peer", peer });
          setStatus("found");
          inputRef.current?.focus();
        } else {
          setStatus("error");
          setErr("No PC found on the cable. Switch to “Enter IP” if you’re on Wi-Fi or Ethernet.");
        }
      })
      .catch(() => alive && setStatus("error"));
    return () => {
      alive = false;
    };
  }, [mode, dispatch]);

  // Manual mode: build the peer from a typed address.
  function parseAddr(input: string) {
    const t = input.trim();
    if (!t) return null;
    const [ip, portStr] = t.split(":");
    const port = portStr ? parseInt(portStr, 10) : TRANSFER_PORT;
    if (!/^\d{1,3}(\.\d{1,3}){3}$/.test(ip) || Number.isNaN(port)) return null;
    return { name: ip, ip, port };
  }
  const manualPeer = mode === "manual" ? parseAddr(addr) : state.peer;

  const digits = entered.replace(/\D/g, "").slice(0, 6);
  const ready = digits.length === 6 && !!manualPeer;

  async function submit() {
    if (!ready || !manualPeer) return;
    setStatus("pairing");
    setErr("");
    try {
      if (mode === "manual") dispatch({ t: "peer", peer: manualPeer });
      await backend.pair(manualPeer, digits);
      const sources = await backend.listSources();
      dispatch({ t: "sources", sources });
      dispatch({ t: "step", step: "select" });
    } catch (e) {
      setStatus("error");
      setErr(
        e instanceof Error && /wrong code|mismatch/i.test(e.message)
          ? "That code didn’t match. Re-read it on the new PC."
          : "Couldn’t reach that PC. Check the IP, that WINC is receiving, and the firewall.",
      );
    }
  }

  return (
    <div>
      <div className="panel-head">
        <div className="eyebrow">Step 2 · Handshake</div>
        <h2>Pair with the new PC</h2>
        <p>
          Confirm the two machines with the 6-digit code shown on the new PC. Auto-find works over
          the direct cable; use “Enter IP” for Wi-Fi or Ethernet.
        </p>
      </div>

      <div className="segmented" role="tablist">
        <button
          role="tab"
          aria-selected={mode === "auto"}
          className={`segmented__btn${mode === "auto" ? " is-on" : ""}`}
          onClick={() => setMode("auto")}
        >
          Find automatically
        </button>
        <button
          role="tab"
          aria-selected={mode === "manual"}
          className={`segmented__btn${mode === "manual" ? " is-on" : ""}`}
          onClick={() => setMode("manual")}
        >
          Enter IP address
        </button>
      </div>

      {mode === "auto" ? (
        <div className="card" style={{ padding: 22, marginBottom: 18 }}>
          <div className="waiting">
            {status === "finding" ? (
              <>
                <span className="spinner" /> <span>Looking for the new PC on the cable…</span>
              </>
            ) : state.peer ? (
              <>
                <span className="readout__dot readout__dot--linked" />
                <span>
                  Found <b>{state.peer.name}</b> at <span className="mono">{state.peer.ip}</span>
                </span>
              </>
            ) : (
              <>
                <span className="readout__dot" /> <span>No peer yet.</span>
              </>
            )}
          </div>
        </div>
      ) : (
        <div style={{ marginBottom: 18 }}>
          <label className="field-label" htmlFor="ipaddr">
            New PC address <span className="mono">(shown on its screen)</span>
          </label>
          <input
            id="ipaddr"
            className="text-input mono"
            placeholder={`192.168.1.24   ·   or  169.254.42.9:${TRANSFER_PORT}`}
            value={addr}
            onChange={(e) => setAddr(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && inputRef.current?.focus()}
            autoFocus
          />
          {addr && !manualPeer && (
            <p className="field-warn">Enter a valid IPv4 address, e.g. 192.168.1.24</p>
          )}
        </div>
      )}

      <label className="field-label" htmlFor="paircode">
        6-digit code
      </label>
      <input
        id="paircode"
        ref={inputRef}
        className="code-input"
        inputMode="numeric"
        placeholder="000000"
        value={digits}
        disabled={status === "pairing"}
        onChange={(e) => setEntered(e.target.value)}
        onKeyDown={(e) => e.key === "Enter" && submit()}
        aria-label="Pairing code"
      />
      {status === "error" && err && <p className="field-warn">{err}</p>}

      <div className="actions">
        <button className="btn btn--ghost" onClick={() => dispatch({ t: "step", step: "connect" })}>
          ← Back
        </button>
        <button className="btn btn--primary" disabled={!ready || status === "pairing"} onClick={submit}>
          {status === "pairing" ? "Pairing…" : "Pair →"}
        </button>
      </div>
    </div>
  );
}

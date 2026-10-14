import { useCallback, useEffect, useState } from "react";
import { useStore, useReset } from "./store";
import { MOCK } from "./lib/api";
import { subscribe } from "./lib/log";
import Cable from "./components/Cable";
import StepRail from "./components/StepRail";
import RolePicker from "./components/RolePicker";
import SessionController from "./components/SessionController";
import LogDrawer, { useLogHotkey } from "./components/LogDrawer";
import ConnectStep from "./steps/ConnectStep";
import PairStep from "./steps/PairStep";
import SelectStep from "./steps/SelectStep";
import CodeStep from "./steps/CodeStep";
import TransferStep from "./steps/TransferStep";
import DoneStep from "./steps/DoneStep";

function StepView() {
  const { state } = useStore();
  const { role, step } = state;
  if (step === "connect") return <ConnectStep />;
  if (step === "transfer") return <TransferStep />;
  if (step === "done") return <DoneStep />;
  if (role === "send") {
    if (step === "pair") return <PairStep />;
    if (step === "select") return <SelectStep />;
  }
  if (role === "receive") {
    if (step === "code") return <CodeStep />;
  }
  return null;
}

export default function App() {
  const { state } = useStore();
  const reset = useReset();
  const [logsOpen, setLogsOpen] = useState(false);
  const [errors, setErrors] = useState(0);

  const toggleLogs = useCallback(() => setLogsOpen((o) => !o), []);
  useLogHotkey(toggleLogs);

  // Badge the toggle so a failure that the flow swallowed is still visible.
  useEffect(
    () => subscribe((es) => setErrors(es.filter((e) => e.level === "error").length)),
    [],
  );
  useEffect(() => {
    if (logsOpen) setErrors(0);
  }, [logsOpen]);

  return (
    <div className="app">
      <header className="topbar">
        <div className="brand">
          <span className="brand__mark">
            WIN<b>CI</b>
          </span>
          <span className="brand__tag">Cable or IP data crossing</span>
        </div>
        <div className="topbar__actions">
          <button
            className="btn btn--ghost"
            onClick={toggleLogs}
            title="Activity log (Ctrl+L)"
          >
            Logs
            {!logsOpen && errors > 0 && <span className="logbtn__dot">{errors}</span>}
          </button>
          {state.role && (
            <button className="btn btn--ghost" onClick={reset}>
              Start over
            </button>
          )}
        </div>
      </header>

      {state.role ? (
        <>
          <Cable />
          <div className="body">
            <StepRail />
            <main className="main">
              <StepView />
            </main>
          </div>
          <SessionController />
        </>
      ) : (
        <RolePicker />
      )}

      <LogDrawer open={logsOpen} onClose={() => setLogsOpen(false)} />

      {MOCK && <div className="mock-flag">Demo mode · no cable</div>}
    </div>
  );
}

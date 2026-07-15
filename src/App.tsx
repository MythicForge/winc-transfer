import { useStore, useReset } from "./store";
import { MOCK } from "./lib/api";
import Cable from "./components/Cable";
import StepRail from "./components/StepRail";
import RolePicker from "./components/RolePicker";
import SessionController from "./components/SessionController";
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

  return (
    <div className="app">
      <header className="topbar">
        <div className="brand">
          <span className="brand__mark">
            WIN<b>C</b>
          </span>
          <span className="brand__tag">Direct cable crossing</span>
        </div>
        {state.role && (
          <button className="btn btn--ghost" onClick={reset}>
            Start over
          </button>
        )}
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

      {MOCK && <div className="mock-flag">Demo mode · no cable</div>}
    </div>
  );
}

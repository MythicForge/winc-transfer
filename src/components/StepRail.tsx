import { useStore, type Step } from "../store";
import { MOCK } from "../lib/api";

const FLOWS: Record<"send" | "receive", { id: Step; label: string; sub: string }[]> = {
  send: [
    { id: "connect", label: "Connect cable", sub: "Thunderbolt / USB4" },
    { id: "pair", label: "Pair devices", sub: "Enter 6-digit code" },
    { id: "select", label: "Choose data", sub: "Files & browser" },
    { id: "transfer", label: "Transfer", sub: "Stream to new PC" },
    { id: "done", label: "Done", sub: "Verify & finish" },
  ],
  receive: [
    { id: "connect", label: "Connect cable", sub: "Thunderbolt / USB4" },
    { id: "code", label: "Show code", sub: "Read it to old PC" },
    { id: "transfer", label: "Receive", sub: "Incoming data" },
    { id: "done", label: "Done", sub: "Everything landed" },
  ],
};

export default function StepRail() {
  const { state } = useStore();
  if (!state.role) return null;
  const steps = FLOWS[state.role];
  const activeIdx = steps.findIndex((s) => s.id === state.step);

  return (
    <nav className="rail" aria-label="Progress">
      {steps.map((s, i) => {
        const cls =
          i === activeIdx ? "rail__step rail__step--active" : i < activeIdx ? "rail__step rail__step--done" : "rail__step";
        return (
          <div key={s.id} className={cls} aria-current={i === activeIdx ? "step" : undefined}>
            <div className="rail__num">{i < activeIdx ? "✓" : i + 1}</div>
            <div>
              <div className="rail__label">{s.label}</div>
              <div className="rail__sub">{s.sub}</div>
            </div>
          </div>
        );
      })}
      <div className="rail__spacer" />
      <div className="rail__foot">
        {state.link.adapter ? `LINK · ${state.link.adapter}` : "LINK · none"}
        <br />
        {state.link.localIp ?? "no address"}
        {MOCK && (
          <>
            <br />
            DEMO DATA
          </>
        )}
      </div>
    </nav>
  );
}

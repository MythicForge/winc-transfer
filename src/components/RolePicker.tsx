import { useStore } from "../store";

export default function RolePicker() {
  const { dispatch } = useStore();
  return (
    <div className="picker">
      <div className="picker__inner">
        <div className="eyebrow">WINC · Direct cable crossing</div>
        <h1>
          Move a whole PC<br />across one wire.
        </h1>
        <p className="picker__lead">
          Plug the two computers together with a Thunderbolt or USB4 cable. WINC carries your
          files and browser data straight across — no cloud, no accounts, no network in between.
        </p>
        <div className="picker__grid">
          <div className="role-card">
            <div className="role-card__glyph">On the OLD PC</div>
            <h3>Send my data</h3>
            <p>This computer holds the files. Pick what to carry over and stream it to the new machine.</p>
            <button className="btn btn--primary" onClick={() => dispatch({ t: "role", role: "send" })}>
              Start sending →
            </button>
          </div>
          <div className="role-card">
            <div className="role-card__glyph">On the NEW PC</div>
            <h3>Receive data</h3>
            <p>This computer is the destination. Show a pairing code to the old PC and stand by.</p>
            <button className="btn" onClick={() => dispatch({ t: "role", role: "receive" })}>
              Wait for a sender
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

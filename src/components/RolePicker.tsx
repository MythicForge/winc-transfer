import { useStore } from "../store";
import logo from "../assets/WINCI.svg";

export default function RolePicker() {
  const { dispatch } = useStore();
  return (
    <div className="picker">
      <div className="picker__inner">
        <img src={logo} alt="WINCI" className="picker__logo" style={{ width: 72, height: 72 }} />
        <div className="eyebrow">WINCI · PC-to-PC data crossing</div>
        <h1>
          Move a whole PC<br />across one wire.
        </h1>
        <p className="picker__lead">
          Connect the two computers with a Thunderbolt / USB4 cable — or use any Wi-Fi or
          Ethernet network by entering the new PC's IP address. WINCI carries your files and
          browser data straight across — no cloud, no accounts, end-to-end encrypted.
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

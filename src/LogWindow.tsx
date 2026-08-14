import LogPanel from "./components/LogPanel";

/** Root of the detached log window (index.html?view=log). It hydrates from the
 *  Rust ring buffer, so opening it mid-transfer still shows how things got here. */
export default function LogWindow() {
  return (
    <div className="logwin">
      <header className="logwin__head">
        <span className="brand__mark">
          WIN<b>CI</b>
        </span>
        <span className="brand__tag">Activity log</span>
      </header>
      <LogPanel popout />
    </div>
  );
}

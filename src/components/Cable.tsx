import { useStore } from "../store";
import { rate } from "../lib/format";

/** The signature element: a live cable between OLD and NEW.
 *  idle -> slack gray wire · linked -> taut copper · flowing -> cobalt pulses. */
export default function Cable() {
  const { state } = useStore();
  const { role, link, peer, progress } = state;

  const flowing = progress.state === "running";
  const linked = link.up && (peer !== null || flowing);

  const wireClass = `wire${linked ? " wire--linked" : ""}${flowing ? " wire--flowing" : ""}`;

  // straight when linked, gentle sag when idle
  const path = linked ? "M 2 20 L 98 20" : "M 2 20 Q 50 32 98 20";

  const thisName = "This PC";
  const thisRole = role === "receive" ? "New device" : "Old device";
  const peerRole = role === "receive" ? "Old device" : "New device";

  let readoutText: React.ReactNode = "No link";
  let dotClass = "readout__dot";
  if (flowing) {
    readoutText = <span className="mono">{rate(progress.bytesPerSec)}</span>;
    dotClass += " readout__dot--live";
  } else if (linked) {
    readoutText = <span className="mono">Linked</span>;
    dotClass += " readout__dot--linked";
  } else if (link.up) {
    readoutText = <span className="mono">Cable up · finding peer</span>;
  }

  return (
    <div className="cable">
      <div className="cable__stage">
        <div className={`node${role ? " node--active" : ""}`}>
          <div className="node__role">{thisRole}</div>
          <div className="node__name">{thisName}</div>
          <div className="node__ip">{link.localIp ?? "—"}</div>
        </div>

        <div className={wireClass}>
          <svg viewBox="0 0 100 40" preserveAspectRatio="none" aria-hidden="true">
            <path id="winc-wire" className="wire__base" d={path} vectorEffect="non-scaling-stroke" />
            <path className="wire__flow" d={path} vectorEffect="non-scaling-stroke" />
            {flowing &&
              [0, 0.33, 0.66].map((delay) => (
                <circle key={delay} className="wire__pulse" r="2.4">
                  <animateMotion dur="1.1s" begin={`${delay * 1.1}s`} repeatCount="indefinite">
                    <mpath href="#winc-wire" />
                  </animateMotion>
                </circle>
              ))}
          </svg>
          <div className="readout">
            <span className={dotClass} />
            {readoutText}
          </div>
        </div>

        <div className={`node${peer ? " node--active" : ""}`}>
          <div className="node__role">{peerRole}</div>
          <div className="node__name">{peer?.name ?? "Waiting…"}</div>
          <div className="node__ip">{peer?.ip ?? "—"}</div>
        </div>
      </div>
    </div>
  );
}

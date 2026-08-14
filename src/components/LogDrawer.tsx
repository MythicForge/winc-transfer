import { useCallback, useEffect, useRef, useState } from "react";
import LogPanel from "./LogPanel";

const MIN_H = 160;
const MAX_FRAC = 0.75;

/** In-app activity log: a resizable bottom drawer. Ctrl+L toggles, Esc closes.
 *  The same panel also runs detached — see LogWindow. */
export default function LogDrawer({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}) {
  const [h, setH] = useState(300);
  const drag = useRef<{ y: number; h: number } | null>(null);

  const onMove = useCallback((e: PointerEvent) => {
    if (!drag.current) return;
    const next = drag.current.h + (drag.current.y - e.clientY);
    setH(Math.max(MIN_H, Math.min(window.innerHeight * MAX_FRAC, next)));
  }, []);

  const stop = useCallback(() => {
    drag.current = null;
    window.removeEventListener("pointermove", onMove);
    window.removeEventListener("pointerup", stop);
  }, [onMove]);

  const start = (e: React.PointerEvent) => {
    drag.current = { y: e.clientY, h };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", stop);
  };

  useEffect(() => stop, [stop]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;

  return (
    <aside className="logdrawer" style={{ height: h }}>
      <div
        className="logdrawer__grip"
        onPointerDown={start}
        role="separator"
        aria-orientation="horizontal"
        title="Drag to resize"
      />
      <div className="logdrawer__head">
        <span className="eyebrow">Activity log</span>
        <button className="btn btn--ghost logs__btn" onClick={onClose}>
          Close
        </button>
      </div>
      <LogPanel />
    </aside>
  );
}

/** Ctrl+L (or Cmd+L) toggle, shared by the app shell. */
export function useLogHotkey(toggle: () => void) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "l") {
        e.preventDefault();
        toggle();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [toggle]);
}

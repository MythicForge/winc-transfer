import { useEffect, useRef } from "react";
import { useStore } from "../store";
import { backend } from "../lib/api";
import { errText, uiLog } from "../lib/log";
import type { TransferProgress } from "../lib/types";

/** Long-lived backend driver. Renders nothing.
 *  Owns the send/receive calls so listeners survive step changes. */
export default function SessionController() {
  const { state, dispatch } = useStore();
  const started = useRef<string>("");
  const lastState = useRef<string>("");

  const role = state.role;
  const step = state.step;

  useEffect(() => {
    const onProgress = (p: TransferProgress) => {
      // Log transitions only — progress ticks arrive several times a second.
      if (p.state !== lastState.current) {
        lastState.current = p.state;
        const detail =
          p.state === "error"
            ? ` — ${p.error ?? "no reason given"}`
            : ` — ${p.filesSent}/${p.filesTotal} files`;
        uiLog[p.state === "error" ? "error" : "info"](
          "ui::transfer",
          `progress: ${p.state}${detail}`,
        );
      }
      dispatch({ t: "progress", progress: p });
      if (p.state === "done") dispatch({ t: "step", step: "done" });
    };

    // SENDER: fire the stream once we reach the transfer step
    if (role === "send" && step === "transfer" && started.current !== "send") {
      started.current = "send";
      const ids = state.sources.filter((s) => s.selected).map((s) => s.id);
      if (state.peer) {
        uiLog.info("ui::transfer", `starting send of ${ids.length} group(s)`);
        backend
          .startSend(state.peer, ids, onProgress)
          .catch((e) => uiLog.error("ui::transfer", `send failed: ${errText(e)}`));
      } else {
        uiLog.error("ui::transfer", "reached the transfer step with no paired peer");
      }
    }

    // RECEIVER: open the listener + code as soon as the code step shows
    if (role === "receive" && step === "code" && started.current !== "recv") {
      started.current = "recv";
      backend
        .startReceiver("This PC")
        .then(({ code, peer }) => {
          dispatch({ t: "code", code });
          dispatch({ t: "selfPeer", peer });
        })
        .catch((e) =>
          uiLog.error("ui::receive", `could not start the receiver: ${errText(e)}`),
        );
      backend
        .receive(
          (peer) => {
            uiLog.info("ui::receive", `sender ${peer.name} (${peer.ip}) connected`);
            dispatch({ t: "peer", peer });
            dispatch({ t: "step", step: "transfer" });
          },
          onProgress,
        )
        .then((dir) => dispatch({ t: "receivedDir", dir }))
        .catch((e) => uiLog.error("ui::receive", `receive failed: ${errText(e)}`));
    }
  }, [role, step, state.peer, state.sources, dispatch]);

  return null;
}

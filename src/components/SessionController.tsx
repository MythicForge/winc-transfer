import { useEffect, useRef } from "react";
import { useStore } from "../store";
import { backend } from "../lib/api";
import type { TransferProgress } from "../lib/types";

/** Long-lived backend driver. Renders nothing.
 *  Owns the send/receive calls so listeners survive step changes. */
export default function SessionController() {
  const { state, dispatch } = useStore();
  const started = useRef<string>("");

  const role = state.role;
  const step = state.step;

  useEffect(() => {
    const onProgress = (p: TransferProgress) => {
      dispatch({ t: "progress", progress: p });
      if (p.state === "done") dispatch({ t: "step", step: "done" });
    };

    // SENDER: fire the stream once we reach the transfer step
    if (role === "send" && step === "transfer" && started.current !== "send") {
      started.current = "send";
      const ids = state.sources.filter((s) => s.selected).map((s) => s.id);
      if (state.peer) backend.startSend(state.peer, ids, onProgress).catch(() => {});
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
        .catch(() => {});
      backend
        .receive(
          (peer) => {
            dispatch({ t: "peer", peer });
            dispatch({ t: "step", step: "transfer" });
          },
          onProgress,
        )
        .catch(() => {});
    }
  }, [role, step, state.peer, state.sources, dispatch]);

  return null;
}

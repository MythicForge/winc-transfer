import {
  createContext,
  useContext,
  useReducer,
  type Dispatch,
  type ReactNode,
} from "react";
import type {
  LinkStatus,
  Peer,
  Role,
  SourceGroup,
  TransferProgress,
} from "./lib/types";

export type SendStep = "connect" | "pair" | "select" | "transfer" | "done";
export type RecvStep = "connect" | "code" | "wait" | "transfer" | "done";
export type Step = SendStep | RecvStep;

export type State = {
  role: Role | null;
  step: Step;
  link: LinkStatus;
  peer: Peer | null;
  selfPeer: Peer | null; // receiver's own address, shown for manual pairing
  code: string; // shown (receive) or entered (send)
  sources: SourceGroup[];
  progress: TransferProgress;
};

const emptyProgress: TransferProgress = {
  state: "idle",
  bytesSent: 0,
  bytesTotal: 0,
  filesSent: 0,
  filesTotal: 0,
  bytesPerSec: 0,
  currentFile: null,
};

export const initialState: State = {
  role: null,
  step: "connect",
  link: { up: false, adapter: null, localIp: null, kind: null },
  peer: null,
  selfPeer: null,
  code: "",
  sources: [],
  progress: emptyProgress,
};

type Action =
  | { t: "role"; role: Role }
  | { t: "link"; link: LinkStatus }
  | { t: "step"; step: Step }
  | { t: "peer"; peer: Peer }
  | { t: "selfPeer"; peer: Peer }
  | { t: "code"; code: string }
  | { t: "sources"; sources: SourceGroup[] }
  | { t: "toggle"; id: string }
  | { t: "progress"; progress: TransferProgress }
  | { t: "reset" };

function reducer(s: State, a: Action): State {
  switch (a.t) {
    case "role":
      return { ...s, role: a.role, step: "connect" };
    case "link":
      return { ...s, link: a.link };
    case "step":
      return { ...s, step: a.step };
    case "peer":
      return { ...s, peer: a.peer };
    case "selfPeer":
      return { ...s, selfPeer: a.peer };
    case "code":
      return { ...s, code: a.code };
    case "sources":
      return { ...s, sources: a.sources };
    case "toggle":
      return {
        ...s,
        sources: s.sources.map((g) =>
          g.id === a.id ? { ...g, selected: !g.selected } : g,
        ),
      };
    case "progress":
      return { ...s, progress: a.progress };
    case "reset":
      return { ...initialState };
    default:
      return s;
  }
}

const Ctx = createContext<{ state: State; dispatch: Dispatch<Action> } | null>(
  null,
);

export function StoreProvider({ children }: { children: ReactNode }) {
  const [state, dispatch] = useReducer(reducer, initialState);
  return <Ctx.Provider value={{ state, dispatch }}>{children}</Ctx.Provider>;
}

export function useStore() {
  const c = useContext(Ctx);
  if (!c) throw new Error("useStore outside provider");
  return c;
}

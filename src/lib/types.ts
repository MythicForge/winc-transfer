export type Role = "send" | "receive";

export type LinkStatus = {
  up: boolean;
  adapter: string | null; // e.g. "Thunderbolt Bridge" / "USB4 Net"
  localIp: string | null;
  kind: "thunderbolt" | "usb4" | "other" | null;
};

export type Peer = {
  name: string;
  ip: string;
  port: number;
};

export type AdapterInfo = {
  name: string;
  ip: string;
  linkLocal: boolean; // 169.254/16 APIPA — the direct-cable signature
  cable: boolean;
  kind: "thunderbolt" | "usb4" | "other" | "network";
};

/** A selectable group of data on the OLD pc. */
export type SourceGroup = {
  id: string;
  label: string;
  hint: string;
  kind: "folder" | "browser";
  path: string | null; // null for browser aggregate / custom
  bytes: number;
  items: number;
  /** caveat text, e.g. saved passwords are DPAPI-bound. */
  caveat?: string;
  selected: boolean;
};

export type TransferProgress = {
  state: "idle" | "running" | "paused" | "done" | "error";
  bytesSent: number;
  bytesTotal: number;
  filesSent: number;
  filesTotal: number;
  bytesPerSec: number;
  currentFile: string | null;
  error?: string;
};

/** One group's outcome from "Import into place" on the receiver. */
export type ImportAction =
  | "imported"
  | "skipped-not-fresh"
  | "skipped-not-installed"
  | "error";

export type ImportEntry = {
  label: string;
  action: ImportAction;
  count: number;
  detail: string | null;
  /** Raw catalog label ("Chrome") on browser entries — used for "Overwrite?". */
  browserLabel: string | null;
};

export type ImportReport = {
  entries: ImportEntry[];
};

/** Events emitted from backend during a live session. */
export type BackendEvent =
  | { type: "link"; payload: LinkStatus }
  | { type: "peer-found"; payload: Peer }
  | { type: "paired"; payload: Peer }
  | { type: "progress"; payload: TransferProgress }
  | { type: "incoming"; payload: { name: string; bytes: number } };

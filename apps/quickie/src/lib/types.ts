// Mirrors crates/quickie-core/src/types.rs.

export interface DeviceInfo {
  id: string;
  name: string;
  platform: string;
}

export interface DeviceStatus extends DeviceInfo {
  paired: boolean;
  /** Paired: connected right now. Unpaired: visible on the network. */
  online: boolean;
}

export type ItemKind =
  | { kind: "text"; text: string }
  | { kind: "file"; name: string; size: number; mime: string }
  /** Sent as a zip built on the fly; desktops unpack it, phones download the zip. */
  | { kind: "folder"; name: string; size: number; files: number };

/** A paired phone browser. */
export interface WebSender {
  id: string;
  name: string;
}

export type Item = {
  id: string;
  owner: string;
  /** Unix epoch milliseconds. */
  created_at: number;
  expires_at: number | null;
  /** Sent from a phone browser; `owner` hosts it on the phone's behalf. */
  via?: WebSender;
} & ItemKind;

/** Anything with bytes behind it: a file or a folder. */
export type FileItem = Exclude<Item, { kind: "text" }>;

export interface Transfer {
  received: number;
  total: number;
}

export interface PairingPrompt {
  device: DeviceInfo;
  code: string;
  expires_at: number;
}

export interface Snapshot {
  me: DeviceInfo;
  port: number;
  devices: DeviceStatus[];
  /** Newest first. */
  items: Item[];
  /** Item id → path, for files whose bytes are on this device. */
  local_files: Record<string, string>;
  transfers: Record<string, Transfer>;
  pairing: PairingPrompt | null;
  web_invite: WebInvite | null;
}

export interface WebInvite {
  /** One per LAN address, best guess first. */
  urls: string[];
  expires_at: number;
}

/** The shelf as a paired phone browser sees it. */
export interface WebSnapshot {
  host: DeviceInfo;
  you: WebSender;
  /** The host first, then its paired devices. */
  devices: DeviceStatus[];
  items: Item[];
  transfers: Record<string, Transfer>;
}

/** `DeviceInfo.platform` of paired phone browsers. */
export const WEB_PLATFORM = "web";

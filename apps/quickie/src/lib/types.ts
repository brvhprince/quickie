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
  | { kind: "file"; name: string; size: number; mime: string };

export type Item = {
  id: string;
  owner: string;
  /** Unix epoch milliseconds. */
  created_at: number;
  expires_at: number | null;
} & ItemKind;

export type FileItem = Item & { kind: "file" };

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
}

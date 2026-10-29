// The shelf as the UI sees it. The phone web client (M2) will implement
// `ShelfClient` over HTTP, so shared components only talk to this interface.
// File-system things only a desktop can do live in `desktop`.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Snapshot } from "./types";

export interface ShelfClient {
  snapshot(): Promise<Snapshot>;
  /** Returns an unsubscribe function. */
  onChange(cb: (snapshot: Snapshot) => void): Promise<() => void>;
  shareText(text: string): Promise<void>;
  unmount(id: string): Promise<void>;
  requestPairing(device: string): Promise<void>;
  pair(device: string, code: string): Promise<void>;
  cancelPairing(): Promise<void>;
  unpair(device: string): Promise<void>;
}

export const tauriShelf: ShelfClient = {
  snapshot: () => invoke<Snapshot>("snapshot"),
  onChange: (cb) => listen<Snapshot>("shelf", (e) => cb(e.payload)),
  shareText: (text) => invoke("share_text", { text }),
  unmount: (id) => invoke("unmount", { id }),
  requestPairing: (device) => invoke("request_pairing", { device }),
  pair: (device, code) => invoke("pair", { device, code }),
  cancelPairing: () => invoke("cancel_pairing"),
  unpair: (device) => invoke("unpair", { device }),
};

export const desktop = {
  /** Resolves to one message per file that couldn't be shared. */
  mountPaths: (paths: string[]) => invoke<string[]>("mount_paths", { paths }),
  fetch: (id: string) => invoke<string>("fetch", { id }),
  open: (id: string) => invoke<void>("open_item", { id }),
  save: (id: string) => invoke<string>("save_item", { id }),
  setExpanded: (expanded: boolean) => invoke<void>("set_expanded", { expanded }),
  createWebInvite: () => invoke<void>("create_web_invite"),
  cancelWebInvite: () => invoke<void>("cancel_web_invite"),
  onToggle: (cb: () => void) => listen("toggle-shelf", cb),
};

import { startDrag } from "@crabnebula/tauri-plugin-drag";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import type { ItemActions } from "./actions";
import { desktop, type ShelfClient } from "./client";
import { dragIcon } from "./dragIcon";

export const desktopActions = (client: ShelfClient): ItemActions => ({
  copyText: (text) => writeText(text),
  open: (item) => desktop.open(item.id),
  save: async (item) => {
    await desktop.save(item.id);
  },
  saveLabel: "Save to Downloads",
  remove: (item) => client.unmount(item.id),
  // Remote files have to be downloaded first, so the first drag starts the download.
  dragOut: async (item, localPath, transfer) => {
    if (localPath) await startDrag({ item: [localPath], icon: dragIcon(item.name), mode: "copy" });
    else if (!transfer) await desktop.fetch(item.id);
  },
});

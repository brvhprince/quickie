import type { FileItem, Item, Transfer } from "./types";

/** What an item card can do, which differs between the desktop app and a phone browser. */
export interface ItemActions {
  copyText(text: string): Promise<void>;
  /** Whether "Open" makes sense here (defaults to yes). */
  canOpen?(item: FileItem): boolean;
  open(item: FileItem): Promise<void>;
  save(item: FileItem): Promise<void>;
  saveLabel: string;
  remove(item: Item): Promise<void>;
  /** Desktop only: drag a file out to Finder / Explorer / another app. */
  dragOut?(item: FileItem, localPath: string | undefined, transfer: Transfer | undefined): Promise<void>;
}

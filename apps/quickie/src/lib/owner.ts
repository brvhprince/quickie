import type { Item } from "./types";

/**
 * Who shared an item, from the viewer's point of view. `you` is the viewer's
 * device id (desktop) or web session id (phone).
 */
export function ownerLabel(item: Item, names: Map<string, string>, you: string): string {
  if (item.via) return item.via.id === you ? "You" : item.via.name;
  if (item.owner === you) return "You";
  return names.get(item.owner) ?? "Unknown device";
}

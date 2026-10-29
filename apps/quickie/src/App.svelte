<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import Bubble from "./lib/Bubble.svelte";
  import Shelf from "./lib/Shelf.svelte";
  import { desktop, tauriShelf as client } from "./lib/client";
  import type { Snapshot } from "./lib/types";

  let snapshot = $state<Snapshot | null>(null);
  let expanded = $state(false);
  let dropping = $state(false);
  let notice = $state("");

  // Another device asking to pair: open up so the code is visible.
  $effect(() => {
    if (snapshot?.pairing && !expanded) toggle(true);
  });

  onMount(() => {
    client.snapshot().then((s) => (snapshot = s));
    const subscriptions = [
      client.onChange((s) => (snapshot = s)),
      desktop.onToggle(() => toggle(!expanded)),
      // Clicking anywhere else tucks the shelf away, unless a pairing code is showing.
      getCurrentWindow().onFocusChanged(({ payload: focused }) => {
        if (!focused && expanded && !snapshot?.pairing) toggle(false);
      }),
      // Files dropped on the bubble or the open shelf are mounted by reference.
      getCurrentWebview().onDragDropEvent(async ({ payload }) => {
        if (payload.type === "enter" || payload.type === "over") dropping = true;
        else if (payload.type === "leave") dropping = false;
        else if (payload.type === "drop") {
          dropping = false;
          const failures = await desktop.mountPaths(payload.paths);
          notice = failures.join("\n");
          if (failures.length && !expanded) toggle(true);
        }
      }),
    ];
    return () => subscriptions.forEach((s) => s.then((unsubscribe) => unsubscribe()));
  });

  // Grow the window before showing the panel, and hide the panel before
  // shrinking, so content never renders clipped.
  async function toggle(next: boolean) {
    if (next === expanded) return;
    if (next) {
      await desktop.setExpanded(true);
      expanded = true;
    } else {
      expanded = false;
      await desktop.setExpanded(false);
    }
  }
</script>

{#if expanded && snapshot}
  <Shelf {snapshot} {client} {dropping} bind:notice onclose={() => toggle(false)} />
{:else}
  <Bubble
    count={snapshot?.items.length ?? 0}
    online={snapshot?.devices.some((d) => d.paired && d.online) ?? false}
    {dropping}
    onopen={() => toggle(true)}
  />
{/if}

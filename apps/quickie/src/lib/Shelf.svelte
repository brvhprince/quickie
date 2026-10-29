<script lang="ts">
  import { onMount } from "svelte";
  import { readText } from "@tauri-apps/plugin-clipboard-manager";
  import Devices from "./Devices.svelte";
  import ItemCard from "./ItemCard.svelte";
  import PhoneInvite from "./PhoneInvite.svelte";
  import { desktop, type ShelfClient } from "./client";
  import { desktopActions } from "./desktopActions";
  import { ownerLabel } from "./owner";
  import type { Snapshot } from "./types";

  let {
    snapshot,
    client,
    dropping,
    notice = $bindable(""),
    onclose,
  }: {
    snapshot: Snapshot;
    client: ShelfClient;
    dropping: boolean;
    notice: string;
    onclose: () => void;
  } = $props();

  let draft = $state("");
  let now = $state(Date.now());
  let input: HTMLTextAreaElement;

  const names = $derived(new Map(snapshot.devices.map((d) => [d.id, d.name] as const)));
  const actions = $derived(desktopActions(client));
  const hasPaired = $derived(snapshot.devices.some((d) => d.paired));

  onMount(() => {
    if (!snapshot.pairing) input.focus();
    const tick = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(tick);
  });

  async function share(text: string) {
    notice = "";
    try {
      await client.shareText(text);
      return true;
    } catch (e) {
      notice = String(e);
      return false;
    }
  }

  async function shareDraft() {
    if (await share(draft)) draft = "";
  }

  async function shareClipboard() {
    try {
      await share(await readText());
    } catch {
      notice = "Clipboard has no text";
    }
  }

  function keydown(e: KeyboardEvent) {
    if (e.key === "Escape") onclose();
  }

  function draftKeydown(e: KeyboardEvent) {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      shareDraft();
    }
  }

  // ⌘V anywhere outside a text field shares the clipboard straight away.
  function paste(e: ClipboardEvent) {
    if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;
    const text = e.clipboardData?.getData("text/plain");
    if (text) {
      e.preventDefault();
      share(text);
    }
  }
</script>

<svelte:window onkeydown={keydown} onpaste={paste} />

<section class="panel" class:dropping>
  <header data-tauri-drag-region>
    <div class="title" data-tauri-drag-region>
      <strong data-tauri-drag-region>Quickie</strong>
      <span class="muted" data-tauri-drag-region>{snapshot.me.name}</span>
    </div>
    <button class="icon" onclick={onclose} aria-label="Close shelf" title="Close (Esc)">
      <svg viewBox="0 0 24 24"><path d="M6 6l12 12M18 6L6 18" /></svg>
    </button>
  </header>

  <Devices devices={snapshot.devices} prompt={snapshot.pairing} {client} {now} />
  {#if snapshot.web_invite}
    <PhoneInvite invite={snapshot.web_invite} {now} oncancel={() => desktop.cancelWebInvite()} />
  {/if}

  <div class="composer">
    <textarea
      bind:this={input}
      bind:value={draft}
      onkeydown={draftKeydown}
      rows="2"
      placeholder="Type, paste, or drop files to share"
    ></textarea>
    <div class="actions">
      <button class="ghost" onclick={shareClipboard}>Share clipboard</button>
      <button class="primary" onclick={shareDraft} disabled={!draft.trim()} title="⌘↵">Share</button>
    </div>
    {#if notice}<p class="notice">{notice}</p>{/if}
  </div>

  <ul class="items">
    {#each snapshot.items as item (item.id)}
      <ItemCard
        {item}
        {actions}
        {now}
        owner={ownerLabel(item, names, snapshot.me.id)}
        mine={item.owner === snapshot.me.id}
        localPath={snapshot.local_files[item.id]}
        transfer={snapshot.transfers[item.id]}
        onerror={(m) => (notice = m)}
      />
    {:else}
      <li class="empty muted">
        {#if hasPaired}
          Nothing on the shelf. Paste text or drop files here and they show up on your other devices.
        {:else}
          Pair a device above to start sharing. Only paired devices can see your shelf.
        {/if}
      </li>
    {/each}
  </ul>

  {#if dropping}
    <div class="drop-hint">Drop to share</div>
  {/if}
</section>

<style>
  .panel {
    position: absolute;
    inset: 6px;
    display: flex;
    flex-direction: column;
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    box-shadow: var(--shadow);
    overflow: hidden;
  }

  .panel.dropping {
    border-color: var(--accent);
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 10px 6px 14px;
    cursor: grab;
  }

  .title {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
  }

  .title .muted {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .icon {
    width: 26px;
    height: 26px;
    border-radius: 7px;
    display: grid;
    place-items: center;
  }

  .icon:hover {
    background: var(--surface);
  }

  .icon svg {
    width: 14px;
    height: 14px;
    stroke: currentColor;
    stroke-width: 2;
    stroke-linecap: round;
  }

  .composer {
    padding: 0 12px 10px;
    border-bottom: 1px solid var(--border);
  }

  textarea {
    width: 100%;
    box-sizing: border-box;
    resize: none;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: 9px;
    background: var(--surface);
    outline: none;
  }

  textarea:focus {
    border-color: var(--accent);
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
    margin-top: 6px;
  }

  .notice {
    margin: 6px 0 0;
    color: var(--danger);
    font-size: 12px;
    white-space: pre-line;
  }

  .items {
    list-style: none;
    margin: 0;
    padding: 8px;
    overflow-y: auto;
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .empty {
    padding: 24px 16px;
    text-align: center;
  }

  .drop-hint {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    background: color-mix(in srgb, var(--panel) 80%, transparent);
    color: var(--accent);
    font-size: 15px;
    font-weight: 600;
    pointer-events: none;
  }
</style>

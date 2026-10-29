<script lang="ts">
  import { writeText } from "@tauri-apps/plugin-clipboard-manager";
  import { startDrag } from "@crabnebula/tauri-plugin-drag";
  import { desktop, type ShelfClient } from "./client";
  import { dragIcon } from "./dragIcon";
  import { ago, bytes, expiresIn, extension } from "./format";
  import type { Item, Transfer } from "./types";

  let {
    item,
    owner,
    mine,
    localPath,
    transfer,
    client,
    now,
    onerror,
  }: {
    item: Item;
    owner: string;
    mine: boolean;
    localPath: string | undefined;
    transfer: Transfer | undefined;
    client: ShelfClient;
    now: number;
    onerror: (message: string) => void;
  } = $props();

  let copied = $state(false);

  async function run(action: () => Promise<unknown>) {
    try {
      await action();
    } catch (e) {
      onerror(String(e));
    }
  }

  async function copy() {
    if (item.kind !== "text") return;
    await writeText(item.text);
    copied = true;
    setTimeout(() => (copied = false), 1200);
  }

  // Press and move to drag the file out to Finder / Explorer / any app. Remote
  // files have to be downloaded first, so the first drag starts the download.
  const DRAG_THRESHOLD = 4;
  let pressedAt: { x: number; y: number } | null = null;

  function down(e: PointerEvent) {
    if (e.button === 0 && item.kind === "file") pressedAt = { x: e.screenX, y: e.screenY };
  }

  function move(e: PointerEvent) {
    if (!pressedAt || item.kind !== "file") return;
    if (Math.hypot(e.screenX - pressedAt.x, e.screenY - pressedAt.y) < DRAG_THRESHOLD) return;
    pressedAt = null;
    if (localPath) {
      run(() => startDrag({ item: [localPath], icon: dragIcon(item.name), mode: "copy" }));
    } else if (!transfer) {
      run(() => desktop.fetch(item.id));
    }
  }

  const percent = $derived(transfer && transfer.total > 0 ? (transfer.received / transfer.total) * 100 : 0);
</script>

<svelte:window onpointerup={() => (pressedAt = null)} />

<li class="item">
  <div class="meta">
    <span class:mine>{owner}</span>
    <span class="muted">· {ago(item.created_at, now)}</span>
    {#if item.expires_at}<span class="muted expires">{expiresIn(item.expires_at, now)}</span>{/if}
  </div>

  {#if item.kind === "text"}
    <p class="text">{item.text}</p>
    <div class="actions">
      <button class="ghost" onclick={copy}>{copied ? "Copied" : "Copy"}</button>
      {#if mine}
        <button class="ghost danger" onclick={() => run(() => client.unmount(item.id))} title="Remove from every device">Unmount</button>
      {/if}
    </div>
  {:else}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="file"
      class:draggable={!!localPath}
      title={localPath ? "Drag out to copy it anywhere" : "Drag to download"}
      onpointerdown={down}
      onpointermove={move}
    >
      <span class="badge">{extension(item.name)}</span>
      <span class="name">{item.name}</span>
      <span class="muted size">{bytes(item.size)}</span>
    </div>
    {#if transfer}
      <div class="progress" role="progressbar" aria-valuenow={Math.round(percent)} aria-valuemin="0" aria-valuemax="100">
        <div class="bar" style:width="{percent}%"></div>
      </div>
      <div class="muted status">Downloading {bytes(transfer.received)} of {bytes(transfer.total)}</div>
    {:else}
      <div class="actions">
        <button class="ghost" onclick={() => run(() => desktop.open(item.id))}>Open</button>
        {#if mine}
          <button class="ghost danger" onclick={() => run(() => client.unmount(item.id))} title="Remove from every device">Unmount</button>
        {:else}
          <button class="ghost" onclick={() => run(() => desktop.save(item.id))}>Save to Downloads</button>
        {/if}
      </div>
    {/if}
  {/if}
</li>

<style>
  .item {
    padding: 9px 10px 6px;
    border-radius: 10px;
    background: var(--surface);
  }

  .meta {
    display: flex;
    gap: 4px;
    font-size: 12px;
    font-weight: 600;
  }

  .mine {
    color: var(--accent);
  }

  .meta .muted {
    font-weight: 400;
  }

  .expires {
    margin-left: auto;
  }

  .text {
    margin: 4px 0 0;
    white-space: pre-wrap;
    word-break: break-word;
    display: -webkit-box;
    -webkit-line-clamp: 6;
    line-clamp: 6;
    -webkit-box-orient: vertical;
    overflow: hidden;
    -webkit-user-select: text;
    user-select: text;
  }

  .file {
    display: flex;
    align-items: center;
    gap: 9px;
    margin-top: 6px;
    padding: 6px;
    border-radius: 8px;
    background: var(--panel);
    cursor: default;
  }

  .file.draggable {
    cursor: grab;
  }

  .badge {
    flex: none;
    width: 34px;
    height: 34px;
    border-radius: 7px;
    display: grid;
    place-items: center;
    background: linear-gradient(135deg, var(--accent), var(--accent-2));
    color: #fff;
    font-size: 10px;
    font-weight: 700;
  }

  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .size {
    flex: none;
    font-size: 12px;
  }

  .progress {
    height: 4px;
    margin-top: 8px;
    border-radius: 2px;
    background: var(--surface-hover);
    overflow: hidden;
  }

  .bar {
    height: 100%;
    background: var(--accent);
    transition: width 150ms linear;
  }

  .status {
    margin: 4px 0 3px;
    font-size: 12px;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
    margin-top: 4px;
    font-size: 12px;
  }
</style>

<script lang="ts">
  import type { ItemActions } from "./actions";
  import { ago, bytes, expiresIn, extension } from "./format";
  import type { Item, Transfer } from "./types";

  let {
    item,
    owner,
    mine,
    localPath,
    transfer,
    actions,
    now,
    onerror,
  }: {
    item: Item;
    /** Who shared it, as shown to this viewer. */
    owner: string;
    /** Whether this viewer may unmount it. */
    mine: boolean;
    localPath?: string;
    transfer?: Transfer;
    actions: ItemActions;
    now: number;
    onerror: (message: string) => void;
  } = $props();

  let copied = $state(false);

  // Not awaited before the action starts, so things like window.open still run
  // inside the click's user gesture (phone browsers block them otherwise).
  function run(action: () => Promise<unknown>) {
    action().catch((e) => onerror(e instanceof Error ? e.message : String(e)));
  }

  function copy(text: string) {
    run(async () => {
      await actions.copyText(text);
      copied = true;
      setTimeout(() => (copied = false), 1200);
    });
  }

  // Press and move to drag the file out (desktop only).
  const DRAG_THRESHOLD = 4;
  let pressedAt: { x: number; y: number } | null = null;

  function down(e: PointerEvent) {
    if (e.button === 0 && actions.dragOut) pressedAt = { x: e.screenX, y: e.screenY };
  }

  function move(e: PointerEvent) {
    if (!pressedAt || item.kind !== "file" || !actions.dragOut) return;
    if (Math.hypot(e.screenX - pressedAt.x, e.screenY - pressedAt.y) < DRAG_THRESHOLD) return;
    pressedAt = null;
    const file = item;
    const dragOut = actions.dragOut;
    run(() => dragOut(file, localPath, transfer));
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
    {@const text = item.text}
    <p class="text">{text}</p>
    <div class="actions">
      <button class="ghost" onclick={() => copy(text)}>{copied ? "Copied" : "Copy"}</button>
      {#if mine}
        <button class="ghost danger" onclick={() => run(() => actions.remove(item))} title="Remove from every device">
          Unmount
        </button>
      {/if}
    </div>
  {:else}
    {@const file = item}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="file"
      class:draggable={actions.dragOut && !!localPath}
      title={actions.dragOut ? (localPath ? "Drag out to copy it anywhere" : "Drag to download") : file.name}
      onpointerdown={down}
      onpointermove={move}
    >
      <span class="badge">
        {#if file.kind === "folder"}
          <svg viewBox="0 0 24 24" aria-label="folder"><path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" /></svg>
        {:else}
          {extension(file.name)}
        {/if}
      </span>
      <span class="name">{file.name}</span>
      <span class="muted size">
        {#if file.kind === "folder"}{file.files} {file.files === 1 ? "file" : "files"} · {/if}{bytes(file.size)}
      </span>
    </div>
    {#if transfer}
      <div class="progress" role="progressbar" aria-valuenow={Math.round(percent)} aria-valuemin="0" aria-valuemax="100">
        <div class="bar" style:width="{percent}%"></div>
      </div>
      <div class="muted status">Fetching {bytes(transfer.received)} of {bytes(transfer.total)}</div>
    {:else}
      <div class="actions">
        {#if actions.canOpen?.(file) ?? true}
          <button class="ghost" onclick={() => run(() => actions.open(file))}>Open</button>
        {/if}
        {#if mine}
          <button class="ghost danger" onclick={() => run(() => actions.remove(item))} title="Remove from every device">
            Unmount
          </button>
        {:else}
          <button class="ghost" onclick={() => run(() => actions.save(file))}>{actions.saveLabel}</button>
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

  .badge svg {
    width: 18px;
    height: 18px;
    fill: #fff;
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

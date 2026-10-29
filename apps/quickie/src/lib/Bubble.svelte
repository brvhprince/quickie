<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";

  let {
    count,
    online,
    dropping,
    onopen,
  }: { count: number; online: boolean; dropping: boolean; onopen: () => void } = $props();

  // Click opens the shelf; press-and-move drags the bubble around the screen.
  const DRAG_THRESHOLD = 4;
  let pressedAt: { x: number; y: number } | null = null;

  function down(e: PointerEvent) {
    if (e.button === 0) pressedAt = { x: e.screenX, y: e.screenY };
  }

  function move(e: PointerEvent) {
    if (!pressedAt) return;
    if (Math.hypot(e.screenX - pressedAt.x, e.screenY - pressedAt.y) > DRAG_THRESHOLD) {
      pressedAt = null;
      getCurrentWindow().startDragging();
    }
  }

  function up() {
    if (pressedAt) {
      pressedAt = null;
      onopen();
    }
  }
</script>

<button
  class="bubble"
  class:dropping
  title={online ? "Quickie: drop files or click to open" : "Quickie: no paired devices online"}
  aria-label="Open shelf"
  onpointerdown={down}
  onpointermove={move}
  onpointerup={up}
>
  <svg viewBox="0 0 24 24" aria-hidden="true">
    <rect x="5" y="6" width="14" height="3" rx="1.5" opacity="0.55" />
    <rect x="5" y="10.5" width="14" height="3" rx="1.5" opacity="0.8" />
    <rect x="5" y="15" width="14" height="3" rx="1.5" />
  </svg>
  {#if count > 0}
    <span class="badge">{count > 99 ? "99+" : count}</span>
  {/if}
  <span class="presence" class:online></span>
</button>

<style>
  .bubble {
    position: absolute;
    inset: 8px;
    border-radius: 50%;
    background: linear-gradient(135deg, var(--accent), var(--accent-2));
    box-shadow: var(--shadow);
    display: grid;
    place-items: center;
    transition: transform 120ms ease;
  }

  .bubble:hover {
    transform: scale(1.06);
  }

  .bubble:active {
    transform: scale(0.96);
  }

  .bubble.dropping {
    transform: scale(1.12);
    box-shadow: 0 0 0 3px var(--panel), 0 0 0 6px var(--accent), var(--shadow);
  }

  svg {
    width: 26px;
    height: 26px;
    fill: #fff;
  }

  .badge {
    position: absolute;
    top: -3px;
    right: -3px;
    min-width: 18px;
    height: 18px;
    padding: 0 5px;
    box-sizing: border-box;
    border-radius: 9px;
    background: var(--danger);
    color: #fff;
    font-size: 11px;
    font-weight: 600;
    line-height: 18px;
    text-align: center;
  }

  .presence {
    position: absolute;
    bottom: 1px;
    right: 1px;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--offline);
    border: 2px solid var(--panel);
  }

  .presence.online {
    background: var(--online);
  }
</style>

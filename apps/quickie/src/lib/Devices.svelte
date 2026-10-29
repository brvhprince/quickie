<script lang="ts">
  import type { ShelfClient } from "./client";
  import type { DeviceStatus, PairingPrompt } from "./types";

  let {
    devices,
    prompt,
    client,
    now,
  }: { devices: DeviceStatus[]; prompt: PairingPrompt | null; client: ShelfClient; now: number } = $props();

  const paired = $derived(devices.filter((d) => d.paired));
  const nearby = $derived(devices.filter((d) => !d.paired));

  /** The nearby device we asked to pair with, while its user reads us the code. */
  let pairingWith = $state<DeviceStatus | null>(null);
  let code = $state("");
  let busy = $state(false);
  let error = $state("");
  let confirmUnpair = $state<string | null>(null);

  async function ask(device: DeviceStatus) {
    error = "";
    busy = true;
    try {
      await client.requestPairing(device.id);
      pairingWith = device;
      code = "";
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    if (!pairingWith) return;
    error = "";
    busy = true;
    try {
      await client.pair(pairingWith.id, code);
      pairingWith = null;
    } catch (err) {
      error = String(err);
      code = "";
    } finally {
      busy = false;
    }
  }

  async function unpair(id: string) {
    confirmUnpair = null;
    await client.unpair(id).catch((e) => (error = String(e)));
  }

  const secondsLeft = (p: PairingPrompt) => Math.max(0, Math.ceil((p.expires_at - now) / 1000));
</script>

{#if prompt && secondsLeft(prompt) > 0}
  <div class="prompt">
    <div><strong>{prompt.device.name}</strong> wants to pair</div>
    <div class="code" aria-label="Pairing code">{prompt.code.slice(0, 3)} {prompt.code.slice(3)}</div>
    <div class="row">
      <span class="muted">Type this on {prompt.device.name} · {secondsLeft(prompt)}s</span>
      <button class="ghost" onclick={() => client.cancelPairing()}>Cancel</button>
    </div>
  </div>
{/if}

<div class="devices">
  {#each paired as device (device.id)}
    <span class="chip" class:offline={!device.online} title="{device.platform} · {device.online ? 'online' : 'offline'}">
      <i class="dot"></i>{device.name}
      {#if confirmUnpair === device.id}
        <button class="unpair confirm" onclick={() => unpair(device.id)}>Unpair</button>
      {:else}
        <button class="unpair" aria-label="Unpair {device.name}" onclick={() => (confirmUnpair = device.id)}>×</button>
      {/if}
    </span>
  {/each}
  {#if paired.length === 0 && nearby.length === 0}
    <span class="muted">Looking for devices on your network…</span>
  {/if}
</div>

{#if pairingWith}
  <form class="pairing" onsubmit={submit}>
    <label for="code">Enter the code shown on <strong>{pairingWith.name}</strong></label>
    <div class="row">
      <!-- svelte-ignore a11y_autofocus -->
      <input
        id="code"
        bind:value={code}
        inputmode="numeric"
        autocomplete="one-time-code"
        maxlength="7"
        placeholder="123 456"
        autofocus
      />
      <button class="primary" disabled={busy || code.replace(/\D/g, "").length !== 6}>Pair</button>
      <button type="button" class="ghost" onclick={() => (pairingWith = null)}>Cancel</button>
    </div>
  </form>
{:else if nearby.length > 0}
  <div class="nearby">
    <span class="muted label">Nearby</span>
    {#each nearby as device (device.id)}
      <button class="near" disabled={busy} onclick={() => ask(device)} title="Pair with {device.name}">
        {device.name} <span class="plus">Pair</span>
      </button>
    {/each}
  </div>
{/if}

{#if error}<p class="error">{error}</p>{/if}

<style>
  .prompt {
    margin: 0 12px 10px;
    padding: 10px 12px;
    border-radius: 10px;
    background: color-mix(in srgb, var(--accent) 12%, var(--panel));
    border: 1px solid color-mix(in srgb, var(--accent) 35%, transparent);
  }

  .code {
    font: 600 28px/1.2 ui-monospace, "SF Mono", Menlo, Consolas, monospace;
    letter-spacing: 0.08em;
    margin: 4px 0;
    -webkit-user-select: text;
    user-select: text;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .row .muted {
    flex: 1;
    font-size: 12px;
  }

  .devices {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    padding: 0 14px 8px;
    min-height: 22px;
    font-size: 12px;
    line-height: 22px;
  }

  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 0 4px 0 9px;
    border-radius: 11px;
    background: var(--surface);
  }

  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--online);
  }

  .offline {
    color: var(--muted);
  }

  .offline .dot {
    background: var(--offline);
  }

  .unpair {
    width: 18px;
    height: 18px;
    border-radius: 9px;
    line-height: 18px;
    color: var(--muted);
    opacity: 0;
  }

  .chip:hover .unpair,
  .unpair:focus-visible,
  .unpair.confirm {
    opacity: 1;
  }

  .unpair:hover {
    background: var(--surface-hover);
  }

  .unpair.confirm {
    width: auto;
    padding: 0 7px;
    color: var(--danger);
  }

  .nearby {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    padding: 0 14px 10px;
    font-size: 12px;
  }

  .label {
    margin-right: 2px;
  }

  .near {
    padding: 2px 4px 2px 9px;
    border-radius: 11px;
    border: 1px dashed var(--border);
  }

  .near:hover {
    border-color: var(--accent);
  }

  .plus {
    margin-left: 4px;
    padding: 0 6px;
    border-radius: 8px;
    background: var(--accent);
    color: var(--accent-text);
    font-weight: 600;
  }

  .pairing {
    padding: 0 14px 10px;
    font-size: 12px;
  }

  .pairing label {
    display: block;
    margin-bottom: 6px;
  }

  input {
    flex: 1;
    min-width: 0;
    padding: 5px 9px;
    border: 1px solid var(--border);
    border-radius: 7px;
    background: var(--surface);
    font: 600 15px ui-monospace, "SF Mono", Menlo, Consolas, monospace;
    letter-spacing: 0.1em;
    color: inherit;
    outline: none;
  }

  input:focus {
    border-color: var(--accent);
  }

  .error {
    margin: -4px 14px 8px;
    color: var(--danger);
    font-size: 12px;
  }
</style>

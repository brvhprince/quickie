<script lang="ts">
  import { renderSVG } from "uqr";
  import type { WebInvite } from "./types";

  let { invite, now, oncancel }: { invite: WebInvite; now: number; oncancel: () => void } = $props();

  let choice = $state(0);
  const url = $derived(invite.urls[Math.min(choice, invite.urls.length - 1)]);
  // Generated locally from our own URL, so rendering it as markup is safe.
  const qr = $derived(renderSVG(url, { border: 1 }));
  const left = $derived(Math.max(0, Math.ceil((invite.expires_at - now) / 1000)));
  const host = (u: string) => new URL(u).host;
</script>

<div class="invite">
  <div class="head">
    <strong>Add a phone</strong>
    <button class="ghost" onclick={oncancel}>Cancel</button>
  </div>
  <div class="qr" aria-label="QR code for {host(url)}">{@html qr}</div>
  <ol class="steps muted">
    <li>Scan this with your phone's camera. It works once, for {Math.floor(left / 60)}:{String(left % 60).padStart(2, "0")}.</li>
    <li>
      Your browser will warn the connection isn't private. That's expected on a home network:
      <b>Show Details → visit this website</b> on iPhone, <b>Advanced → Proceed</b> on Android.
    </li>
  </ol>
  {#if invite.urls.length > 1}
    <label class="muted address">
      Phone can't connect? Try another address:
      <select bind:value={choice}>
        {#each invite.urls as u, i (u)}<option value={i}>{host(u)}</option>{/each}
      </select>
    </label>
  {/if}
</div>

<style>
  .invite {
    margin: 0 12px 10px;
    padding: 10px 12px 12px;
    border-radius: 10px;
    background: color-mix(in srgb, var(--accent) 10%, var(--panel));
    border: 1px solid color-mix(in srgb, var(--accent) 35%, transparent);
    font-size: 12px;
  }

  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-size: 13px;
  }

  .qr {
    width: 168px;
    margin: 8px auto;
    padding: 6px;
    border-radius: 8px;
    /* QR codes need dark-on-light to scan reliably, even in dark mode. */
    background: #fff;
    color: #000;
  }

  .qr :global(svg) {
    display: block;
    width: 100%;
    height: auto;
  }

  .steps {
    margin: 0;
    padding-left: 18px;
    display: grid;
    gap: 4px;
  }

  .address {
    display: block;
    margin-top: 8px;
  }

  select {
    margin-left: 4px;
    font: inherit;
  }
</style>

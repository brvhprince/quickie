<script lang="ts">
  import { onMount } from "svelte";
  import ItemCard from "../src/lib/ItemCard.svelte";
  import { bytes } from "../src/lib/format";
  import { ownerLabel } from "../src/lib/owner";
  import type { WebSnapshot } from "../src/lib/types";
  import { Unpaired, follow, web, webActions, type LinkStatus } from "../src/lib/webClient";

  type Phase = "loading" | "naming" | "unpaired" | "ready";

  let phase = $state<Phase>("loading");
  let snapshot = $state<WebSnapshot | null>(null);
  let status = $state<LinkStatus>("reconnecting");
  let secret = "";
  let name = $state(guessDeviceName());
  let draft = $state("");
  let notice = $state("");
  let now = $state(Date.now());
  let uploads = $state<{ key: number; name: string; size: number; sent: number }[]>([]);
  let stopFollowing: (() => void) | null = null;

  const names = $derived(new Map(snapshot?.devices.map((d) => [d.id, d.name] as const) ?? []));
  const others = $derived(snapshot?.devices.filter((d) => d.id !== snapshot?.host.id) ?? []);

  onMount(() => {
    // The QR code carries a one-time secret in the fragment, which never reaches
    // the server in a URL. Take it out of the address bar straight away.
    const fromQr = new URLSearchParams(location.hash.slice(1)).get("pair");
    if (fromQr) {
      secret = fromQr;
      history.replaceState(null, "", location.pathname);
      phase = "naming";
    } else {
      connect();
    }
    const tick = setInterval(() => (now = Date.now()), 15_000);
    return () => {
      clearInterval(tick);
      stopFollowing?.();
    };
  });

  function guessDeviceName() {
    const ua = navigator.userAgent;
    if (/iPhone/.test(ua)) return "iPhone";
    if (/iPad/.test(ua) || (/Macintosh/.test(ua) && navigator.maxTouchPoints > 1)) return "iPad";
    if (/Android/.test(ua)) return /Mobile/.test(ua) ? "Android phone" : "Android tablet";
    return "Browser";
  }

  const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

  async function connect() {
    try {
      snapshot = await web.shelf();
      phase = "ready";
      stopFollowing = follow(
        (s) => (snapshot = s),
        () => {
          snapshot = null;
          phase = "unpaired";
        },
        (s) => (status = s),
      );
    } catch (e) {
      if (!(e instanceof Unpaired)) notice = message(e);
      phase = "unpaired";
    }
  }

  async function pair(e: SubmitEvent) {
    e.preventDefault();
    notice = "";
    try {
      await web.pair(secret, name);
      await connect();
    } catch (err) {
      notice = message(err);
      phase = "unpaired";
    }
  }

  async function share(text: string) {
    notice = "";
    try {
      await web.shareText(text);
      return true;
    } catch (e) {
      notice = message(e);
      return false;
    }
  }

  async function shareDraft() {
    if (await share(draft)) draft = "";
  }

  async function pasteAndShare() {
    try {
      const text = await navigator.clipboard.readText();
      if (text) await share(text);
    } catch {
      notice = "Your browser didn't allow reading the clipboard. Paste into the box instead.";
    }
  }

  let nextKey = 0;
  async function send(e: Event) {
    const input = e.currentTarget as HTMLInputElement;
    const files = [...(input.files ?? [])];
    input.value = "";
    notice = "";
    // One at a time keeps a phone's upload bandwidth on the file you'll get first.
    for (const file of files) {
      const entry = { key: nextKey++, name: file.name, size: file.size, sent: 0 };
      uploads.push(entry);
      try {
        await web.upload(file, (sent) => {
          const live = uploads.find((u) => u.key === entry.key);
          if (live) live.sent = sent;
        });
      } catch (err) {
        notice = message(err);
      } finally {
        uploads = uploads.filter((u) => u.key !== entry.key);
      }
    }
  }

  async function disconnect() {
    stopFollowing?.();
    await web.logout().catch(() => {});
    snapshot = null;
    phase = "unpaired";
  }
</script>

<main>
  <header>
    <div class="brand">
      <span class="logo" aria-hidden="true"></span>
      <strong>Quickie</strong>
    </div>
    {#if phase === "ready" && snapshot}
      <span class="status {status}">
        {status === "live" ? `Connected to ${snapshot.host.name}` : status === "polling" ? `Connected to ${snapshot.host.name} (slow mode)` : "Reconnecting…"}
      </span>
    {/if}
  </header>

  {#if phase === "loading"}
    <p class="muted center">Connecting…</p>
  {:else if phase === "naming"}
    <form class="card" onsubmit={pair}>
      <h1>Pair this phone</h1>
      <label for="name">Name it so your other devices know where things came from:</label>
      <input id="name" bind:value={name} maxlength="40" autocomplete="off" />
      <button class="primary big">Pair</button>
    </form>
  {:else if phase === "unpaired"}
    <div class="card">
      <h1>This phone isn't paired</h1>
      <p>On your computer, open Quickie, choose <b>Add phone</b>, and scan the QR code with this phone's camera.</p>
    </div>
  {:else if snapshot}
    {#if others.length > 0}
      <div class="devices">
        {#each others as device (device.id)}
          <span class="chip" class:offline={!device.online}><i class="dot"></i>{device.name}</span>
        {/each}
      </div>
    {/if}

    <section class="composer">
      <textarea bind:value={draft} rows="3" placeholder="Type or paste text to share"></textarea>
      <div class="row">
        <button class="ghost" onclick={pasteAndShare}>Paste & share</button>
        <button class="primary" onclick={shareDraft} disabled={!draft.trim()}>Share</button>
      </div>
      <label class="send">
        <input type="file" multiple onchange={send} />
        Send photos or files
      </label>
    </section>

    {#if uploads.length}
      <ul class="uploads">
        {#each uploads as upload (upload.key)}
          <li>
            <div class="row"><span class="name">{upload.name}</span><span class="muted">{bytes(upload.sent)} / {bytes(upload.size)}</span></div>
            <div class="progress"><div class="bar" style:width="{upload.size ? (upload.sent / upload.size) * 100 : 100}%"></div></div>
          </li>
        {/each}
      </ul>
    {/if}

    <ul class="items">
      {#each snapshot.items as item (item.id)}
        <ItemCard
          {item}
          {now}
          actions={webActions}
          owner={ownerLabel(item, names, snapshot.you.id)}
          mine={item.via?.id === snapshot.you.id}
          transfer={snapshot.transfers[item.id]}
          onerror={(m) => (notice = m)}
        />
      {:else}
        <li class="muted center">Nothing on the shelf yet. Anything you share here shows up on your computers.</li>
      {/each}
    </ul>

    <footer class="muted">
      Shared items disappear after 24 hours. <button class="link" onclick={disconnect}>Unpair this phone</button>
    </footer>
  {/if}

  {#if notice}<p class="notice" role="alert">{notice}</p>{/if}
</main>

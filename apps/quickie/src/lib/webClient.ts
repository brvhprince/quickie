// The phone web client's view of the shelf, served by the desktop it paired with.
// Auth is an HttpOnly session cookie, so nothing here touches credentials.

import type { ItemActions } from "./actions";
import type { WebSnapshot } from "./types";

export class Unpaired extends Error {
  constructor() {
    super("This phone isn't paired");
  }
}

async function call(path: string, init?: RequestInit): Promise<Response> {
  const res = await fetch(path, { credentials: "same-origin", ...init });
  if (res.status === 401) throw new Unpaired();
  if (!res.ok) throw new Error((await res.text()) || `Request failed (${res.status})`);
  return res;
}

const json = (body: unknown): RequestInit => ({
  method: "POST",
  headers: { "Content-Type": "application/json" },
  body: JSON.stringify(body),
});

export const contentUrl = (id: string, inline = false) =>
  `/v1/web/items/${encodeURIComponent(id)}/content${inline ? "?inline=1" : ""}`;

export const web = {
  shelf: async () => (await call("/v1/web/shelf")).json() as Promise<WebSnapshot>,
  pair: async (secret: string, name: string) => {
    await call("/v1/web/pair", json({ secret, name }));
  },
  shareText: async (text: string) => {
    await call("/v1/web/text", json({ text }));
  },
  remove: async (id: string) => {
    await call(`/v1/web/items/${encodeURIComponent(id)}`, { method: "DELETE" });
  },
  logout: async () => {
    await call("/v1/web/logout", { method: "POST" });
  },
  /** XHR rather than fetch, for upload progress. */
  upload: (file: File, onProgress: (sent: number) => void) =>
    new Promise<void>((resolve, reject) => {
      const xhr = new XMLHttpRequest();
      xhr.open("POST", `/v1/web/files?name=${encodeURIComponent(file.name)}`);
      xhr.setRequestHeader("Content-Type", "application/octet-stream");
      xhr.upload.onprogress = (e) => onProgress(e.loaded);
      xhr.onload = () =>
        xhr.status < 300 ? resolve() : reject(new Error(xhr.responseText || `Upload failed (${xhr.status})`));
      xhr.onerror = () => reject(new Error(`Upload of ${file.name} failed: connection lost`));
      xhr.send(file);
    }),
};

export type LinkStatus = "live" | "polling" | "reconnecting";

/**
 * Follows the shelf over a WebSocket, reconnecting with backoff. Some browsers
 * (notably Safari) refuse WebSockets to a self-signed certificate even after the
 * page itself was accepted, so if the socket never opens we poll instead.
 * Returns a function that stops following.
 */
export function follow(
  onSnapshot: (snapshot: WebSnapshot) => void,
  onUnpaired: () => void,
  onStatus: (status: LinkStatus) => void,
): () => void {
  let stopped = false;
  let socket: WebSocket | null = null;
  let everOpened = false;
  let failures = 0;
  let retry: ReturnType<typeof setTimeout> | undefined;
  let poll: ReturnType<typeof setInterval> | undefined;

  const unpaired = () => {
    stop();
    onUnpaired();
  };

  async function refresh() {
    try {
      onSnapshot(await web.shelf());
      return true;
    } catch (e) {
      if (e instanceof Unpaired) unpaired();
      return false;
    }
  }

  function startPolling() {
    onStatus("polling");
    refresh();
    poll = setInterval(refresh, 2000);
  }

  function open() {
    if (stopped) return;
    socket = new WebSocket(`wss://${location.host}/v1/web/events`);
    socket.onopen = () => {
      everOpened = true;
      failures = 0;
      onStatus("live");
    };
    socket.onmessage = (e) => onSnapshot(JSON.parse(e.data));
    socket.onclose = async (e) => {
      socket = null;
      if (stopped) return;
      if (e.code === 4001) return unpaired();
      failures++;
      // A refused upgrade looks the same as a dead network; ask over plain HTTP.
      const reachable = await refresh();
      if (stopped) return;
      if (reachable && !everOpened && failures >= 2) return startPolling();
      onStatus("reconnecting");
      retry = setTimeout(open, Math.min(30_000, 1000 * 2 ** Math.min(failures, 5)));
    };
  }

  // Phones suspend background tabs; catch up as soon as we're visible again.
  function onVisible() {
    if (document.visibilityState !== "visible" || stopped) return;
    if (poll !== undefined) refresh();
    else if (!socket) {
      clearTimeout(retry);
      open();
    }
  }

  function stop() {
    stopped = true;
    clearTimeout(retry);
    clearInterval(poll);
    socket?.close();
    document.removeEventListener("visibilitychange", onVisible);
  }

  document.addEventListener("visibilitychange", onVisible);
  open();
  return stop;
}

/** Clipboard API where allowed, else the old select-and-copy trick. */
async function copyText(text: string) {
  if (navigator.clipboard && window.isSecureContext) {
    try {
      await navigator.clipboard.writeText(text);
      return;
    } catch {
      // Fall through.
    }
  }
  const area = document.createElement("textarea");
  area.value = text;
  area.setAttribute("readonly", "");
  area.style.cssText = "position:fixed;opacity:0;top:0";
  document.body.append(area);
  area.select();
  area.setSelectionRange(0, text.length);
  const copied = document.execCommand("copy");
  area.remove();
  if (!copied) throw new Error("Couldn't copy. Press and hold the text to copy it instead.");
}

export const webActions: ItemActions = {
  copyText,
  // Folders arrive as a zip, which a browser can't show; "Download" covers it.
  canOpen: (item) => item.kind === "file",
  open: async (item) => {
    window.open(contentUrl(item.id, true), "_blank", "noopener");
  },
  save: async (item) => {
    const link = document.createElement("a");
    link.href = contentUrl(item.id);
    link.download = item.kind === "folder" ? `${item.name}.zip` : item.name;
    link.click();
  },
  saveLabel: "Download",
  remove: (item) => web.remove(item.id),
};

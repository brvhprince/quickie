# Quickie

A floating shelf for sharing text and files across your devices on the home network.
See [docs/plan.md](docs/plan.md) for the design and roadmap.

## Run

```sh
pnpm install
pnpm dev            # builds the phone web client, then runs the Tauri app
```

The phone web client (`apps/quickie/web`) is bundled into the app from `apps/quickie/dist-web`.
A plain `cargo build` needs that folder to exist, so run `pnpm --filter quickie build:web` once
after cloning. To work on the phone UI, run `pnpm --filter quickie dev:web` (rebuilds on save) and
start the app with `QUICKIE_WEB_ROOT=apps/quickie/dist-web`, then reload the phone page.

- **Pair** once per device: open the shelf, click a device under *Nearby*, and type the 6-digit
  code it shows. Only paired devices can see your shelf.
- **Phones** (no app needed): *Add phone* shows a QR code. Scan it, accept the browser's
  certificate warning once (expected on a home network: there's no certificate authority), name the
  phone, done. The phone sees the whole shelf through that computer and can share text, photos and
  files; the computer hosts what the phone shares.
- **Share**: type or paste text, ⌘V anywhere in the open shelf, or drop files and folders on the
  bubble. Files copied in Finder / Explorer can be pasted too. Nothing is copied up front; other
  devices pull files only when opened, saved or dragged out (folders arrive as folders; phones get a
  zip).
- **Unmount** takes an item off every device. Items expire on their own after 24h.
- Click the bubble or press ⌃⌥Space (Ctrl+Alt+Space) to open the shelf; drag the bubble to move it.
  Esc or clicking elsewhere tucks it away. Quit and *Open at Login* are in the menu-bar icon.

### A second device on the same machine

While `pnpm dev` is running:

```sh
QUICKIE_INSTANCE=b target/debug/quickie
```

Or a headless node (type lines to share; `/ask`, `/pair`, `/file`, `/get`, `/rm`, `/ls`;
see the top of `crates/quickie-core/examples/node.rs`):

```sh
cargo run -p quickie-core --example node -- "Test Phone"
```

## Test

```sh
cargo test --workspace       # real nodes over mDNS: pairing, transfers, and the phone web API
pnpm --filter quickie check  # Svelte/TypeScript
```

## Layout

```
crates/quickie-core   Rust: discovery, peer protocol, shelf state
apps/quickie          Tauri 2 + Svelte 5 desktop app (mobile targets later)
docs/plan.md          Design and milestones
```

## Protocol (v1)

TLS 1.3 only. Every device has a self-signed certificate; its id is derived from the certificate,
and clients pin the id they dial. Servers accept any client certificate and check per route.

| Route | Who | |
|---|---|---|
| `GET /v1/hello` | anyone | device id, name, platform |
| `POST /v1/pair/request` | any device cert | ask this device to show a pairing code |
| `POST /v1/pair/start`, `/v1/pair/confirm` | the requesting device | SPAKE2 exchange + key confirmation |
| `GET /v1/items` | paired | this device's own items |
| `GET /v1/items/{id}/content` | paired | file bytes, with Range for resume |
| `WS /v1/events` | paired | `snapshot`, then `added` / `removed` for this device's own items |

Phone browsers can't present a certificate, so they use a separate API under `/v1/web/*`,
authenticated by an HttpOnly, `SameSite=Strict` session cookie obtained by redeeming the QR code's
one-time secret (`POST /v1/web/pair`). Requests with a foreign `Origin` are refused. Shared files
are served with `Content-Security-Policy: sandbox` so nothing shared can script the web client. The
app itself is served at `/`.

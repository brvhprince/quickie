# Quickie

A floating shelf for sharing text and files across your devices on the home network.
See [docs/plan.md](docs/plan.md) for the design and roadmap.

## Run

```sh
pnpm install
pnpm dev            # Tauri dev build; the bubble appears on the right edge of the screen
```

- **Pair** once per device: open the shelf, click a device under *Nearby*, and type the 6-digit
  code it shows. Only paired devices can see your shelf.
- **Share**: type or paste text, ⌘V anywhere in the open shelf, or drop files on the bubble.
  Files stay where they are; other devices pull them only when opened, saved or dragged out.
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
cargo test --workspace       # includes real two-node pairing/transfer tests over mDNS
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

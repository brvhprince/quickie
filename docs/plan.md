# Quickie plan

A floating shelf for moving files and text between a Mac, a Windows PC, an iPhone and an
Android phone on the **home network**. Quick and ephemeral, not a sync service.

## Model

- **Mount** puts a reference on the shelf. The bytes stay on the owner's device; others pull on demand.
- **Unmount** takes it off every shelf. Copies already pulled by someone stay with them.
- **Each device is authoritative for its own items.** The shelf is the union of all online
  peers' items, so there is nothing to merge. A peer going offline removes its items.
- **Phones can't serve reliably** (iOS suspends background apps), so phones *push* copies to an
  online desktop instead of mounting by reference.

## Stack

- `crates/quickie-core` (Rust): mDNS discovery (`_quickie._tcp`), axum over TLS 1.3 on port
  47800 (falls back to a random port), peer following with reconnect/backoff.
- `apps/quickie` (Tauri 2 + Svelte 5): floating bubble that expands into the shelf panel, tray menu.
- Components talk to a `ShelfClient` interface so the phone web client can reuse them over HTTP.

## Milestones

- [x] **M0**: discovery + live text shelf between desktops, bubble UI, unmount.
- [x] **M1 desktop MVP**: pairing with a 6-digit code (SPAKE2) + TLS with pinned certs; file
  drop onto bubble/shelf, on-demand fetch with HTTP Range resume, drag-out (`tauri-plugin-drag`,
  copy mode), open / save to Downloads, 24h TTL expiry, ⌃⌥Space shortcut, open at login,
  single-instance.
- [ ] **M1 leftovers**: folders (zip on the fly?), pasting files copied in Finder/Explorer,
  `tauri-nspanel` so the bubble doesn't steal focus on macOS, Windows test pass.
- [ ] **M2 phone web client**: desktop serves the Svelte UI; phone scans a QR code (URL + token);
  upload, download, copy text. Plain HTTP on the LAN, so no service worker and an `execCommand`
  copy fallback.
- [ ] **M3 Tauri mobile**: same core + UI on iOS/Android; share-sheet target, native discovery.
- [ ] **M4 OS-level mount** (optional): localhost WebDAV gateway → Windows Cloud Files API /
  macOS File Provider.

## Decisions

- Home network only for now. If "works anywhere" is wanted later, swap the transport for iroh
  (QUIC, dial by public key, relays), which drops into the Rust core.
- M2 (web client) before M3 (native mobile).
- IPv4 only for peer connections (IPv6 link-local needs scope ids in URLs).
- Device identity is a self-signed **ECDSA P-256** cert; the device id is the first 128 bits of
  its SHA-256. P-256, not Ed25519, because browsers (M2) don't accept Ed25519 in TLS.
- Pairing uses SPAKE2 so an attacker on the LAN gets one online guess per run; the responder
  allows 3 runs per code. Confirmation HMACs cover both TLS-verified device ids.
- Downloaded copies of other devices' items live in a cache that is wiped on startup and when the
  item leaves the shelf. "Save to Downloads" makes a copy the user keeps.

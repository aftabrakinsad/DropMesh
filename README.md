# DropMesh

Peer-to-peer file sharing across your own devices. No cloud, no accounts, no servers.

Files transfer directly over your local network, encrypted end to end. If the
target device is offline, the file waits in a local queue and delivers itself
the moment that device reappears.

> **Status: early development.** The core transfer path works, but several
> security and reliability items are unfinished — see
> [Known limitations](#known-limitations) before relying on this for anything
> that matters.

---

## How it works

Each device holds a long-term X25519 identity keypair. Pairing two devices
exchanges those identities over the local network, authenticated by a 6-digit
code, and both sides confirm a matching 4-emoji fingerprint.

Devices find each other over mDNS (`_dropmesh._udp.local.`) — no IP addresses,
no configuration. Transfers run over QUIC, which brings TLS 1.3 to every
connection. On top of that, each transfer derives its own session key, so
compromising the long-term key does not expose past transfers. Every file
carries an HMAC proving it came from a paired device, and a SHA-256 hash
proving it arrived intact. A file failing either check is discarded, not saved.

When you send to an offline device, the file is staged in your own allocated
storage and queued. A background poller watches for that device to appear and
delivers automatically. Queue entries expire after 7 days.

---

## Requirements

- [Rust](https://rustup.rs/) (stable)
- [Node.js](https://nodejs.org/) 18+
- Platform build tools for Tauri — see the
  [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/)

## Running

```bash
git clone https://github.com/<your-username>/dropmesh.git
cd dropmesh
npm install
cargo tauri dev
```

The first build compiles ~600 crates and takes a few minutes. Subsequent
builds are fast.

## Pairing two devices

Both devices must be on the same Wi-Fi network.

1. On device A, tap **+ Add device** — a 6-digit code appears
2. On device B, find device A under **Nearby** and tap it
3. Enter the code and tap **Connect**
4. Both devices show 4 emojis — confirm they match on both screens

Once paired, select the device and drop files onto the drop zone.

---

## Testing with two instances on one machine

Useful during development. Copy the project, then change two values in the copy:

- `src-tauri/tauri.conf.json` → `identifier` (so it gets its own database)
- `vite.config.js` → `server.port`, and `devUrl` in `tauri.conf.json` to match

Run each from its own terminal.

---

## Architecture

```
src/                        Svelte 5 frontend
  App.svelte                UI: devices, queue, history, pairing
src-tauri/src/
  commands/                 Tauri IPC handlers
  services/
    discovery.rs            mDNS advertise + browse, live peer registry
    pairing_handshake.rs    Pairing wire protocol
    security.rs             Identity, key derivation, AES-GCM, HMAC
    transfer.rs             QUIC listener, chunked send/receive
    queue.rs                Async file queue, staging, expiry
    delivery_watcher.rs     Flushes the queue when peers appear
    storage.rs              Allocated folder + quota
  db/                       SQLite schema and migrations
  models/                   Shared data types
```

---

## Known limitations

These are real and worth understanding before trusting the project:

**Large files exhaust memory.** The sender reads the entire file into RAM and
the receiver buffers every chunk before writing to disk. The UI permits files
up to 2 GB, which will consume gigabytes on both ends. Streaming I/O is the
next planned change.

**TLS certificates are not verified.** `build_client_config` accepts any
certificate. Authenticity currently rests on the pairing code and the per-file
HMAC, which is meaningful protection, but pinning each peer's certificate at
pairing time is not yet implemented.

**Pairing is not brute-force resistant.** The group key derives from the 6-digit
PIN via HKDF. Anyone who captures the handshake can try all 900,000 codes
offline. SPAKE2 is a dependency and stubbed in `security.rs` but not yet wired
into the flow; it would make the PIN secure by requiring a live connection per
attempt.

**Private keys are not in the keychain.** `generate_identity` writes a
placeholder rather than the real private key.

**Progress reporting is cosmetic.** The progress bar animates on a timer.
`TransferProgress` is defined but never emitted, so a stalled transfer looks
identical to a fast one.

**Key rotation is partial.** Removing a device rotates the group key locally,
but the new key is not propagated to remaining paired devices.

**Desktop only so far.** Tauri supports mobile, but iOS and Android have not
been built or tested.

---

## Roadmap

- [x] Device identity and local database
- [x] Async file queue with staging and expiry
- [x] mDNS discovery
- [x] Pairing with emoji verification
- [x] QUIC file transfer with encryption and authentication
- [ ] Streaming I/O for large files
- [ ] Real transfer progress
- [ ] Certificate pinning
- [ ] SPAKE2 pairing
- [ ] Keychain-backed private keys
- [ ] System tray and notifications
- [ ] Mobile builds

---

## Security reporting

This project has not had a security review. If you find a vulnerability, please
open an issue or contact the maintainer directly rather than disclosing it
publicly.

## License

MIT — see [LICENSE](LICENSE).
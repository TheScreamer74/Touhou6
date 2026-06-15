# Web build (private beta)

Runs the engine in the browser via WebAssembly. **No game assets are bundled
or served** — each visitor uploads their own `th06` folder, and those bytes
stay in their browser (read locally, never sent to any server).

## Build

Needs [`wasm-pack`](https://rustwasm.github.io/wasm-pack/):

```sh
cargo install wasm-pack          # once

# from the repo root (touhou6/):
wasm-pack build crates/game --release --target web --out-dir ../../web/pkg
```

This produces `web/pkg/th06.js` + `web/pkg/th06_bg.wasm`, which
`web/index.html` imports.

## Run locally

The module must be served over HTTP (ES-module imports + wasm MIME), not
opened as a `file://`:

```sh
cd web && python3 -m http.server 8080
# open http://localhost:8080
```

Select your game folder (the one with `TL.DAT`, `CM.DAT`, `ST.DAT`,
`IN.DAT`, `th06e_ST.DAT` and `bgm/`).

## Invite key

`index.html` has a client-side invite gate. Set the hash of your chosen key:

```sh
printf '%s' 'your-secret-key' | shasum -a 256
```

Paste the hex digest into `INVITE_HASH` in `index.html`.

**This is not real access control.** Anyone who can load the page can read
the JS and bypass the check. It only keeps the beta low-profile. For genuine
gating, enforce it on the server that hosts these files (HTTP basic auth,
signed/expiring URLs, or an auth proxy). Until `INVITE_HASH` is set the gate
lets everyone through (for local testing).

## Hosting

Upload `web/index.html` and `web/pkg/` to any static host. Serve `.wasm`
with `Content-Type: application/wasm`. That is the entire deployment — there
are no game files on the server.

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

## Access

There is no login. Access is implicit: only someone who already owns the
game files can play (the page is useless without them), and nothing is
uploaded. If you want to keep the beta private anyway, restrict it at the
host (HTTP basic auth, signed/expiring URLs) — not in the page.

## Hosting

Upload `web/index.html` and `web/pkg/` to any static host. Serve `.wasm`
with `Content-Type: application/wasm`. That is the entire deployment — there
are no game files on the server.

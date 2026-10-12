# Deployed build — 2026-10-12

- Public URL: https://games.blackoutburst.com/th06/
- Build: `full-game-20261012-eef7cca16643`
- Code commit: `fa9985abd269c6cb0cba356ec3e4bfaa0e9c6229`
- WASM SHA256: `eef7cca166435eadb1912e42800bdb087b4e030e876c037d38cefb387de4dc7b`
- Release directory: `/home/ubuntu/touhou6-web/releases/full-game-20261012-eef7cca16643`
- Published link: `/home/ubuntu/Touhou6-Mac/web/th06`
- Previous deployment: `/home/ubuntu/touhou6-web/releases/legacy-20261012-083504`
- Source archive: `/home/ubuntu/touhou6-web/source/touhou6-fa9985a-source.tar.gz`
- Server receipt: `/home/ubuntu/touhou6-web/deployment.json`

The existing dirty server checkout was preserved. The release was transferred
separately, verified with SHA256SUMS, and exercised under
`/th06-preview-20261012/`. Linux `renameat2(RENAME_EXCHANGE)` switched the initial
directory to the new symlink atomically. Other game routes were unchanged.
Future releases can atomically replace the published symlink after validation.

The hosted set includes seven archives and 17 Ogg tracks converted from the
provided WAVs at their original sample rate. No original executable is hosted.
`res` and `bgm` aliases retain the old asset URLs. Assets remain outside Git.

HTTPS checks returned 200 for build.json, the WASM, TH04 and the portal. The
WASM was served as `application/wasm` and its downloaded hash matched the local
build. Browser checks covered startup, Hard/Reimu B selection, a game-over
Result flow, saving a T6RP replay and seeing it again after reload. The six
endings and 96 stage routes are covered by the separate original-asset checks
in [the validation report](REVERSE_ENGINEERING.md).

## Roll back

On the server, create a temporary symlink and replace the published symlink in
one operation. The previous release and its asset links remain available:

```python
import os
from pathlib import Path
root = Path('/home/ubuntu/Touhou6-Mac/web')
backup = Path('/home/ubuntu/touhou6-web/releases/legacy-20261012-083504')
next_link = root / '.th06-rollback'
assert backup.is_dir()
assert not next_link.exists() and not next_link.is_symlink()
next_link.symlink_to(backup)
os.replace(next_link, root / 'th06')
```

## Browser updates

New builds must change `build.json.version` after uploading both JS and WASM.
Use a hard reload once if the browser retained the previous HTML. The link
`https://games.blackoutburst.com/th06/?v=eef7cca16643` also bypasses that older
HTML cache. Score/replay storage keys and the personal asset cache are retained.

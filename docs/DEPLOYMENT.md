# Deployed build — 2026-10-12

- Public URL: https://games.blackoutburst.com/th06/?v=3ac820702f24
- Build: `fidelity-20261012-3ac820702f24`
- Code commit: `4a8d092b690929a7804eeab678ca307cfdcb39d9`
- WASM SHA256: `3ac820702f24fdeca7dbdf17061f14d00392f10cc525334b42a2ec31aa3f95e6`
- Release directory: `/home/ubuntu/touhou6-web/releases/fidelity-20261012-3ac820702f24`
- Published link: `/home/ubuntu/Touhou6-Mac/web/th06`
- Previous release: `/home/ubuntu/touhou6-web/releases/music-loop-20261012-5f7145f2299d`
- Initial legacy backup: `/home/ubuntu/touhou6-web/releases/legacy-20261012-083504`
- Source archive: `/home/ubuntu/touhou6-web/source/touhou6-4a8d092-source.tar.gz`
- Source archive SHA256: `5b41573aae02d702ebaed25e02ea86405369e8666bc31e0412a750f6d5ba6946`
- Server receipt: `/home/ubuntu/touhou6-web/deployment.json`

The existing dirty server checkout was preserved. The separate release passed
all SHA256SUMS before publication. Browser startup was checked at the HTTPS
preview, then the published symlink was replaced atomically with `os.replace`.
A post-switch check confirms the downloaded WASM hash and `application/wasm`
content type, Worklet JavaScript and full SoundFont license. TH04 and the portal
still return HTTP 200; their routes were unchanged. All previous releases remain
available. [Deployment receipt](deployment-fidelity-2026-10-12.json).

The hosted set contains seven game archives, 17 Ogg tracks retaining the original
WAV sample rates/frame counts, and a separate TimGM6mb General MIDI SoundFont.
The bank's attribution and full GPL-2 license are served beside it. Original
executables, MS Gothic and the Windows GS bank are not hosted. Game assets and
the freely licensed bank remain outside Git. `res` and `bgm` aliases preserve the
previous asset paths. `midi-worklet.js` must deploy beside `index.html` and `pkg/`.

The final browser check exercised 16-bit rendering, Normal/Marisa B selection,
shots and Master Spark, plus saved-MIDI cold startup. HTTPS preview and public
startup both loaded all 25 assets and rendered with no observed console errors
or warnings. The original title demo runs on the public build. These browser
checks are distinct from the 96-route internal replay regression and the 3,720
original-executable observations in [the validation report](REVERSE_ENGINEERING.md).

The original WAV/Ogg loop correction remains active, including the 48 kHz
resampling case. [Loop receipt](audio-loop-2026-10-12.json),
[MIDI worklet receipt](midi-worklet-2026-10-12.json).

## Roll back

Create a temporary symlink to the previous release and replace the current
symlink in one operation. No release data needs to be removed:

```python
import os
from pathlib import Path
root = Path('/home/ubuntu/Touhou6-Mac/web')
backup = Path('/home/ubuntu/touhou6-web/releases/music-loop-20261012-5f7145f2299d')
next_link = root / '.th06-rollback'
assert backup.is_dir()
assert not next_link.exists() and not next_link.is_symlink()
next_link.symlink_to(backup)
os.replace(next_link, root / 'th06')
```

## Browser updates

New builds must change `build.json.version` after uploading JS, WASM and the MIDI
worklet. The public link above bypasses older cached HTML; a hard reload also
refreshes it. Score/replay storage and the personal asset cache are retained.

# Deployed build — 2026-10-14

- Public URL: https://games.blackoutburst.com/th06/
- Build: `community-20261014-747d6d7cbed0`
- Runtime code commit: `352966da3d33fab59913dc0fdd7caac401220e25`
- WASM SHA256: `747d6d7cbed03bed4cf7c78666c44da20d869d9fceb7127d900d399cf2ab1b69`
- Release: `/home/ubuntu/touhou6-web/releases/community-20261014-747d6d7cbed0`
- Published link: `/home/ubuntu/Touhou6-Mac/web/th06`
- Previous release: `/home/ubuntu/touhou6-web/releases/fidelity-20261014-autostart-0cb6025d5b09`
- Source archive: `/home/ubuntu/touhou6-web/source/touhou6-352966da3d33-source.tar.gz`
- Source archive SHA256: `2916c92013579297db38d003d8cce0a73bb7fc5f9a502f416b7bb596661cfa68`

The release adds the requested language and hitbox options, corrects text alpha
composition, and removes Exit from the browser title menu. See
[port extensions](PORT_EXTENSIONS.md) and the
[extension validation receipt](extension-validation-2026-10-14.json).

Publication replaced only the game symlink atomically after checksum and browser
checks. The existing server checkout and previous releases remain available.
HTTPS checks downloaded and hashed all six release files, checked all 25 declared
resources, and confirmed TH04 and the portal still return 200. See the
[publication receipt](deployment-extensions-2026-10-14.json).

The published game also passed a short Firefox check: automatic startup, attract
gameplay, seven title entries without Exit, and legible title text. The saved
Original language preference remained intact. Local preview checks exercised the
extended Options menu and keyboard recovery after an external window closes.
Native CUA exposed screenshots and keys, so these checks do not claim DOM or
console inspection, a controlled performance measurement, or a full playthrough.

The page starts automatically after downloading `assets.json` and its resources.
The first key press or click unlocks browser audio. Existing browser options,
scores and replays are retained. The independent hosting interface is optional;
its implementation and deployment records are outside this repository.

The hosted set contains seven game archives, 17 Ogg tracks retaining original
WAV sample rates/frame counts, and a separate TimGM6mb General MIDI SoundFont.
The bank's attribution and full GPL-2 license are served beside it. Original
executables, MS Gothic and the Windows GS bank are not hosted. Game assets and
the separate bank remain outside Git. `res` and `bgm` aliases preserve older
asset paths. `midi-worklet.js` deploys beside `index.html` and `pkg/`.

The music loop correction remains in the source, including the 48 kHz resampling
case. The new extension checks compare ending music/fade requests between
languages; they do not repeat the historical PCM or original replay campaigns.
Those earlier results retain their recorded builds in the
[audio loop receipt](audio-loop-2026-10-12.json),
[AudioWorklet receipt](midi-worklet-2026-10-13.json), and
[reference report](REVERSE_ENGINEERING.md).

## Roll back

Create a temporary symlink to the retained previous release, then replace the
public symlink. No release data needs to be removed:

```python
import os
from pathlib import Path
root = Path('/home/ubuntu/Touhou6-Mac/web')
previous = Path('/home/ubuntu/touhou6-web/releases/fidelity-20261014-autostart-0cb6025d5b09')
temporary = root / '.th06-rollback'
assert previous.is_dir()
assert not temporary.exists() and not temporary.is_symlink()
temporary.symlink_to(previous)
os.replace(temporary, root / 'th06')
```

## Browser updates

Change `build.json.version` after uploading the complete immutable release.
The page uses that version for JS, WASM and worklet URLs. The origin currently
sends Last-Modified for HTML; a normal reload revalidates a cached page. Score,
replay and option storage remain independent of the release version.

Earlier deployments are preserved in
[the October 12 receipt](deployment-fidelity-2026-10-12.json) and
[the October 14 runtime/autostart receipt](deployment-2026-10-14.json).

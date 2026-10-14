The four UTF-8 JSON catalogs cover French, English, German and Dutch. Japanese
remains the original game data and can be selected separately.

Most keys are exact source strings decoded from Windows Shift-JIS. Preserve
leading/trailing spaces when editing a key. `ui.*`, `menu.*`, `options.*` and
the other dotted keys identify text originally drawn inside menu images.
Both bare music titles and their `♪` variants are included.

Endings also have occurrence keys: `end:<lowercase filename>:<byte end offset>`.
The offset is the VM's program counter immediately after the original text
chunk, before its NUL terminator. Each original 32-byte chunk has its own
translation. This preserves script timing and resolves repeated fragments such
as a closing quote without moving their meaning into another scene.

The English base comes from the version 0.8 English patch in the user's game
archive (its readme credits TouhouWiki, 2005-11-17, and patch author S"). Spell
names are translated from the original Japanese because that patch retains
Japanese names in ECL. French, German and Dutch are authored for this port.
Proper character, author and organization names are preserved.

Run `scripts/audit_locales.py` against locally extracted ST, ED and MD data to
check every dialogue entry, spell name, stage/music title, Music Room comment,
ending line/chunk/occurrence and source UI key. This verifies coverage and
encoding. It does not establish human linguistic review or approve every
rendered text layout. Source archives and extracted original assets stay
outside the repository.

# Touhou6 — project rule

> **Nothing should be approximated, everything should be taken from the decomp and reimplemented as is.**

The decompilation at `../refs/th06-decomp/` (namespace `th06`, EoSD 1.02h) is the
authoritative ground truth. For every element:

1. Find the exact decomp source first (positions, colours, constants, formulas, assets).
2. Port the real assets verbatim — front.anm/etc. sprites, lookup tables (e.g.
   `g_SpellcardScore`), AsciiManager metrics (15px glyph / 14px advance) — never an
   invented rect, label, or "close enough" value.
3. If the engine can't express something exactly (e.g. the single-tint `DrawCmd` vs the
   power bar's per-vertex gradient quad), **extend the engine** rather than approximate.
4. Use YouTube footage only to validate. If a difference is found and the right answer is
   unclear, ask.

Japanese text (2026-07-03): a Japanese font subsystem exists (`engine::text`:
Shift-JIS decode + `ab_glyph` rasterisation of bundled Noto Sans JP, reproducing
`TextHelper::RenderTextToTexture`). All gameplay-visible text now renders the
original Japanese via pre-baked textures: **spellcard names** (ecl op93),
**dialogue** (ST.DAT msg), **stage names** (STD name[128]). Only **song names**
(STD songNames[4][128]) remain on hold — they need a Music Room menu, which the
port doesn't have yet.

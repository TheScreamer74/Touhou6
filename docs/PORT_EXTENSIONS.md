# Requested port extensions

These features are optional additions to the original game presentation.
The original script bytes, waits, collision rules and replay inputs are retained.
The Options menu now has eleven rows, so historical nine-row menu navigation
receipts must not be described as validation of this extended menu.

## Languages

Options selects Japanese, French, English, German or Dutch. The four catalogs
contain 1,535 entries each. The source audit currently requires 1,386 unique
keys spanning every dialogue branch, spell name, stage/music title, Music Room
comment, six endings and UI captions; none are missing. See the dated
localization receipt for source and catalog hashes. Coverage and UTF-8 checks
do not substitute for human linguistic review or all-frame visual comparison.

Ending translations use filename/program-counter occurrence keys, retaining
each original 32-byte chunk and its timing. Staff roles and instructions are
translated; proper names, logos, version/date and an ambiguous handwritten final
phrase retain their original pixels. The loading image retains its artwork
above the functional translated caption band.

Changing language regenerates only the active text textures at the same slots
and dimensions. Previous textures are released. Five GPU copies are not retained.
The native screenshot paths apply the same replacement queue as the browser.
New browser profiles start in French; existing options migrate without losing
lives, bombs, music, color, display, difficulty or texture-blending preferences.

## Hitbox

Off, Focus and Always are persisted in Options. The marker is visible only for
an alive player outside the death transition. Its center follows the player’s
shaken RHW position; its 2.5-pixel core marks the original collision rectangle.
It adds drawing commands without changing player motion or collision tests.

## Browser menu and text readability

The browser title menu omits Quit from both drawing and keyboard navigation.
Up/Down wrap over the remaining entries; Q, Escape and Bomb cannot select an
invisible Quit entry at the title. Returning to the title from gameplay remains
available. The native application retains its original title exit behavior.

Text rasterization now composites foreground coverage over the shadow correctly
in straight RGBA. Zero coverage preserves existing pixels and opaque strokes
replace opaque shadow pixels. Glyph metrics, font sizes, shadow offsets and
sprite alignment are retained. Regression checks cover partial alpha and actual
Latin glyph strokes.

## Hosting adapter

The page may load an independently hosted interface from
`/platform/community.js`. This repository contains only language, suspension
and completed-run metadata hooks. Shared site source and services are maintained
in a separate project. Their absence does not block the game.

Only a live, terminal run emits its category, score and counters. Replays,
attract demos and diagnostic/god-mode captures are excluded. Clear results are
emitted before the ending; later result navigation cannot publish the same run
twice. Practice and Extra stay separate from normal mode. Publication, public
statistics and persistence are responsibilities of the independent host.

## Validation scope

The dated extension receipt records the exact source and build hashes, unit
checks, bounded option/hitbox captures and ending control/music-event comparisons
between languages. These are new extension checks, not a new claim of complete
original-executable or pixel/audio equivalence. Historical fidelity results
remain attached to the builds recorded in their own receipts.

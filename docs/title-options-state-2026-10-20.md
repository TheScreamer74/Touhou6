# Title animation state — ticket #13

The human report was reproduced on the pinned pre-fix native binary. After
Options→Back, the subtitle lost its diagonal and appeared as two horizontal
lines. Original title03.anm scripts1/2 set Z rotation to-3pi/8 at interrupt1;
interrupt2 restores position/visibility without setting that angle. Recreating
the runners and replacing pending interrupt1 with2 therefore erases rotation.

Original MainMenu.cpp enters Options with interrupt3 and returns with2 on the
same VMs. The port now transfers its live MenuArt between Title and Options;
Options keeps ticking those runners, and return no longer calls reset. Cursor,
input, pending navigation and timer still receive their existing return values.

Demo return has a different lifecycle: MainMenu is recreated. It still resets
its art, but its first calculation receives neutral input, as Supervisor does
when registering the new chain. The cancel/confirm key can no longer replace
interrupt1 before its startup rotation executes. It is not treated as an
Options return.

## Evidence

Four fresh silent native runs used the same original assets and isolated
storage: baseline/candidate Options and baseline/candidate demo cancellation.
Each produced six checkpoints (24PNGs total, kept private). Options return was
inspected after36,120and360ticks; subtitle diagonal is preserved. The initial
Options frame150 and idle-demo frame700 are byte-identical before/after, and
the full Options navigation CSV is byte-identical. Demo traces confirm an
actual stage at frame900 and title after the cancel input at1000. The startup
animation resumes with diagonal text; its intermediate kanji animation is
retained instead of skipping straight to the active menu.

135 release tests pass,18 are ignored by default. The original title03 script
test was explicitly run and passed for scripts1/2 with both native and web Quit
policies. It checks Options return rotation at36/120/360ticks and a recreated
demo title chain. The default synthetic test catches a reset in either Title
or Options. Existing navigation/selection/hidden-web-Quit tests pass.
Release build, WASM library check, formatting and diff whitespace pass.

The first capture-driver attempt used the wrong period/count argument order
for `tap`; the native parser refused before any capture. Its log is preserved
privately. Corrected inputs were rerun into fresh directories.

The adjacent receipt records scripts, commands and image hashes. Proprietary
pixels/assets are excluded from Git/Board. This is native validation and real
ANM-state validation; no fresh original process, Firefox runtime, Web package,
main merge or live deployment is claimed.

Candidate SHA256:
`eda900feb00204f2b83242ee40d9b85945594d2bd8e2508e291b2596855f9444`.
Runtime source fingerprint:
`d7b222ef68e1dc66144311231d574ed957713b6c33b193ee3cf1db86d0c5ee70`.
It cumulatively includes the separate Perfect Freeze white-index correction
e2464f9. That correction has its own commit; it does not affect these title
scenarios. The same exact cumulative binary was used for its fresh9stage
comparisons, whose receipt is also included.

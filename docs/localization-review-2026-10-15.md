# Localization review — 2026-10-15

Exhaustive linguistic review of the four catalogs in `crates/game/assets/locales/`
(1535 entries each). Method: LLM-assisted review — one Claude Opus 5.5 reviewer per
language read every entry against its Japanese source key and the English base,
reading `end:` occurrence keys as running text per file in numeric offset order.
Each reviewer produced a correction list (key, old, new, reason); a coordinating
agent (Claude Code, board ticket Touhou 6 #5) spot-checked the lists, verified
split-line duplications against the other catalogs and applied them with an
old-value match. No human native-speaker review is claimed.

| Language | Entries read | Corrections |
|---|---|---|
| FR | 1535 | 20 |
| EN | 1535 | 118 (57 are ending speaker-label normalization) |
| DE | 1535 | 27 |
| NL | 1535 | 25 |

Invariants checked after applying: same 1535 keys and order in every catalog,
byte-exact JSON formatting round-trip, valid UTF-8, `{…}` placeholders unchanged,
no value newly containing Japanese, no key added/removed (END occurrence keys
untouched). `cargo test --workspace` passes (154 passed, 30 ignored).

Not yet done: `scripts/audit_locales.py` and the ending timing/state/music replay
checks need extracted ED.DAT/MD.DAT, which were not available on the review
machine. Since no key changed, script timing is unaffected by construction, but
the replay check is still to be run.

Each section below is the reviewer's own report, including open doubts that were
deliberately left unchanged for a human to decide.


---

## French catalog review — fr.json

Every one of the 1535 entries in `crates/game/assets/locales/fr.json` was read. Each was compared with its Japanese key and with the English value in `en.json`. Ending occurrence keys (`end:<file>:<offset>`) were read as running text per file, sorted numerically by offset. They were also cross-checked against their Japanese ending-line keys (full lines and fragments).

The corrections are in `fr-corrections.json`: 20 entries. Every `key`/`old` pair was checked with jq against fr.json and all of them match byte for byte. No repo file was modified.

## Category counts

These counts come from a heuristic jq classification by key shape. Fragment keys with no leading spaces, such as `るのか？」` or `」`, fall under "dialogue" even when they belong to an ending or a comment.

| Category | Reviewed | Changed |
|---|---|---|
| UI / menu / options (ASCII + dotted keys) | 259 | 8 |
| Ending chunks (`end:*`) | 248 | 4 |
| Ending lines (JP keys with leading full-width spaces, speaker prefixes, BAD ENDING) | 213 | 5 |
| Dialogue (stage msg, incl. boss names/titles and unpadded fragments) | 570 | 2 |
| Spell names | 80 | 1 |
| Music titles (bare, `♪`, `♪ `, `No.N`) | 69 | 0 |
| Music Room comments | 89 | 0 |
| Stage names | 7 | 0 |
| **Total** | **1535** | **20** |

## Global checks (all passed unless listed under corrections)

- **Typography:** the convention is a regular space before `? ! : ;`, after `«` and before `»`, and is applied consistently. There is no NBSP or narrow NBSP anywhere, and no French punctuation is missing its space. Speaker labels are always `Name : « … »`. Ellipses use `...`. The only `…` are in `credits.and_you` and `credits.staff`, mirroring EN; I left them.
- **Apostrophes:** straight `'` is used in about 350 entries. Curly `’` appeared only in 3 difficulty cards, which are corrected.
- **Music titles:** every `♪` and `♪ ` variant and every `No.N` title matches its bare title exactly.
- **Names and terms:** these are consistent throughout:
  - Reimu, Marisa, Sakuya, Remilia, Flandre, Patchouli, Cirno, Rumia and Hong Meiling.
  - 紅魔館 = « manoir écarlate ».
  - お嬢様 = « maîtresse » (lowercase everywhere).
  - スペルカード = « carte de sort ».
  - 弾幕 = « danmaku ».
  - 巫女 = « prêtresse ».
  - Spell-sign prefixes are consistent too: 幻X → « X illusoire », 上級 → « supérieur », 禁忌 → Tabou, 禁弾 → Tir interdit.
- **Placeholders and padding:** no `{x}` tokens exist. The right-aligned difficulty labels keep the 9-character width ("   Facile", "Difficile", "Lunatique"). The `options.help` and replay header column spacing is preserved.

## Notable issues fixed

1. **Mistranslation (end10, Marisa):** 「つーか、入ればうまくなるのか？」 asks whether the cake tastes better with the bamboo flowers in it. The French "Et ça les rend meilleures ?" made the flowers the object. It now reads "Et ça le rend meilleur ?". The fix is applied to both chunks and to all three ending-line keys.
2. **Stage terminology:** 「Stage」 is « Niveau » everywhere (HUD, replay, Music Room comments). The exceptions were the difficulty cards ("(6 stages)") and `credits.extra_stage` ("Stage Extra"), which are now aligned.
3. **Player address:** UI text addresses the player with « vous » ("N'essayez pas !", "et vous…", "Si vous pouvez essayer"). "Appuie sur Z" and the bad-ending advice "termine en Normal…" used « tu ». They are switched to « vous », covering both chunks and both ending-line keys.
4. **Dialogue:** 週休２日 lost "days off". It now reads "deux jours de repos par semaine".
5. **Dialogue:** the fragment 「気持ちいいわね」 began with a stray "!" ("! Quel plaisir."). It is aligned with its sibling line.
6. **Spell-name consistency:** ミスディレクション was translated « Détournement d'attention » in one spell and « Diversion » (« Diversion illusoire ») in the other. Both now use « Diversion ».

## Doubts left unchanged (for a human)

- **Name-card padding:** these entries drop the source's full-width or ASCII padding: `" 十六夜 咲夜  "` → "Sakuya Izayoi", `"チルノ　　　　　"`, `"ルーミア　　　　"`, `"紅 美鈴　　　　 "`, `"宵闇の妖怪　　　 "`, `"湖上の氷精　　　 "`, `"知識と日陰の少女　 "`, `"華人小娘…"`. The trailing-space ending prompts ("She died by the bullet  ", etc.) do the same. EN does not pad these either, so I assume layout is handled in code. Please confirm.
- **`ああ、そうかい`** → "Ah, quant à ça." This reads oddly as a standalone sentence (JP ≈ "Oh, is that so"), and the EN "I can, thus." is also odd. I couldn't verify the surrounding lines, so a human should check it in context. "Ah, je vois." is a possible fix.
- **Long dialogue lines that may overflow the text box** (EN is much shorter):
  - `だあね` → "...par l'édit de compassion envers les êtres vivants." (55 chars)
  - `そういって返り討ちに会った人は` → "Celles qui ont dit ça et échoué sont plus nombreuses..."
  - Music Room comment lines of about 70 characters:
    - `　させないような曲にしました。`
    - `　で。短いステージの…`
- **Music Room comment line breaks:** two lines start with punctuation because of the per-line split: `　ージしているからです。` begins with ": j'imaginais…", and `　あんまり重厚さを…` begins with "» Je n'aime pas…". I left them because rebalancing would mean moving text across keys.
- **Spell name:** 土符「レイジィトリリトン」. The katakana is "Lazy Trilithon" (Patchouli), but EN uses "Rage" and FR follows it with « Trilithe furieux ». This is the community-standard EN reading, so I left it.
- **Music Room numbering:** titles use "No.1"…"No.17", while the rest of the FR UI uses "N°". It could be changed to "N° 1" if the layout allows.
- **`end00` 384–408 / `あんな薄暗い夏じゃ、向日葵の油すら取れないわ`:** "les tournesols ne donnaient rien" uses the imperfect where a conditional ("n'auraient rien donné") would be more precise. It is acceptable, so I left it.
- **`" ...脳なさそうだな"`:** the FR drops the leading "..." that both JP and EN have. This is minor.
- **`end11` 1982–1994:** 「大丈夫です、ねずみは生きています」 is addressed to Marisa in FR ("Rassure-toi"). The JP polite form is ambiguous; the current reading works with Marisa's reply.

---

## English catalog review — crates/game/assets/locales/en.json

Coverage: **100% of the 1535 entries were read** (all non-ending keys dumped key => value and read in full; all 248 `end:` occurrence keys read as running text per file, sorted numerically by offset). fr.json (and de/nl for Music Room line-order checks) used as cross-reference only. No repo file was modified; no python/cargo/scripts were run.

Output: `en-corrections.json` — **118 corrections** (all `key`/`old` values were generated from en.json with jq, so they match byte-for-byte; every `old != new`).

| Category | Entries reviewed (approx., by key shape) | Changed |
|---|---|---|
| Dialogue (stage msg) incl. boss titles/names | ~590 | 29 (+1 stage/scene title) |
| Spell names | 68 | 0 |
| Stage names | ~8 | 0 (1 scene title, see below) |
| Music titles (bare, `♪ `, `♪`, `No.N`) | 17 bare + 52 variants | 8 |
| Music Room comments | ~85 | 2 |
| Ending lines/fragments + `end:` chunks | 231 + 248 | 77 |
| UI / menu / options / credits / English source keys | 258 | 1 |

## Dialogue (stage msg)
Changed 29 + 1 scene title.
- **Line-split duplications** (the 2005 patch put the whole sentence on line 1 while the continuation line repeats its tail, so the player reads it twice): `人間って飲み物の形でしか見たこと`, `こんなに月も紅いから`, `あなたもこの館に雇われたの`, `あなたが、コンティニュー`, `良い子はおとなしく帰って`, `私はあなたみたいに鳥目`, `それともあなたは人間じゃ`, `お嬢様は滅多に人に会うような`, `とにかく、ここから出ていって`, `私は好きじゃないわ`, `さっきの女の子がおかしくなっ`, `あいつは絶対に、調理は出来ない`, `護衛にあのメイドを`, `さて、早速仕事に取りかかって`, `って、私たちいつから知り合いに`, `一個じゃ、`, `私の予定では最後の一人は`. The first line was trimmed so it carries only its own half; the continuation was left as is.
- **Mistranslations**: `恋愛係？` / `恋愛係は中等部なのか？` ("love doll" → romance duty), `って、そうじゃなくて、` (it was translated as the next line's question), `・・・神社にもう一人悪い子` ("stay here" vs "...at the shrine"), `要注意人物なんだね`, `消極的にやっつけるには・・・` ("in front of you" → "me"), and `夜の境内裏はロマンティックね` (temple → shrine).
- Rhyme glosses `（一人が弾幕を避けきれず` / `（一人が首を吊って` were both "and then there were none.", which left the `)` of `そして誰もいなくなった）` with no opening parenthesis. They are now the parenthetical glosses.
- Typos: `dont` → `don't`; `I that came` → `that I came`.
- `紅 美鈴` "Hong Meirin" → "Hong Meiling", to match the Music Room.
- `能ある鷹は尻尾隠さず...` (scene title): restored ZUN's "does not hide its tail" twist.

Open doubts left unchanged:
- `ああ、そうかい` = "I can, thus." This is odd English, but without the msg line order I cannot place it safely.
- `暗くて` ("It is dark.") + `どこに行っていいかわからないわ` ("It's so dark I don't know where to go."): possible duplication, but adjacency is not confirmed.
- `止められた` / `お外は豪雨で歩けない`: possible overlap.
- `そーなのかー` = "Oh, can I now?"
- `１３枚` = "13 times." (slices of bread).
- `だから～、私は鳥目じゃない` contains a stray "- -".
- Boss name/title pairs are split across the two lines on purpose ("The Youkai of / Darkness Rumia", "Maid of the Scarlet Devil / Mansion Sakuya Izayoi", "The Eternally Young Scarlet / Moon Remilia Scarlet", "Ice Fairy of / the Lake Cirno", "The Girl of Knowledge and / Sunshade Patchouli Knowledge"). This only reads correctly if the title is always drawn directly above the name. fr does not split them. Verify in-game.
- The source padding in the English rhyme keys (`     and then there were none.`, `She died by the bullet  `) is replaced by a "..." prefix or trimmed. The port does this deliberately in all catalogs, so it was left. `   and then there were none...` has no leading "..." because the source text ends with "...".
- `そこの紅白！` = "The one in red-white over there!" while `　そこの紅白！` = "You, red-white!". These are separate source keys; both are fine.

## Spell names (68)
No changes. The style is consistent: `X Sign "..."`, multi-element `A & B Sign`, `(Advanced)` for 上級, and Tepes, Lævateinn and Q.E.D. are spelled the same everywhere.
Doubts: `土符「レイジィトリリトン」` = "Rage Trilithon". The katakana reads "Lazy", but this matches the common TouhouWiki rendering, so it was kept. `「亜阿相界」` = "Acanthus" was not verified.

## Stage names
No changes. `暗闇の館　～ Save the mind.` → "Mansion of Darkness ~ Save the Mind": ZUN's English subtitle was re-capitalized and lost its period. This is cosmetic and was left.

## Music titles
Changed 8: `赤より紅い夢` and `紅より儚い永遠`, each in 4 forms (bare, `♪ `, `♪`, `No.N`). "Scarlet Beyond a Crimson Dream" and "Crimson Beyond a Fleeting Eternity" garbled the より comparison. They are now "A Dream More Scarlet than Red" and "An Eternity More Transient than Scarlet". All other titles are consistent across their 4 forms.
Doubt: the key `No.10  メイド…` has two spaces in the source, while the value uses one. Left unchanged.

## Music Room comments
Changed 2: "favourite" → "favorite" and "metre" → "meter", because the rest of the catalog uses American spelling.
I reconstructed the line order from the forced Japanese word splits (イメ/ージ, こうなっ/てしまいました, フレー/ズ, etc.) and checked it against the de/fr reflow. Under that order, every English line runs on correctly (e.g. Cirno "Yet I" → "cannot decide…" → "…perhaps. Well, it is stage two"). No reflow changes were needed.

## Endings
Changed 77: 20 `end:` chunks + 57 line/fragment keys.
- Speaker labels: one file mixed `Reimu "`, `Reimu  "`, `Reimu: "`, `Remilia: "` and `???    "`. All are now `Name "` with one space and no colon, which is the style most chunks already use. Applied to chunks and to the matching line/fragment keys.
- end01:633 `一人だぜ`: the source speaker is ？？, and the name has not been revealed yet. The label "Remilia" was changed to "???" (matches fr).
- end01:245 now uses the same wording as end00 for the identical Japanese line.
- end10: "a rare stuff" → "rare stuff". "Candy's rare leaves" → "a rare Kandy variety" (Kandy tea). "…own the place." → "?". "hot isn't it" → "hot, isn't it".
- end00: "20 years worth" → "20 years' worth".
- end11: "Yes Mistress" → "Yes, Mistress".
- All chunk edits stay within their own chunk and keep roughly the same length.

Doubts:
- "Sakuya "Arrrg, … errrr...."" (four dots) and "Marisa "...That was hard to say."" (とか言って見るテスト) are 2005 style and were left.
- "the dead rat is still alive" is an intentional joke and was left.

## UI / menu / options
Changed 1: `credits.extra_challenge`, from "If it was able to try,\nI would like you to challenge." to "If you are able to,\nplease take on the challenge.". If the broken English is a deliberate homage to ZUN's original staff-roll English, revert it.
Everything else is fine:
- No `{x}` placeholders exist.
- Padded difficulty keys keep their padding.
- `Loading...` → "Loading" and `Spell Card Bonus!` → "Spell Card Bonus" drop punctuation; this matches the dotted keys and was left.
- Curly apostrophes in `difficulty.extra_card` are a port-UI choice and were left.

---

## German catalog review (de.json)

Coverage: 100% of de.json was read, 1535 of 1535 entries, each next to its EN value and Japanese key. Ending occurrence keys (248) were read as running text per file, sorted numerically by offset (end00, end00b, end01, end10, end10b, end11). No repo file was modified. The corrections are in `de-corrections.json`: 27 entries, and every `old` value was checked byte-exactly against de.json.

The bucket counts below are approximate, from a key-shape heuristic.

Global checks:
- Quotation style: „…“ is used throughout. There are no straight double quotes, and the only apostrophe is the correct genitive in „Vlad Țepeș' Fluch“.
- No Japanese text was left untranslated.
- No `{x}` placeholders exist in this catalog.
- Leading and trailing spaces in keys are untouched, because only values are corrected.
- Register: Sakuya uses „Ihr/Euch/Herrin“ toward Remilia, and everyone else uses du. This is consistent.

## UI / menu / options (~276: 124 dotted keys + ~152 ASCII source keys)
- Changed: 6.
  - „Stages“ → „Stufen“ on the 4 difficulty cards.
  - credits.extra_stage „Extra-Stage“ → „Extra-Stufe“. Stage is „Stufe“ everywhere else (HUD, results, Music Room).
  - pause.confirm „Zum Titelmenü?“ → „Zum Titelbildschirm?“, to match pause.quit.
- Notable: the HUD labels („Bomben“, „Leben“, „Punkte“, „Streifer“, „Rekord“) and the duplicated ASCII/dotted keys are consistent with each other.
- Doubts, not changed:
  - credits.and_you „und dir…“ (dative) follows „und alle Testspieler“ (nominative). It is acceptable in a credits list.
  - credits.extra_challenge reads a little stiffly but is correct.

## Dialogue (~600 entries, mostly lexically sorted fragments)
- Changed: 1. „...“ restored on „...Du scheinst kein Gehirn zu haben.“
- Notable:
  - Fragments chain correctly into each other (checked across EN pairs).
  - Wordplay is handled well: Akanthus/„Ach, Akanthus“, the Falke/Schwanz malapropism, and the Thorium-Zerfallsreihe line.
  - The „keines mehr“ nursery-rhyme motif matches the official German Christie title.
- Doubts, not changed:
  - メイド長 is translated both as „leitendes Dienstmädchen“ and as „Chefin“. This is acceptable as shorthand.
  - 大量殺人犯 → „Serienmörderin“ (more literally „Massenmörderin“).
  - „Heute geht es meinem Asthma gut“ is literal, but so is the joke in the Japanese.

## Spell cards (~62)
- Changed: 8.
  - 5 dual-element cards were missing „-zeichen“. Example: „Erd- und Metallzeichen“. Also fixed the spelling „Lavacromlech“ → „Lavakromlech“.
  - 3 上級 cards used „…, höher“, which is not idiomatic. Changed to „…, fortgeschritten“.
- Doubts, not changed:
  - „Fünf Elemente „Stein der Weisen““ is a deliberate shorthand for 火水木金土符.
  - EN-based „Wütender Trilith“: レイジィ could literally be read as „lazy“, but the established reading is Rage.
  - Ț/ș in „Țepeș“: confirm the bitmap font has these glyphs.

## Stage names, song titles, boss titles (~52 ♪/No. + bare titles)
- Changed: 0. Bare, „♪ “ and „♪“ variants and the No.-prefixed titles are identical across the board.
- Doubt: Music Room titles keep „No.1“, while the UI uses „Nr.“. This is probably intentional, matching the original style.

## Music Room comments (~110 line chunks)
- Changed: 1. „Thema der Enden.“ → „Thema des Endes.“
- Notable: every comment was reconstructed as running text. Line breaks carry sentences correctly, for example „…kaum anders als sonst. Die Melodie / ist für mich leicht verständlich…“.

## Endings (248 occurrence keys + matching full-line and fragment keys)
- Changed: 11 across 5 issues.
  - 居ないぜ: „Nein, ich bin es nicht“ → „Bin gar nicht da.“ (2 keys).
  - とか言って見るテスト: the EN mistranslation „Das war schwer zu sagen“ → „...Hab ich nur mal so gesagt.“ (2 keys).
  - Mood clash „wenn du … einatmest, würdest du …“ → „wirst du unsterblich.“ (2 keys).
  - Restored the sarcastic お嬢様 („komme sofort, Herrin.“), which explains Sakuya's „Euch“ toward Marisa (3 keys).
  - Removed the comma before „und vertrieb“ in end10/end11:171 (2 keys).
- All ending chunks keep their length and stay in their own scene.
- Doubts, not changed:
  - end10/11 render the same opening sentence differently from end00 („Die Sonne war endlich aus ihrer Pause zurück“ vs „Nach einer langen Pause war die Sonne endlich zurück“). Both are fine.
  - „Imbiss“ for おやつ is acceptable, though „Naschwerk“/„Snack“ would be less ambiguous.

---

## Dutch (nl.json) localization review: Touhou 6 port

Scope: all 1535 entries in `crates/game/assets/locales/nl.json` were read side by side with `en.json` and the Japanese key. That is 100% coverage. Ending chunks were read as running text per file, in numeric offset order. Music Room comment lines were rebuilt into their per-track sequences (by JP line order) to check how sentences continue from one line to the next. No repo file was modified.

Corrections: 25, in `nl-corrections.json` (ui 9, ending 6, music_comment 6, dialogue 3, spell_name 1).

Category sizes below are approximate. Keys are not typed, so they were classified by key shape.

## UI / menu / options (153 English-source UI strings + 124 dotted keys)
- Changed: 9
  - The "Continues used" stat read `Doorgangen gebruikt` ("passages used"). Now `Keren doorgegaan`, in both result-stat keys.
  - `Stage` terminology. UI, HUD and the Music Room all use **niveau**, but `credits.extra_stage` said `Extra-stage` and the four difficulty cards said `(n stages)`. All now use `niveau(s)`. In Dutch, "stage" also means internship.
  - `Special thanks` → `Bijzondere dank`, to match `credits.special_thanks`.
  - `She went and hanged herself` → `Ze ging zich ophangen...`. The old text calqued the English "went and".
- Checked and fine: the right-aligned padded difficulty labels (all 9 characters wide), the column headers (spacing kept), placeholders (there are none in this catalog) and the je-register in the UI.
- Open doubts:
  - `options.defaults` / `Restore defaults` = `Standaardinstellingen` is a noun, not an action. `Standaard herstellen` might read better if space allows.
  - The Bad Ending headings are split as `SLECHT` + `EINDE NR. n`, but the JP split is "BAD ENDING No." + "n". Whether this renders well depends on how the two chunks are laid out.

## Endings (248 occurrence chunks, 6 files) plus full-line and fragment ending keys
- Changed: 6
  - end00:211 and end01:211 (and the full-line key 神社は快晴続きだった。) started with a capital `En` straight after a comma. Now lowercase, matching end10/end11:203.
  - end11:671 (and its fragment and full-line variants) had a comma between subject and verb: `Zonder toestemming binnenkomen en lezen, is verboden`. Comma removed.
- Checked and fine: chunk lengths stay similar. Quote style is „…“ throughout, with speaker labels as `Naam: „…“`. Sakuya calls Remilia `u`/`meesteres` and Marisa `jij`. In end10, ??? is Remilia (おやつまだぁ？). end01:633 correctly uses `???` where EN wrongly says Remilia.
- Open doubts: end00 1717/1741 `Alsof er bij de schrijn nooit iets zou veranderen.` is a verbless fragment. It is acceptable as a literary style, so it was left as is.

## Music Room comments (~95 lines) and track titles (17 bare, 17 `No.n`, 17+17 `♪` variants)
- Changed: 6. Several NL lines copied the EN patch's line breaks, which do not match the JP lines, so in-game each track's comment had dangling or stray words:
  - U.N. Owen comment: `Ik / weet ik niet` had a duplicate subject. Removed.
  - U.N. Owen comment ended on a dangling comma. It now ends with `misschien.`, which was taken out of Cirno's comment.
  - Cirno's comment: `Toch / begrijp de melodie` had no subject. Now `begrijp ik`.
  - Cirno's comment: removed the leading stray `misschien.` from the まぁ、２面だし line.
  - Stage 5 comment ended mid-sentence (`…rock nog steeds`). `verkeerd.` is now on that line, and Sakuya's last line no longer starts with the stray `verkeerd.`.
- Checked and fine: titles are consistent across bare, `No.n`, `♪ ` and `♪` variants. Proper names are kept (Voile, Tepes, Shanghai Alice, Meiji 17).
- Open doubts: `Maanklok ~ Maanschijf` for "Luna Dial" (a "dial" is a wijzerplaat) is acceptable, so it was kept. The 中華風に仕上げる… → Fusionというより… chain (`dus plaatste / ik hem hier`) assumes those two lines are consecutive in the same track, which matches the original comment.

## Spell names (~64 cards) and stage/character titles
- Changed: 1. `Doodsteken` (冥符) also reads as the verb "doodsteken" (to stab to death). Now `Onderwereldteken`.
- Checked and fine: the `X-teken „…“` pattern is consistent. Combined element signs use `Aarde en metaal`, `Hout en vuur` and so on, without "teken", and do so consistently. `上級` is rendered `, hoger` consistently. Stage titles and `Scarlet-landhuis` are consistent.
- Open doubts:
  - `, hoger` for 上級 could be `, gevorderd`. It is consistent as it stands, so it was not changed.
  - `Illusiegeest „Jack the Ludo Bile“` is left in English as a pun name.
  - `Taboe „Vier gelijken“`: in poker Dutch this is a "carré".

## Dialogue (stage scripts: full lines plus split fragments)
- Changed: 3
  - 「亜阿相界」 had a stray period after the closing quote.
  - " ...脳なさそうだな" lost the source's leading ellipsis.
  - せめて１ボムでも潰させないと～ said the player must waste a bomb. It is the speaker's obligation: `moet / ik je minstens één bom laten verspillen!`.
- Checked and fine: fragment pairs join grammatically. Puns are kept (`Ach, acanthus.` / „Acanthus“, the Washington Convention, decimal-system jokes). Character voices are distinct (Marisa casual, Sakuya polite to Remilia, Flandre childlike). There are no untranslated leftovers apart from proper names.
- Open doubts:
  - `妹君に言いたいけど` → `Jou als zusje wilde ik zeggen...` reads awkwardly, but the context was unclear.
  - `お嬢さん？` → `...jongedame.` drops the question mark. It may be intended as a continuation.
  - `逃すぜ` → `Ik laat haar gaan.` is interpretive, while EN has "She got away".
  - Some EN lines are themselves wrong (for example `まだまだだわ`, `って、そうじゃなくて、`). In those cases NL follows the JP and is correct.

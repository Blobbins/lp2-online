# Lost Planet 2 multiplayer: systems map

Starting points for the hybrid rewrite: replace these systems function by function from
`crates/xlive` (which already runs inside the game), then lift each finished system into a
standalone engine later.

Both exes load at the fixed base `0x400000` (no ASLR, no relocations). DX9 and DX11 builds
share every gameplay class; only renderer (`nDraw::*`) classes differ.

## Engine facts (MT Framework)

| What | DX9 | DX11 | Notes |
|---|---|---|---|
| `MtDTI::MtDTI(name, parent, size, flags, 0)` | `0x780a60` | `0x78f840` | Every class registers here. Output: `dx9/classes.json`, `dx11/classes.json` (3,538 / 3,545 classes) |
| Tuning registry `register(name, &global, 0, 0)` | `0x7bc2b0` | `0x7cb170` | Named 4-byte tuning globals. Output: `*/tuning.json` (2,560 unique names) |
| `MtPropertyList` insert (reflection) | `0x45aa60` | ? | Stack `MtProperty { name, u16 type, u16 attr, owner, field address... }`. Field offset = address − owner. Not yet mapped per class |

Defaults for most tuning globals are set by code at startup (they live in `.bss`), so
`lp2tuning_dump.txt`, written by the DLL while the game runs, is the source of truth for values.

## Multiplayer classes

| Class | Parent | Size | DTI (DX9) | DTI (DX11) |
|---|---|---|---|---|
| `sNetRuleMediator` | cSystem | 0x3480 | 0x18427b4 | 0x1852854 |
| `cNetRule` | cNetBase | 0xb70 | 0x18b85f0 | 0x18c8690 |
| `cNetGameManager` | MtObject | 0xb4c | 0x18b8590 | 0x18c8630 |
| `sGameNetwork` | cSystem | 0x28ed0 | 0x18420a8 | 0x1852148 |
| `sBattle` | cSystem | 0x4930 | 0x1841e78 | 0x1851f18 |
| `sGameManager` | cSystem | 0x52c40 | 0x1842084 | 0x1852124 |
| `cInfoGame` | MtObject | 0x8080 | 0x18b5dc4 | 0x18c5e64 |
| `sSaveLoadManager` | cSystem | 0xc05c | 0x1842c38 | 0x1852cd8 |
| `rSaveData` | cResource | 0x68 | 0x18b707c | 0x18c711c |
| `sItemManager` | cSystem | 0xbcd0 | 0x1842240 | 0x18522e0 |
| `cNetStorage` | cNetBase | 0x41c | 0x18b8610 | 0x18c86b0 |
| `cNetCtrl` | cNetBase | 0x1b8 | 0x18b8570 | 0x18c8610 |
| `uIdBattleResult` | uSprLayout | 0x6d0 | 0x18b4d70 | 0x18c4e10 |
| `uIdBattleRankResult` | uPlanetFlash | 0x2060 | 0x18b4d50 | 0x18c4df0 |
| `aBattle` | cArea | 0x3c | 0x1844af0 | 0x1854b90 |
| `aLobby` | cArea | 0x30 | 0x1845050 | 0x18550f0 |
| `aBattlePlayerMatch` | cArea | 0x3c | 0x1844bb0 | 0x1854c50 |
| `aBattleRankMatch` | cArea | 0x3c | 0x1844bf0 | 0x1854c90 |
| `aCustomize` | cArea | 0x3c | 0x1844d30 | 0x1854dd0 |
| `aRecordOnline` | cArea | 0x40 | 0x18451b0 | 0x1855250 |

Guesses from names (to confirm): `sNetRuleMediator`/`cNetRule` run mode rules (Elimination,
Data Post, Akrid Egg, Fugitive...); `cInfoGame` holds the player record (level, abilities,
unlocks); `sSaveLoadManager`/`rSaveData` persist it; `a*` classes are menu/flow areas.

## Live objects (both exes, same field offsets)

Every `cSystem` singleton keeps its instance pointer in the 4 bytes just before its MtDTI.

| Object | Instance global DX9 | DX11 | Notes |
|---|---|---|---|
| `sNetRuleMediator` | 0x18427b0 | 0x1852850 | |
| `sGameNetwork` | 0x18420a4 | 0x1852144 | |
| `sInfoManager` | 0x1842198 | 0x1852238 | `cInfoGame` is embedded at +0x3e0 |

Reflected fields: `dx9/fields.json`, `dx11/fields.json` (`tools/props.py`; ~17-18k fields, 1.3k
classes). MtProperty types: 3 bool, 4 u8, 5 u16, 6 u32, 7 u64, 8 s8, 9 s16, 10 s32, 11 s64, 12 f32.
The 100 multiplayer fields used by `crates/xlive/src/rules.rs` are in
`crates/xlive/data/fields_mp.txt`. Key ones:

| Field | Offset | Type |
|---|---|---|
| `InfoGame.mNetWork.gameTimerMax` | 0x358 | f32 |
| `NetRule.missionTimer` | 0x10c8 | f32 |
| `NetRule.isGameFinished` | 0x1150 | bool |
| `NetRule.mActiveMemberNum` | 0x1c00 | s32 |
| `GameNetwork.mIsHost` | 0x28 | bool |
| `GameNetwork.mSessionMaxJoin` | 0x3c | s32 |

Online save: `netSaveData00.dat` in per-user XStorage (likely progression). Rankings: XUserReadStats
views 1-3, columns 1-9 (`NetworkRankingLive`).

## Save file

`Documents\CAPCOM\LOST PLANET 2\<gamertag>\Lostplanet2.Lostplanet2Save-capcom`, 0x5020 bytes, written
by sSaveLoadManager (DX11 0x867f80) through XLiveProtectData (which this project's DLL passes through
unchanged, like the original dummy):

| Offset | Size | Contents |
|---|---|---|
| 0x00 | 20 | SHA-1 of the decoded body, standard IV (stored XOR 0xbeeff00d at 0x1850d38), each 32-bit word little-endian |
| 0x14 | 4 | magic 0x77cec928 |
| 0x18 | 4 | save counter |
| 0x1c | 4 | 0x1000 |
| 0x20 | 0x5000 | body, every byte XOR 0x73 (one byte derived from the key "shiorasikunai", DX11 0x7b3cd0) |

The body is a sequence of variable-length sections (dispatch table at DX11 0x868104), so field
offsets vary between saves. The first sGameManager section (DX11 0x7df920) starts with the owned
customization bitfields copied from the game manager (instance pointer DX9 0x1842020, DX11 0x18520c0):
heads 0x50710, bodies 0x50730, legs 0x50750, backpacks 0x50770 (32 bytes each), titles 0x507ec
(128 bytes). `crates/xlive/src/unlocks.rs` sets them.

| Unlock | Game manager offset | Layout | Real items | Source of the count |
|---|---|---|---|---|
| Heads | 0x50710 | 256 bits, MSB first per u32 | 134 | part table (DX11 lookup 0xf5c9a0) |
| Bodies | 0x50730 | same | 123 (table has 173; set 0x2d has no files on PC) | part table + CustomParts files |
| Backpacks | 0x50770 | same | 123 | part table |
| Legs | 0x50750 | same | 123 | part table |
| Special parts | bits 180..249 of each part field | same | 70 ids (`...0e`) | shared table DX11 0x16418b4; not unlocked |
| Titles | 0x507ec | 2 bits per title, ids 0x30.. | 476 (0x30..0x20b) | title table DX11 0x1744b90 |
| Weapons + grenades | 0x509c8 | 120 bits | 120 (17 special: ids at DX11 0x17450b0) | achievement check DX11 0x7f9080 |
| Weapon slot variants (what Customization → Weapons offers; "Weapons %" is out of these 60) | 0x5088c | 5 gun-slot bytes × 7 variants, then 5 grenade-slot bytes × 5 variants, bit n = variant n, bit 0 = default | 60 | DX11 0x7de8f0 / 0x7de920, Weapons tab handler 0xeee250 (uIdCustomize state 4) |
| Abilities | 0x501fc | 42 bits | 42 | achievement check DX11 0x7f5750 |

Part table id 0xAABBCCDD maps to `nativePCrcXmlTmpPC\CustomParts\<hd|bd|bk|lg><set><item>[<variant>].arc`
with set = 0x20 + DD (0x0f stays 0f), item = BB, variant = CC.

## Match flow (versus, DX11 addresses)

Host presses Start in the player-match lobby (uIdBattlePlayerLobby, vtable 0x162bcb0, update
0xe9b810) → countdown → session commands on the network thread → aGame. The session manager
(NetworkSessionManagerLive, vtable 0x16d37c0) runs commands posted with 0x44b4d0 (handler object and
method stored in the request); the log's `CM[o]: <Command> (n)` lines are those commands.

| Step | Where | Player-count gate (patched by `[startup] solo_start`) |
|---|---|---|
| Start button | lobby input 0xe96c72 | members > 1 (> 2 if sNetRuleMediator+0x87) |
| Countdown start | lobby 0xe9af6b | `sGameNetwork+0xa208` > 1, else msg 0x66 "Failed to initiate game session" + leave |
| Countdown end | lobby 0xe97690 | rule-mediator member records > 1 |
| Lock, match list | `CloseSession` → `MatchSession` → XSessionStart | none |
| In match | end check 0x1008be0 (reason codes for sGameNetwork 0x8596b0 "disconnect") | reason 7 "not enough players": count < 2, team entry, per-mode switch |

sGameNetwork (vtable 0x1379f30): vt[30] 0x85df30 leaves the session; 0x8596b0 sets the
disconnect flag (+0x113430) and reason (+0x11342c), which aGame's check (0x8a10b0) turns into a jump
to aError. All of these were found with `[trace]` hooks (`crates/xlive/src/hook.rs`).

## Versus bots (DX11 addresses)

Training -> AI Enemy Battle is offline versus with bots; online needed three things it lacks
(`crates/xlive/src/bots.rs`, `botmaps.rs`, `[bots] count = N`):

1. **Member records.** Bots are sNetRuleMediator member records with `isBot` (+0x69), `isGameMember`,
   `isStartMember`, a `botTableNo` (+0x46) and no `isValid`. The host fills free slots in the lobby
   from `data/bot_records.bin` (copied from an AI Enemy Battle). The match then creates a uCtrlPlayer
   slot, a `cThinkPlHm6x` think (virtual pad 0x800N) and a `uHm6x` character for each.
2. **Bot classes.** `autoArc\uHm60..6e` (model, `tool\AI\Think\HumanBotBt` = rThinkPlanAI) are queued by
   the stage loader 0x7d4420 only for Training stages; without them the character's resource check
   (vt[41]) fails on activation and it deletes itself. Queued with the area archive via 0x7d2630.
3. **Navigation.** Online maps ship no AI navigation; Training stages `aArea_TUTOREAL_NN` are online
   maps plus `tool\AI\NaviMesh\..\List_..` (rAINavigationMeshList), `Way_..` (rAIWayPoint) and
   `tool\AI\AreaAI\..`, bound through their script list (Tool\ScrList, XFS: entries mType 0x1d/0x1e/0x1f).
   Without them the AI runs but never moves or finds targets. Pairs: ONLINE_12 (Cube) <- 26,
   ONLINE_10 <- 31, ONLINE_01 <- 41, ONLINE_09 <- 46 (36 is Cube's second layout).

Archive type hashes are `~crc32(class name) & 0x7fffffff`. Found with `[trace]` hooks and `[watch]`
hardware watchpoints (`crates/xlive/src/watch.rs`).

## Next RE steps

1. Playtest a hosted match with `[rules]` empty: read `lp2rules_dump.txt` and the `match:` log
   lines to learn units (is `gameTimerMax` seconds or frames?) and which fields drive the match.
2. Game modes: map `gameRule` (per-member, no static offset yet) and where `X_CONTEXT_GAME_MODE`
   picks the mode; the modes' own state lives in `sNetRuleMediator` (data posts, Akrid egg, fox).
3. Progression: capture `netSaveData00.dat` after a match and decode it into a Rust struct.
4. Per-member arrays in `sNetRuleMediator` (nickName, careerLevel, classRank...) are registered
   relative to a member record, so `props.py` reports no offset; resolve the record stride.

## Tools

`tools/*.py` (Python 3 + `capstone`, `pefile`):

- `xref.py EXE name...`: code that references a string.
- `dti.py EXE CTOR OUT`: class table.
- `tuning.py EXE REGISTER OUT` and `tuning_defaults.py EXE OUT`: tuning names, addresses and the defaults recoverable statically.

`lp2-arc` (Rust, `crates/arc`) reads, writes, extracts, repacks and greps the game's `.arc` archives
(all 2,951 shipped archives read and round-trip unchanged).

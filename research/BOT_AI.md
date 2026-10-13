# Versus bot AI: data and knobs

What decides how a versus bot (classes uHm60..6A, think `cThinkPlHm6x`, see SYSTEMS.md "Versus bots")
moves, picks targets and shoots, where each setting lives, and which edits should make bots better.
Addresses are DX11 (`LP2DX11.exe`, image base 0x400000); DX9 equivalents are not mapped yet.

Four things drive a bot:

| What | Where | Format | Same for every bot? |
|---|---|---|---|
| Plan graph (what to do next) | `tool\AI\Think\HumanBotBt` in `arcLp2Xml\autoArc\uHm60..6e.arc` | XFS, `rThinkPlanAI` | yes, the 15 files are byte-identical (31,106 bytes, md5 865bcdd7...) |
| Position scoring (cover, shot, approach, escape points) | `Tool\AI\Pef_Hm40` in every `aArea_TUTOREAL_NN.arc` and in the online stage archives | XFS, `rAIPositionEvaluationFuncList` | yes, all 75+ copies identical (3,824 bytes) |
| Personality: 21 percentages per bot (aim speed, aggression, cover...) | exe `.rdata` 0x14d7880 (8 presets x 24 bytes), copied into the character at +0x2428 | raw bytes | one of 8 presets, chosen per slot |
| Fire rhythm: when a bot holds the trigger | exe `.data` 0x17b4f58 (10 rows x 9 floats) | f32 frame counts | yes |

Nothing else under `tool\AI` ships in the bot class archives. The stage AI files (`NaviMesh\List_*` "LNV",
`Way_*` "WAY", `AreaAI\*` "IBL") are not XFS; the navmesh sessions own them.

`research/tools/xfs.py` reads and writes both XFS files (`dump`, `pack`, `plans`, `test`);
`research/BOT_AI_plans.txt` is the whole plan graph as `xfs.py plans` prints it.

## XFS layout (version 8)

```
0x00 "XFS\0"  u16 version = 8  u16 unknown = 1  u32 class count  u32 definition block size
0x10 u32 offset[count]  (offsets in this block count from 0x10)
     class: u32 type hash = ~crc32(class name) & 0x7fffffff, u32 field count,
            field (24 bytes): u32 name offset, u8 type, u8 attr, u16 size, 16 zero bytes
     field names, NUL-terminated, each once in order of first use; zero padding to 4 bytes
0x10+size  root object
object: u16 (class index << 1 | 1), u16 object id (sequential), u32 size counting itself,
        then for each field of the class: u32 count, count values
values: class/classref (1, 2) = nested object; bool/u8 (3, 4) 1 byte; u32 (6), s32 (0xa), f32 (0xc) 4;
        u64 (7) 8; string (0xe) NUL-terminated; rect (0x12) 4 x s32
```

`attr` (0x80, 0x91, 0xa0, 0xa1 on a few fields) is kept as is; its meaning is unknown and the loader
does not need it to read the data. Every one of the 90 AI files in the uHm6x and Training archives
survives dump -> JSON -> pack byte for byte (`python -I research/tools/xfs.py test`).

## HumanBotBt (`rThinkPlanAI`)

Root: `mSaveInfo.itemAIInfoArray` (79 plans), `mSaveInfo.defaultItemIndex` = 0, `mQuality` = 2.

A plan (`rThinkPlanAI::cItemPlanInfo`):

| Field | Meaning (inferred from the data and the plan code) |
|---|---|
| `mName`, `mIndex` | name, its index in the array |
| `mAttr` | 0x200 = root plan, 0x1 = leaf (runs, then the root is re-chosen), 0 = step that hands over to `mItemNextIndexArray` |
| `mRootAttr` | bit mask of the root tree(s) the plan belongs to: 0x1 Fight, 0x2 NoTarget, 0x4 FSM, 0x8 TargetDie, 0x10 rappel, 0x20 TEST, 0x40 in a VS, 0x80 shield, 0x100 wait area, 0x200 jump, 0x400 avoid area, 0x800 carrying the egg |
| `mItemNextIndexArray` / `mItemPreIndexArray` | children (candidates) / parents, by plan index |
| `mFilterFuncList` | all must pass (`mAndCheck`; `mInverse` negates) for the plan to be a candidate |
| `mEvaluationFuncList` | score terms; the best-scoring candidate wins |
| `mMoveFuncList` | what the bot does while the plan runs; a move that returns (`mReturn`) ends the plan |
| `mTiredPoint`, `mFrustrationPoint`, `mTiredRecoverPoint` | repetition penalty: tiredness gained per run, gained on failure, recovered over time (unknown exact formula) |
| `mItemKind`, `mRect` | always 0 / editor box position |

Units: times are frames at 30 per second (`cAIPlanMoveTime` with `mIsParamCheck` converts a percentage
to `p * 0.01 * 30`); distances are game units (100 = about 1 m, unverified).

Function classes used (fields as named by the game):

- Filters: `TargetExist`, `Distans(TgtType, Radius, IsInner)`, `CharIsXXX`, `CharStatus(CheckChar, CheckStatus u64 mask)`, `TracePos(TgtType)` (a point of that type exists), `MessagePickUp(MessageType)`, `FSM`, `Reload` (magazine low), `IsAction`, `True`, `Weapon(WpMode, WpAttr, WpAttr2 u64 masks)`, `Area(CheckArea)`, `GetAkEgg`, `DefenceAkEgg`.
- Evaluations: `Default(Weight)`, `Distans(TgtType, Radius, IsInner, Weight)`, `DamageRemain(IsMinusRemainCheck, Weight)`, `DataPostOn`, `MessagePickUp`, `AIParam(CheckParamMode, Weight)` (personality byte, below), `DangerArea`, `GetWp`, `Reload`.
- Moves: `Trace(TgtType, Radius, ArriveRadius, CountTimer, ..., MoveTimeObj)` (path to a point), `Shot(NoShotAttr)` (aim and fire, see fire rhythm), `Time(Return, Time, IsParamCheck)`, `Damage(DamageLimit)` (end the plan after taking that much damage), `Distance(TgtType, Radius, IsOut, Return)`, `HmAction(PadAction)`, `HarmoControl`, `PickMessage`, `ShotGranade`, `JumpControl`, `ReturnX`.

`TgtType` (from the plans that use them; unverified): 0 enemy target, 1 friend/leader, 5 goal, 8 FSM position,
10 cover point, 11 shot/patrol point, 13 approach point, 16 data post, 17 weapon on the ground,
21 wall point, 22 Vital Suit, 23 wait-area point, 24 avoid point, 26 Akrid egg, 27 egg goal.
`PadAction`: 1 action/melee/board, 2 reload, 3 crouch, 4 dodge roll, 9 and 26 taunt, 11 activate post,
12 pick up, 14 switch weapon, 23/24 anchor jump/cancel (unverified).

Root plans and their base weights: Fight 20 (target exists), NoTarget 10, TargetDie 30, FSM 50, AutoRappel 70,
SeeOrSpaceJump 72, WAIT_AREA 45, AVOID_AREA 46, FightShield 25, EggToGoal 23, YATTA 100 (in a VS),
TEST 10000 (its filter can never pass). Fight's 20 children are the combat menu:

| Plan (index) | Does | Score terms (weights) | Ends after |
|---|---|---|---|
| Wait (7) | stands and shoots | Default 500, AIParam mode 2 x200 | 90 frames, 30 damage, or grenade message |
| RunToShotP (8) -> WaitShotP (9) | runs to a Pef "ShotPoint", shoots | Default 800, DamageRemain -300, grenade/danger message 300, AIParam 3 x100 | **40 frames** |
| RunToTarget (28) -> ShotTaregt (27) | closes to 800, shoots | Default 700, near (<1000) -400, far (>1500) +300, AIParam 3 x100 | 90 frames or target < 500 |
| RunToCoverP (12) -> WaitCoverP (17) | runs to a "CoverPoint", crouches | DamageRemain 800, Default 300, DangerArea 1000, near 400, near friend 600, AIParam 4 x100 | 30 frames |
| RunToWallP (6) -> WaitAI-P (5) | wall/AI point, crouches | DamageRemain(minus) 800, DangerArea 1000, Default 300, near target -600, friend 500 | 150 frames |
| RunToTarget (3) -> WaitNear (11) -> Strike (4) | melee | within 700: 700, message 1000, AIParam 3 x1000 | 30 frames |
| Avoid (23) | dodge roll | DamageRemain 1000, Default 500, AIParam 1 x1000 | 20 frames |
| Avoid-G (10) | dodge a grenade | needs grenade message; Default 1000 | 20 x param |
| ThrowG (29) | grenade | target 500..3000 away, height check 1300, AIParam 10 x1000 | 40 frames |
| RunToDataPost (1) -> ExecDataPost (2) | capture a data post | Default 300, DataPostOn 1000, DamageRemain -40; needs post < 1500 and enemy > 500 away | 300 frames, 10 damage, or enemy < 400 |
| RunToWp (24) -> GetWp (25) | pick up a weapon (< 1400) | Default 500, GetWp 1000 | |
| WpChange (26) | switch weapon | Default 30, DamageRemain -200 | |
| RunToReload (36) -> Reload (37) | reload behind cover | Reload 500, AIParam 3 x100 | |
| RunToYatta (48) -> YattaRideOn (47) | board a Vital Suit | Default 1500 when a VS point exists | |
| RunToApproach (49) -> WaitAppPoint (50) | flank via "ApproachPoint" | Default 500, far (>1000) 500 | 15 frames |
| Ft-RunToReader (51), RunToFriend (75) -> FriendFollow (76) | follow a friend | FriendFollow: AIParam 9 x1000, Default 500 | 90 frames |
| Ft-RunToGoal (16), RunEgg (67), RunEggArround (78) | mode objectives | | |

## Pef_Hm40 (`rAIPositionEvaluationFuncList`)

Loaded by 0x835900 into the stage's "PICK" AI service. One `cAIPositionEvaluationFuncList` "Main" with
six scorers; each has a doughnut filter around the bot (`mRangeNearXZ`..`mRangeFarXZ`, `mRangeY`) and
weighted terms (`mWeight` s32, `mRatio`/`mGoodDist` f32):

| Point | Filter near..far, height | Terms |
|---|---|---|
| CoverPoint | 150..1000, 400 (territory) | DangerRange 200, DangerMessageRange 100, DistToPos -1 x0.1, TargetVisibilityHigh 30, TargetVisibilityLow -200, TargetRange 1, TargetGoodDistans 1 @1500 |
| ShotPoint | 300..1000, 200 (territory) | DangerRange 200, DangerMessageRange 100, DistToPos -1 x0.1, TargetGoodDistans 1 @1500, TargetVisibility 50 |
| LookPoint | 200..800, 500 | PlayerVisibility 50 for players 0..3, DistToPos 1 |
| ApproachPoint | 300..1000, 200 (territory) | DistToPos -1, TargetRange -1 x1.2, TargetVisibilityLow 100, TargetGoodDistans 1 @1000 |
| EscapePos | 600..1000, 800 | DangerRange 200, DangerMessageRange 200, DistToPos -1 |
| FreePoint | 1500..2500, 100 | DistToPos 70 x26 |

## Personality (`cAIParam`, character +0x2428)

The uHm constructor 0xc57c20 sets all bytes to 50. When the character is AI-driven (`+0xcd0` = 1), its
spawn code 0xc76ee0 copies a 21-byte preset through the virtual at vtable+0x808 (0xc20130 ->
`cAIParam` vt[5] 0x835020): `+0x20f0` set -> 0x14d7940; else by `sGameManager.mEpisodeNo` (+0x257):
1..6 (campaign episodes) -> 4-row tables 0x14d7958..0x14d7b80; anything else -> the 8-row table
0x14d7880, row = (slot object 0x1852080[slot] +0xa) & 7. Which row a versus bot gets is not verified
(log `[0x1852080 + 4*slot] + 0xa`). (`cAIParam` vt[6] 0x835050 is a second setter that also caps byte 0
by difficulty: 80/90/100/70; the spawn path does not use it.)

Byte i is at character +0x242c+i. Meaning from the code that reads it:

| Byte | Read by | Effect |
|---|---|---|
| 0 | aim update 0xfbf460 | **aim tracking speed**: the aim point closes `speed x byte%` of the gap to the target per update (capped at 1; 0.5 in one campaign case) |
| 1 | AIParam mode 1; `MoveTime` with IsParamCheck | dodge (Avoid, Avoid-G) score; the length of param-timed waits (byte% x 30 frames) |
| 2 | mode 2 | Wait score |
| 3 | mode 3 (+50 for some weapon classes, +100 if the target holds weapon 0x64) | attack: Strike, WaitShotP, ShotTaregt, Reload |
| 4 | mode 4 | cover: WaitAI-P, WaitCoverP |
| 5, 6, 8 | modes 5, 6, 8 (unused by HumanBotBt); byte 6 also 0xfb5840 (x5) | unknown |
| 7 | mode 7 | objective: Ft-RunToGoal, WaitGoal |
| 9, 18, 19 | nothing found | unknown |
| 10 | mode 9 | follow friends: FriendFollow |
| 11 | mode 10 (+5/+10 in some modes) | grenades: ThrowG |
| 12 | MoveShot 0x9947ba | > 80 picks fire-rhythm row 0 for some weapon classes |
| 13, 15, 17 | modes 14, 13, 12 (unused by HumanBotBt) | unknown |
| 14 | mode 11 | taunt after a kill (Appeal1) |
| 16 | MoveShot 0x994b11 | **fire hesitation**: for weapons where 0x900990 returns 2 the rhythm clock runs at `max(0.1, 1 - byte%)` speed, so 50 doubles every pause (and the 2.7 s first-shot delay becomes 5.3 s) |
| 20 | not read | preset type id |

Current presets (table 0x14d7880, the one versus bots should use; bytes 0..19):

```
row 0: 50 100  50  50  50  50  50 100  90  50  50  90  50  50 100  50  50  50  50  50   type 0
row 1: 10 100  10  10  10  10  10  80  90  50  50  80  50  50 100  50  50  50  50  50   type 1
row 2: 80 100 100 100 100 100 100 100 100 100  50  60  50  50  50 100  50  50  50  50   type 2
row 3: 70 100  10 100  70  70  70  80  80  10  50  50  50  50 100  50  50  50  50  50   type 3
row 4: 50 100  50  50  50  50  50 100  85  50  50  90  50  50 100  50  50  50  50  50   type 0
row 5: 10 100  10  10  10  10  10  50  80  50  50  80  50  50 100  50  50  50  50  50   type 1
row 6:100 100 100 100 100 100 100 100 100 100  50  75  80  50 100  90  70  10  50  50   type 2
row 7: 65 100  15 100  45  55  55  65  75  15  55  95  55  55 100  55  55  55  55  55   type 3
```

## Fire rhythm (`cAIPlanMoveShot`, 0x994430)

MoveShot holds fire unless: the weapon/target checks pass, a target-info value (think +0x1a8 -> +0x194,
probably aim alignment) is at least 0.6, and the rhythm says "fire". The rhythm is a row of 9 frame
counts at 0x17b4f58 + 36 x row, read as alternating windows **fire, pause, fire, pause, ..., fire**
against `cThinkPlanAI::mPastTime` (+0x840, the time spent in the current plan, the same clock
`cAIPlanMoveTime` ends plans with); after the 9th window it holds fire.

Row choice: by weapon class (byte 2 of the weapon id at weapon +0x1c, 0x30..0x34 and 0x41 have their own
cases); every other weapon uses the distance: within 13000 units -> row 3, beyond -> row 2 (rows 1/2
when target-info +0x19c is set).

```
row 0: 30 20 15 10 30 15 30 20 90      row 5: 15 50 35 100 40 60 15 100 15
row 1: 15 30 10 20 20 25 10 20 50      row 6: 10 90 30 60 70 89 100 80 260
row 2:  0 50 15 40 30 30 20 40 15      row 7: 20 15 40 260 100 100 160 90 260
row 3:  0 80 15 60 30 100 20 80 15     row 8: 15 90 15 90 35 55 20 90 15
row 4: 40 90 15 90 35 55 20 90 15      row 9: 15 50 35 100 40 60 15 100 15
```

Row 3, the usual one, starts with a 0-frame fire window and an 80-frame pause: **a bot fires nothing in
the first 80 frames (2.7 s) of every plan**, and only 80 of 405 frames overall. Most firing plans are
shorter: WaitShotP ends at 40 frames (never fires), Wait/ShotTaregt/FriendFollow at 90 (fire for 10
frames). This, more than aim, is why bots look passive. The table is shared with the campaign's AI humans.

## Other places checked

- `aimOffHosei{Normal,Hard,Extreme,Single}` (tuning, .bss 0x185180c, 0x1851838, 0x1851788, 0x1851898):
  difficulty-indexed aim offset used by 0x903f10/0x90ed70, reached from the think message handler
  0x916290 (scripted shoot commands); not seen on the bot path.
- `cThinkPlHm6x` (vtable 0x13bb768, 0x260 bytes) adds no AI fields of its own; `cThinkPlanAI` (0x8b0 bytes)
  holds the running state (`mpItemNow`, `mPastTime`).
- `uIdTrainingTop.mSelectLevel` (+0x19c) is the Training menu's level choice; where it flows was not traced.

## Knobs

| Knob | Where | Now (uHm60..6A all alike) |
|---|---|---|
| Reaction time / fire duty | rhythm table 0x17b4f58 rows 2, 3 | 1.7 s / 2.7 s before the first shot in a plan; 33% / 20% duty |
| Fire hesitation | personality byte 16 | 50 in rows 0-5, 70 row 6, 55 row 7 (rhythm at 50% / 30% / 45% speed) |
| Aim accuracy (tracking) | personality byte 0 | 50, 10, 80, 70, 50, 10, 100, 65 |
| Fire gate on aim | MoveShot 0x99476b compares with the shared constant 0.6 at 0x16f7e4c | 0.6 (patch the instruction, not the constant) |
| Aggression | personality byte 3; HumanBotBt ShotTaregt (#385 Default 700), WaitShotP (#152 Default 800), Wait (#126 Default 500) | see table |
| Time spent shooting | `MoveTime` #157 (WaitShotP 40), #393 (ShotTaregt 90), #129 (Wait 90) | 40 / 90 / 90 frames |
| Target selection / range | ShotTaregt filters 850..3000, evals near -400 / far +300; Pef ShotPoint GoodDistans 1500 | as listed |
| Retreat / health | `EvaluateDamageRemain` weights (Avoid 1000, WaitCoverP 800, WaitShotP -300, ExecDataPost -40), `MoveDamage` limits (Wait 30, RunToDataPost 50, ExecDataPost 10) | as listed |
| Cover use | personality byte 4; WaitCoverP / WaitAI-P weights; Pef CoverPoint | 50 typical |
| Grenades | personality byte 11; ThrowG (#29) height check 1300, AIParam 10 x1000, range 500..3000 | |
| Weapon choice | WpChange (26) Default 30, DamageRemain -200; GetWp (25) | rarely chosen |
| Vital Suits | RunToYatta (48) -> YattaRideOn (47) Default 1500; in VS: YATTA root (Yt-Shot 100, Yt-Wait 80, Yt-RideOff 500 when target > 1000) | |
| Data posts | RunToDataPost (1) -> ExecDataPost (2): Default 300, DataPostOn 1000, post < 1500, enemy > 500 | |

## Proposal (ranked)

1. **Fire rhythm: shoot sooner and longer.** Write 72 bytes at DX11 0x17b4f58 + 36 x 2 (rows 2 and 3, f32):
   row 2 `0 50 15 40 30 30 20 40 15` -> `0 8 30 20 30 20 30 20 9999`,
   row 3 `0 80 15 60 30 100 20 80 15` -> `0 8 45 20 45 20 45 20 9999`.
   First shot 0.27 s into a plan instead of 1.7/2.7 s, then 1-1.5 s bursts with 0.67 s pauses and steady
   fire after 5.3/6.8 s; WaitShotP starts firing at all. Apply from xlive (`write_mem`) only while bots are on,
   since campaign AI shares the table.
2. **Personality: aim and no hesitation.** In the 8 presets at DX11 0x14d7880 (24 bytes each, `.rdata`),
   set byte 0 (aim tracking) to 90, byte 3 (attack) to 100 and byte 16 (fire hesitation) to 0 in every row:
   offsets 0x14d7880 + 24 x row + {0, 3, 16}. Rows 0..7 cover whichever row a bot gets. Bots that
   track at 10-50% now trail any strafing player.
3. **HumanBotBt: stand around less, shoot from shot points longer.** With `xfs.py dump` / `pack` on
   `tool\AI\Think\HumanBotBt` (then `lp2-arc pack` into each `uHm60..6A.arc`, installed with a `.orig`
   backup like `botmaps`): `cAIPlanEvaluateDefault` #126 (Wait) `mWeight` 500 -> 150;
   `cAIPlanMoveTime` #157 (WaitShotP) `mTime` 40 -> 120; `cAIPlanMoveTime` #393 (ShotTaregt) `mTime` 90 -> 180.

None of this was tried in game. Before editing, confirm the personality row with a log of
`[[0x1852080 + 4*slot]] + 0xa` and the rhythm row by watching `esi` at 0x994b4e.

# Teams in versus modes (DX11 addresses, static analysis)

Short version: a member's team is the u32 at **member record +0x00** (`teamKind`), not a slot index.
The host's game already balances bots onto teams and sends the result to every PC. Our DLL then
overwrites it: `bots.rs` rewrites every bot record that differs from the template on every frame
until the match starts, and the template has `teamKind = slot`. So each bot ends up as its own team.

## 1. Where the team is stored

Member record = 0x7c bytes at `sNetRuleMediator(0x1852850) + 0x148 + i*0x7c`. The game registers
the record's fields as MtProperties (0x8247e1..0x82562d), which gives their names:

| Off | Type | Name | Notes |
|---|---|---|---|
| +0x00 | s32 | `teamKind` | **The team.** -1 = none, 0/1 = the two teams in team rules, slot index in free-for-all, 0xf = judge |
| +0x04 | s32 | (no name) | second team field. The balancer sets it to -1 along with +0x00; the team mask receiver sets it to the team for `isValid` members still at -1. Probably the requested/initial team |
| +0x08 | s32 | `lobbyProcess` | record version: a newer record replaces an older one only when this is >= the old value (0xff89c0). 0x820dd0 sets it to 3 at countdown end |
| +0x2c | str | `nickName` | |
| +0x3c | u8 | `id` | the real slot index |
| +0x3d / +0x3e | u8 | `countryFlagNo` / `influenceNo` | influence = the faction used by faction matches |
| +0x44 / +0x45 | u8 | `careerLevel` / `classRank` | the balancer sorts on classRank |
| +0x46 | u8 | `botTableNo` | |
| +0x64..+0x6d | bool | `isValid isGameMember isStartMember isInvited isLeader isBot isJudge isStickerComplete isTeamDecide isVoteDecide` | `isTeamDecide` (+0x6c) = this record got its team from the host's team mask |

- Read accessor: `getTeamKind(slot)` = **0x821020** (thiscall on sNetRuleMediator, u8 slot, returns
  `[rule+0x148+slot*0x7c]`). 85 callers.
- Team names: `teamRanking[TEAM_RED]` is at rule+0x1a44, `[TEAM_BLUE]` at +0x1a45 (then GREEN,
  YELLOW), so the enum is **0 = red, 1 = blue**. Team-based spawning (0xff9930) only accepts teamKind
  <= 1. The red/blue labels come from enum order only; the game screen can confirm them.
- Per-team counters indexed by teamKind: `mActiveMemberNum[16]` at rule+0x1c00 (0x821270 skips values
  > 0xf), `mTeamOccupiedDataPostNum` at rule+0x1e7c (0x821040 skips -1).
- **Which rules are team rules**: 0xff9010(rule) = `rule == 0 || (4 <= rule <= 11)`, where rule =
  `sNetRuleMediator+0x28` (the first dword of the 0x64-byte match settings block at rule+0x28..+0x8b,
  written by 0x872bf0). Rules 1..3 and > 11 are free-for-all. The rule ids still need names (see
  §6). Rules with extra team logic:
  - **rule 0xa**: teamKind 0 is a special side (0x7debb0 / 0x93b180 return "member is on team 0";
    the balancer gives team 0 to a chosen few `isValid` players and team 1 to everyone else). This fits
    Fugitive.
  - **rule 7**: `isLeader` (+0x68) members get a per-team flag at rule+0x1ebe+team (0x8f9d15), and the
    balancer has a leader pass (0xffae18).
  - **faction matches** (game manager +0x8474 == 2): teams come from `influenceNo`, giving
    `teamKind = (influence == my influence)`, i.e. 1 = my faction (0xffa607).
- `rule+0x87` (settings +0x5f) is a separate team-mode flag, checked in the lobby (more than 2
  members needed to start) and in the match-end check.
- **Not the team:** `sGameNetwork(0x1852144)+0x28d98+i*12` is a per-member network state table
  (dword 1 = in the session; the countdown 0xe97264 lists the members whose state is 1 and byte +8 is
  0). The end check at 0x1008ca1 tests *my own* entry (`[net+0xa204]`) == 1 in team mode. It is not a
  team number.
- **In the match**, the team becomes the character's power group: when the slot's character is
  created (0x8fb1bc) the game sets `mPowerGroup = getTeamKind(slot)` in team rules and `= slot`
  otherwise, writing both char+0xce2 (`mPowerGroup`) and char+0xce3 (`mInitPowerGroup`). The character
  is `[ctrl+0x40]`, where ctrl is the slot's uCtrlPlayer and ctrl+0xb0 is the slot. **This happens once,
  at spawn**, so the record has to be right before the characters are built.

## 2. Who assigns teams, and when

Lobby countdown, `uIdBattlePlayerLobby` state function 0xe97130. When the countdown finishes
(0xe972d5..):

1. `0x828110(-1)`: the host sends the settings block (command packet, type 0).
2. **If the rule is a team rule: `0xffa040` = the team balancer**, then `0x821c20(2)` = command 2,
   the "team mask". (Rule 7 also sends command 0xa.)
   **Otherwise (free-for-all): every record gets teamKind = slot** (0xe97386 loop). This is where
   Training's `teamKind = slot` comes from.
3. Later, at countdown end 0xe97610 → `0x820dd0`: isGameMember = isValid || isBot,
   isStartMember = 1, lobbyProcess = 3 for all 16 records. **This is the moment our DLL stops touching
   records**, which is after step 2.

**The balancer 0xffa040.** Online it returns immediately unless this PC is the host (0x802d10); offline
(game manager +0x25f) it always runs.
- Takes every record with `isValid || isBot`, except `isJudge` ones. **Bots are included.** Sort key:
  players use classRank (+0x45); bots use classRank plus a random value.
- If `sGameNetwork+0x1136db == 2`, first resets teamKind and +0x04 of all 16 records to -1.
- Default path: sorts by key and hands out teams in the pattern **1,0,0,1, 1,0,0,1, ...**, a snake
  draft, so the teams come out balanced in count and in rank.
- Rule 0xa: a separate path picks the special side. It only considers `isValid == 1` records, so
  **bots get no team there** (they keep -1 on the host).
- Writes go through 0xff89c0 (setRecord, keeping the newer `lobbyProcess`).

**How the result reaches other PCs: command 2.** This is a type-2 command packet (event 0x42000002,
packer 0x1011df0 case 2 at 0x1011ea3, receiver 0x1012710 case 2 at 0x10128ef). It carries a 16-bit
mask in which bit i = `teamKind(i) & 1` for every slot with isValid or isBot, so **bot slots are
included**. The receiver, for each of the 16 slots:
- `isJudge`: teamKind = +0x04 = 0xf in team rules (otherwise `id`).
- everyone else: **teamKind = bit i (0/1)**, isTeamDecide = 1, and +0x04 = the same value if the slot
  is isValid and +0x04 was -1.

So a joining PC gets the host's bot teams without the bot records being sent. (Records are sent one
at a time with packet type 1 = 0x81f860 → 0x10117c0, receiver 0x1011a70. The game only sends your own
slot (0x828150). The sender would also accept isBot records, but nothing calls it for them.)

**Lobby team choice.** When settings +0x08 (`rule+0x30`) is non-zero, the lobby runs an extra state
after the countdown (0xe9b032 → 0xe96230(1)). That is the team-select phase where "Join Blue/Red Team"
is used. A player's choice travels in their own type-1 record (the packet's teamKind byte, unpacked at
0x1011c1f). When +0x30 == 1 the in-match rejoin check at 0xff9fc7 skips that member. I did not trace
the select screen's UI. With +0x30 == 0 (auto balance) the balancer decides everything.

## 3. Friend-or-foe test

There is no single function. Every check compares the two characters' power group bytes inline,
`a+0xce2 != b+0xce2` means enemy. Examples: 0x909c10, 0x9b29a3, 0x901991, 0x912df5, 0x9130ac (I did not trace which is
AI targeting and which is damage). About 214 instructions touch +0xce2. Since power group = teamKind
at spawn:
- free-for-all: power group = slot, so everyone is an enemy.
- team rule with teamKind 0/1: teammates share a group.
- **teamKind -1: power group 0xff, an enemy of everyone**, and no team spawn point.
- teamKind 5..15 (what our bots get now): every bot is its own group, i.e. free-for-all. This matches
  the symptom.
Power group 1 has extra meaning for category-3 (player) characters in a few places (0x909bc0,
0x912cd6). That is the game's own campaign logic, and it applies to blue team players too.

## 4. Training AI Enemy Battle

The bot record builder **0x828ca0(slot)** (called by Training setup 0x8a0ef6 / 0x8a0fdc, and
0x873deb, 0xebc785, 0x1012c17) sets teamKind:
- game manager +0x256 == 0 (not versus, i.e. campaign AI partners): **1**, so everyone is on one side.
- team rule: `rule+0x30 == 1` ? **slot & 1** : **0**.
- free-for-all: **slot**.
The player's own record (0x820b87) gets 1 when not versus, otherwise `(rule+0x30 == 1)`, i.e. 1 or 0
(1 when +0x30 == 1). So Training does handle team rules: with +0x30 == 1 the player (team 1) has the
odd-slot bots as teammates against the even-slot ones. Our `bot_records.bin` came from a free-for-all
AI Enemy Battle, which is why its +0x00 equals the slot.

## 5. What the DLL should write

**The fix is mostly to stop undoing the game's work.** `bots::on_frame` currently rewrites a bot slot
whenever `rec(i) != want`. The balancer (host) and the command-2 receiver (every PC) change teamKind,
+0x04 and isTeamDecide during the countdown, before 0x820dd0 sets isGameMember, so the next frame
puts `teamKind = slot` back.

Proposed change (host and joiners alike, lobby only, as today):
1. When building `want` for bot slot i, **copy the live record's +0x00..+0x07 (teamKind and the
   second team field) and +0x6c (isTeamDecide) into `want`** before the compare and write. The game
   owns those bytes.
2. For a slot that has no bot record yet (first write), use `teamKind = +0x04 = (i & 1)` in team
   rules (`0xff9010(rule+0x28)`) and `= i` otherwise. This is only a lobby placeholder, the same thing
   Training does with +0x30 == 1. The host's balancer replaces it at countdown end, and command 2
   replaces it on every PC.
3. **Rule 0xa only:** the balancer leaves bots at -1 on the host, and the joiners take bit = 1 from
   the mask. On the host, after the countdown (any bot slot with teamKind == -1 while the rule is
   0xa), write `teamKind = +0x04 = 1` so the bots join the majority side, matching what joiners
   already have. This must happen before the match spawns characters (power groups are copied at
   spawn). The window is the gap between the team mask and 0x820dd0, or the DLL can check the rule
   explicitly instead of relying on the frame loop.

Nothing has to be sent: command 2 already carries the bots' bits to joiners. The bytes per bot slot i
are: rule+0x148+i*0x7c+0x00 (s32 team), +0x04 (s32, same value), +0x6c (u8 isTeamDecide, set by
the game).

## 6. What to check at runtime (session that drives the game)

Team match with bots, host and one joiner if possible:
1. **Lobby, before Start:** dump the 16 records (`[0x1852850]+0x148`, stride 0x7c). Note `+0x00`,
   `+0x04`, `+0x6c` per slot, and `[0x1852850]+0x28` (rule id) and `+0x30` (team-choice setting).
2. **Proof of the overwrite:** hardware write watchpoint on bot slot 5's +0x00
   (`[0x1852850]+0x148+5*0x7c`) across the countdown. Expected writers: 0xffa3ee (-1), 0xff89c0 (0/1,
   balancer), then our DLL's `write_mem` (back to 5). On a joiner: 0x101299d (command 2), then the
   DLL.
3. **In the match:** each slot's character `+0xce2` (mPowerGroup). Today we expect bots = 5..15,
   players = 0/1. After the fix, all 0/1. The quickest way to get the character: break at 0x8fb1f4,
   where `[esi+0x40]` = character and `[esi+0xb0]` = slot.
4. Rule id ↔ mode name: write down `[0x1852850]+0x28` for Elimination, Team Elimination, Data Post,
   Akrid Egg, Fugitive and the faction modes, to confirm that rule 0xa = Fugitive and to name rule 7.
   Team rules should be 0 and 4..11.

# LP2 Online — Lost Planet 2 multiplayer, revived in Rust

Lost Planet 2 (PC) did its online play through Games for Windows - LIVE, which is gone.
The game's own netcode is fine; what died is the `xlive.dll` service layer underneath it.
This project replaces that layer, so the original game plays online again against servers
anyone can run.

| Crate | What it is |
|---|---|
| `crates/xlive` | Drop-in 32-bit `xlive.dll`. Exports the exact 117 ordinals LP2 imports. Signs you in, gives the game working XNet sockets and session search. |
| `crates/master` | `lp2-master`, the server. Lists hosted lobbies (TCP) and relays game packets for players behind NAT (UDP), both on one port. It stores nothing on disk and runs no game logic: the hosting player's game runs the match. |
| `crates/proto` | Wire format both of them share. |
| `crates/arc` | `lp2-arc`: read, write, extract, repack and search the game's `.arc` archives. |
| `research/` | Reverse-engineering tools and tables: classes, tuning globals, and the multiplayer systems map (`research/SYSTEMS.md`). |

## Changing the game (hybrid rewrite)

The DLL runs inside the game process, so it can change LP2 itself as well as the online layer.
The plan is to replace multiplayer systems one at a time in Rust while the original game runs
everything else.

**Tuning (works now).** LP2 has about 2,500 named tuning values (abilities, Vital Suits, weapons,
data posts and more). About 10 seconds after startup the DLL writes them all, with their live
values, to `lp2tuning_dump.txt` in the game folder. Copy any lines into `lp2mods.ini` and edit them:

```ini
[tuning]
AbilityCritical_DamageUpRatio = 1.5
```

Values with a decimal point are floats; whole numbers are integers. Keep the form the dump uses.
Players only see lobbies whose host has the same `lp2mods.ini` overrides (mismatched tuning
would desync a match). Unknown names are reported in `lp2net.log`.

**Match rules (`[rules]`, first rewritten system).** The DLL reads LP2's live multiplayer state
through the game's own objects and reflection names. After a match starts it writes every field
to `lp2rules_dump.txt`. Fields you set under `[rules]` are applied only while you host:

```ini
[rules]
GameNetwork.mSessionMaxJoin = 16
InfoGame.mNetWork.gameTimerMax = 600.0
```

`lp2net.log` records each change in the match state (host, lobby size, players, time limit,
finished) and the game's own debug messages (`[game] ...`). What each field does in a real match
still needs confirming in playtests; start from the values in the dump.

**Unlocks (`[unlocks]`).** `parts`, `titles`, `weapons`, `abilities` = true unlocks them all.

**Solo start (`[startup] solo_start = true`).** The host can start a Player Match alone, and a match
ends on time or score only, never for lack of players.

**Bots (`[bots] count = 1..15`).** The host's free lobby slots fill with the game's own Training
("AI Enemy Battle") bots. They need the map's AI navigation, which only maps with a Training
version have, so bots move on four online maps (Cube and three others); elsewhere they stand
still. On first start the DLL copies that data into `nativePC\arcXmlTmpPC\aArea_ONLINE_{12,10,01,09}.arc`,
keeping each original as `.arc.orig` (rename them back to undo). Everyone in the lobby should use
the same `lp2mods.ini`.

`[startup] skip_logos` / `skip_title` skip the boot screens.

**Next:** see `research/SYSTEMS.md`.

## Build

Needs Rust with the MSVC toolchain and the 32-bit target:

```bash
rustup target add i686-pc-windows-msvc
```

```bash
cargo dll
```

```bash
cargo build -p lp2-master --release
```

Output: `target/i686-pc-windows-msvc/release/xlive.dll` and `target/release/lp2-master(.exe)`.
The master is plain Rust std, so it also builds on Linux for a VPS.

## Run a server

```bash
lp2-master 27900
```

Open port 27900 for **both TCP and UDP**. Relayed traffic uses the server's bandwidth.

## Install on each player's PC

1. In the game folder, rename the existing `xlive.dll` to `xlive.dll.bak` (that's the offline
   dummy; putting it back undoes everything).
2. Copy the built `xlive.dll` into the game folder.
3. Copy `lp2net.ini.example` there as `lp2net.ini`; set `name` and `master`.

Saves: profile data is kept in the game folder's `ProflieSetting\title_specific{1,2,3}.bin` in
the same format the dummy used (the typo is the dummy's), so switching DLLs back and forth is safe.

## How it works

- **Sockets.** The game's UDP/VDP sockets are virtual. All traffic goes over one real UDP socket,
  tagged with source/destination player and port. Each player gets a fake `10.x.y.z` address,
  which is what the game sees.
- **Addresses.** A player's XNADDR carries their id, LAN and public endpoints, and whether they are
  directly reachable. Packets go direct when the target is reachable or on the same LAN, otherwise
  through the master's relay. Replies follow whatever path a packet arrived on.
- **Sessions.** A hosted matchmaking session is published to the master with its contexts,
  properties, slot counts and QoS (lobby) data, refreshed every 5 s and dropped 30 s after the host
  goes quiet. A search lists the open sessions.
- **Master safety.** Only the IP that published a session can change or remove it. An instance id
  can't be taken over while its owner is alive. The relay forwards only from the endpoint that
  registered the sender. Sizes are capped.

## Status

Experimental. Tested on LP2 PC v1.0.0.129, DX11 (DX9 shares the code but is less tested):
hosting, searching, a 2-player match (two game instances on one PC), solo matches and bot
matches all work. **Not yet tested: a match between two PCs over the internet**, and whether
joining players see the host's bots. Read `lp2net.log` when something fails.

Known gaps (each is a `shortcut:` comment in the code):

- Search returns every open lobby; game mode/map filters are not applied.
- QoS reports a nominal 50 ms ping; real latency is not measured.
- No voice chat (the engine object accepts every call and never has audio).
- No System Link LAN broadcast discovery; LAN players use a master too (run one locally).
- The game's TCP sockets (if it uses any) need a direct route; they can't go through the relay.
- No leaderboards, achievements, title storage, DLC or invites.

## References

- [chasmlol/2010-rust-rewrite-mashup](https://github.com/chasmlol/2010-rust-rewrite-mashup): its
  `iw4l-master` (relay plus server browser, host runs the match) is the model for `lp2-master`.
- [XLiveLessNess](https://gitlab.com/GlitchyScripts/xlivelessness) (LGPL): used as a reference for
  GFWL ordinals, signatures and struct layouts. No code is copied.

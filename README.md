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
| `directory` | The server directory: a Cloudflare Worker where servers list themselves and games find them. |
| `dist` | Installer, uninstaller, `host-server.bat` and the release packaging script. |
| `crates/arc` | `lp2-arc`: read, write, extract, repack and search the game's `.arc` archives; export map collision as OBJ (`research/COLLISION.md`). |
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

**Unlocks (`[unlocks]`).** `parts` and `titles` = true unlock every cosmetic. Weapons and abilities
unlock as your online career level rises: with `weapons_by_level = 20`, a share of them each level and
all by level 20 (the original game only gave them out at random after campaign chapters).
`weapons` / `abilities` = true unlock them all at once instead. Unlocks are kept in each player's save.

**Solo start (`[startup] solo_start = true`).** The host can start a Player Match alone, and a match
ends on time or score only, never for lack of players.

**Bots (`[bots]`).** A Player Match lobby you host starts with just you; **F7 adds a bot and F8
removes one**, up to 11 (the lobby lists the bots and its help line shows the keys). Free lobby slots fill with the game's own Training
("AI Enemy Battle") bots. They need the map's AI navigation, which only maps with a Training
version have, so bots move on four online maps (Cube and three others); elsewhere they stand
still. On first start the DLL copies that data into `nativePC\arcXmlTmpPC\aArea_ONLINE_{12,10,01,09}.arc`,
keeping each original as `.arc.orig` (rename them back to undo); the lobby help line is
added to `nativePC\arcLp2Xml\ID_FCORE_*_PC.arc` the same way. Everyone in the lobby should use
the same `lp2mods.ini` tuning. Joining players pick up the host's bot count automatically (their
`lp2mods.ini` needs a `[bots]` section). Up to 11 bots.

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

## Play

You need your own copy of Lost Planet 2 (PC, v1.0.0.129: Steam, retail or other). Nothing from
the game is included here.

1. Download the zip from [Releases](../../releases) and unzip it anywhere (the game folder is easiest).
2. Double-click `install.bat`. It finds the game (next to it, in a Steam library, or asks you to
   pick the folder), backs up the old `xlive.dll` and asks for your player name.
3. In the game: Online > Player Match. The search lists every lobby on every LP2 Online server.

`uninstall.bat` puts the game back exactly as it was. Saves: profile data is kept in the game
folder's `ProflieSetting	itle_specific{1,2,3}.bin` in the same format the original dummy DLL
used (the typo is the dummy's), so switching back and forth is safe.

## Host a server

A server (`lp2-master`) lists lobbies and relays game traffic for players behind routers; the
hosting player's game still runs each match. Anyone can run one:

1. Forward port 27900 (**TCP and UDP**) on your router to the PC.
2. Run `host-server.bat` (or `lp2-master 27900 --name "My server"` on Windows or Linux).

Every minute it announces itself to the server directory, which first checks that the port answers
from the internet; then it's in everyone's search. `--private` keeps it out of the directory (for a
group that sets `master = host:port` in `lp2net.ini`). Relayed traffic uses the server's bandwidth.

**The directory** (`directory/`) is a small Cloudflare Worker that only keeps the list of servers
online (in memory, nothing stored). See `directory/README.md` to deploy your own.

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
matches all work, including a match with all traffic forced through the relay. LP2 uses UDP only,
so the relay carries everything. **Not yet tested: a match between two PCs over the internet.** Read `lp2net.log` when something fails.

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

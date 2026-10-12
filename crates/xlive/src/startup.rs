//! Boot flow. LP2's menus are MT Framework areas, switched by `areaManager->jump(areaDTI)`:
//!
//! ```text
//! aInit -> aPowerOn -> aLogo (logos) -> aTitle (press start, demo loop) -> ... -> aMainMenu
//! ```
//!
//! `[startup] skip_logos = true` in lp2mods.ini: aLogo is a state machine (state 0 setup check,
//! states 1-5 one logo each, state 6 finishes init and jumps to aTitle). Its jump-table entries
//! for states 1-5 are pointed at its own "next state" code, so every logo is skipped but aLogo's
//! setup (which also starts the network system) and state 6 still run.
//! (Jumping from aPowerOn straight to aTitle skipped that setup and left a black screen.)
//!
//! `skip_title`: see SKIP_TITLE.

use crate::mods::{Exe, EXE, MODS};
use crate::patch::{self, Patch};

/// Five little-endian u32s, for jump-table patches.
const fn table(v: [u32; 5]) -> [u8; 20] {
    let mut out = [0u8; 20];
    let mut i = 0;
    while i < 20 {
        out[i] = v[i / 4].to_le_bytes()[i % 4];
        i += 1;
    }
    out
}

const SKIP_LOGOS: [Patch; 2] = [
    Patch {
        name: "skip_logos (DX9)",
        exe: Exe::Dx9,
        addr: 0x0089_D9A4, // aLogo update jump table, entries 1-5
        original: &table([0x0089_D89D, 0x0089_D8A7, 0x0089_D8B1, 0x0089_D8BB, 0x0089_D8C5]),
        patched: &table([0x0089_D981; 5]),
    },
    Patch {
        name: "skip_logos (DX11)",
        exe: Exe::Dx11,
        addr: 0x008A_D154,
        original: &table([0x008A_D04D, 0x008A_D057, 0x008A_D061, 0x008A_D06B, 0x008A_D075]),
        patched: &table([0x008A_D131; 5]),
    },
];

/// `[startup] skip_title = true`: aTitle's states are 0 setup, 1 title/press-start logic,
/// 2 act on the menu choice, 3 idle back to logos. The choices are 0 start (-> aGuide, the
/// sign-in step that leads to the main menu), 1 benchmark, 2 PC settings, 3 exit. State 1's
/// jump-table entry is pointed at choice 0's code, so the game goes on as if Start was chosen.
/// That code skips the title's sound-fade call made before every choice, so title music may run
/// on briefly.
const SKIP_TITLE: [Patch; 2] = [
    Patch {
        name: "skip_title (DX9)",
        exe: Exe::Dx9,
        addr: 0x0089_FF70, // aTitle update jump table, entry 1
        original: &0x0089_FED8u32.to_le_bytes(),
        patched: &0x0089_FF45u32.to_le_bytes(),
    },
    Patch {
        name: "skip_title (DX11)",
        exe: Exe::Dx11,
        addr: 0x008A_F670,
        original: &0x008A_F5D8u32.to_le_bytes(),
        patched: &0x008A_F645u32.to_le_bytes(),
    },
];

/// `[startup] multi_instance = true`: LP2 refuses to start while another copy runs
/// (named mutex + GetLastError() == ERROR_ALREADY_EXISTS (0xB7) -> restore the other window, quit).
/// The compared constant becomes 0xFFFFFFFF, which GetLastError never returns.
const MULTI_INSTANCE: [Patch; 2] = [
    Patch {
        name: "multi_instance (DX9)",
        exe: Exe::Dx9,
        addr: 0x0045_D4B3,
        original: &0xB7u32.to_le_bytes(),
        patched: &u32::MAX.to_le_bytes(),
    },
    Patch {
        name: "multi_instance (DX11)",
        exe: Exe::Dx11,
        addr: 0x0045_D653,
        original: &0xB7u32.to_le_bytes(),
        patched: &u32::MAX.to_le_bytes(),
    },
];

/// `[startup] solo_start = true`: the versus lobby (uIdBattlePlayerLobby input, DX11 0xe96c72) only
/// lets the host start when `members > 1`, or `> 2` when sNetRuleMediator+0x87 is set (team modes):
/// `mov eax,1 / je +5 / mov eax,2 / cmp ecx,eax / jle no_start`. Both limits become 0.
/// When the countdown starts, the lobby (DX11 0xe9af6b) checks the session's member count again:
/// `cmp [sGameNetwork+0xa208],1 / jg ok`, else "Failed to initiate game session" and leave
/// (found with `[trace]` on sGameNetwork vt[30] 0x85df30). The limit becomes 0.
/// When it reaches 0 (DX11 0xe97690) it counts the rule mediator's member records and needs
/// `count > 1` (`cmp ecx,ebx / jg ok`); `jg` becomes `jge`.
/// In the match, the end check (DX11 0x1008be0, run continuously) returns reason 7 "not enough
/// players" when the player count is < 2 or no other member is present (found with `[trace]` on
/// the disconnect setter 0x8596b0). `cmp eax,2 / jl reason7` becomes `jmp` past both checks, so a
/// match also keeps running when everyone else leaves.
/// Team modes then also need this member's team entry (`[sGameNetwork+0x28d98+i*12] == 1`):
/// `cmp [rule+0x87],0 / je ok` becomes `jmp ok`.
/// Last, a per-mode switch (by rule mode 1..12: two alive players, both teams populated...) also
/// returns 7 alone; its `ja default` becomes `jmp default`, which returns 0 (keep playing).
/// With all of `solo_start`, a match ends on time/score only, never for lack of players.
const SOLO_START: [Patch; 14] = [
    Patch { name: "solo_start 7 (DX9)", exe: Exe::Dx9, addr: 0x00FF_894F, original: &[0x83, 0xF8, 0x0B, 0x77, 0x60], patched: &[0x83, 0xF8, 0x0B, 0xEB, 0x60] },
    Patch { name: "solo_start 7 (DX11)", exe: Exe::Dx11, addr: 0x0100_8D2F, original: &[0x83, 0xF8, 0x0B, 0x77, 0x60], patched: &[0x83, 0xF8, 0x0B, 0xEB, 0x60] },
    Patch { name: "solo_start 6 (DX9)", exe: Exe::Dx9, addr: 0x00FF_88C1, original: &[0x80, 0xBD, 0x87, 0, 0, 0, 0, 0x74, 0x1E], patched: &[0x80, 0xBD, 0x87, 0, 0, 0, 0, 0xEB, 0x1E] },
    Patch { name: "solo_start 6 (DX11)", exe: Exe::Dx11, addr: 0x0100_8CA1, original: &[0x80, 0xBD, 0x87, 0, 0, 0, 0, 0x74, 0x1E], patched: &[0x80, 0xBD, 0x87, 0, 0, 0, 0, 0xEB, 0x1E] },
    Patch { name: "solo_start 5 (DX9)", exe: Exe::Dx9, addr: 0x00FF_8871, original: &[0x83, 0xF8, 0x02, 0x7C, 0x67], patched: &[0x83, 0xF8, 0x02, 0xEB, 0x36] },
    Patch { name: "solo_start 5 (DX11)", exe: Exe::Dx11, addr: 0x0100_8C51, original: &[0x83, 0xF8, 0x02, 0x7C, 0x67], patched: &[0x83, 0xF8, 0x02, 0xEB, 0x36] },
    Patch { name: "solo_start 4 (DX9)", exe: Exe::Dx9, addr: 0x00E8_75FF, original: &[0x3B, 0xCB, 0x7F, 0x2E], patched: &[0x3B, 0xCB, 0x7D, 0x2E] },
    Patch { name: "solo_start 4 (DX11)", exe: Exe::Dx11, addr: 0x00E9_76CF, original: &[0x3B, 0xCB, 0x7F, 0x2E], patched: &[0x3B, 0xCB, 0x7D, 0x2E] },
    Patch { name: "solo_start 3 (DX9)", exe: Exe::Dx9, addr: 0x00E8_AEA0, original: &[0x83, 0xB8, 0x08, 0xA2, 0, 0, 1], patched: &[0x83, 0xB8, 0x08, 0xA2, 0, 0, 0] },
    Patch { name: "solo_start 3 (DX11)", exe: Exe::Dx11, addr: 0x00E9_AF70, original: &[0x83, 0xB8, 0x08, 0xA2, 0, 0, 1], patched: &[0x83, 0xB8, 0x08, 0xA2, 0, 0, 0] },
    Patch { name: "solo_start 1 (DX9)", exe: Exe::Dx9, addr: 0x00E8_6BBD, original: &[0xB8, 1, 0, 0, 0], patched: &[0xB8, 0, 0, 0, 0] },
    Patch { name: "solo_start 2 (DX9)", exe: Exe::Dx9, addr: 0x00E8_6BC4, original: &[0xB8, 2, 0, 0, 0], patched: &[0xB8, 0, 0, 0, 0] },
    Patch { name: "solo_start 1 (DX11)", exe: Exe::Dx11, addr: 0x00E9_6C8D, original: &[0xB8, 1, 0, 0, 0], patched: &[0xB8, 0, 0, 0, 0] },
    Patch { name: "solo_start 2 (DX11)", exe: Exe::Dx11, addr: 0x00E9_6C94, original: &[0xB8, 2, 0, 0, 0], patched: &[0xB8, 0, 0, 0, 0] },
];

/// At DLL load, before the game's WinMain.
pub fn early() {
    let Some(exe) = *EXE else { return };
    crate::log!("instance {} as '{}'", *crate::INSTANCE, crate::CONFIG.name);
    if enabled("multi_instance") {
        MULTI_INSTANCE.iter().filter(|p| p.exe == exe).for_each(|p| drop(patch::apply(p)));
    }
}

fn enabled(key: &str) -> bool {
    MODS.ini.get("startup").is_some_and(|s| s.iter().any(|(k, v)| k == key && (v == "true" || v == "1")))
}

/// Called once, before the game's area system runs (XLiveInitializeEx).
pub fn apply() {
    let Some(exe) = *EXE else { return };
    crate::hook::apply();
    crate::bots::install();
    crate::watch::apply();
    for (key, patches) in [("skip_logos", &SKIP_LOGOS[..]), ("skip_title", &SKIP_TITLE[..]), ("solo_start", &SOLO_START[..])] {
        if enabled(key) {
            patches.iter().filter(|p| p.exe == exe).for_each(|p| drop(patch::apply(p)));
        }
    }
}

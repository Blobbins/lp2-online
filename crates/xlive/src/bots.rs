//! Versus bots. LP2 already has them: Training -> AI Enemy Battle fills the free member slots of
//! sNetRuleMediator with bot records (`isBot`, a `botTableNo` naming the bot's look and AI, its name
//! and custom parts) and the match spawns an AI-driven character for every such slot.
//!
//! A `[bots]` section in lp2mods.ini turns bots on. A hosted lobby starts with none; F7 adds one and
//! F8 removes one (shown in the window title). With N bots, the top N member slots (16-N..16) get
//! those records (`data/bot_records.bin`, copied from an AI Enemy Battle); top slots because the
//! session hands joining players the lowest free member index. The count is published with the
//! session and joining players follow it.
//!
//! A bot's class comes from its slot (0x7e3320: slot s -> uHm5f+s), and only uHm60..6A have a
//! network puppet, so `SLOT_CLASSES` makes slots 5..15 uHm60..6A instead: bot k (Training's bot k,
//! class uHm60+k-1) sits in slot k+4. (Training with 5+ bots then repeats looks; harmless.)
//!
//! Every PC in the lobby writes the same records (the host's count, see `follow`). The host's
//! game runs the bots' AI and sends their state like a player's; on the other PCs a bot slot gets a
//! network puppet (`cThinkNetPuppetHM6x`, the think chooser 0x8daef0 picks it for a bot it doesn't
//! host), and `isBot` keeps the start and "everyone loaded" checks from waiting for it. Puppets
//! exist only for uHm60..6A, so at most 11 bots.
//!
//! Member record (0x7c bytes at sNetRuleMediator+0x148+i*0x7c): +0x00 index, +0x2c name,
//! +0x3c index, +0x46 botTableNo, +0x64 isValid, +0x65 isGameMember, +0x66 isStartMember,
//! +0x69 isBot.
//!
//! Online also lacks what Training loads for them: the bot classes (queued below with the stage)
//! and the maps' AI navigation data (`botmaps`), so bots work on the 4 maps that have Training
//! versions (`botmaps::PAIRS`; Cube is one).

use crate::mods::{read_mem, write_mem, Exe, EXE, MODS};
use crate::patch::Patch;
use std::sync::atomic::{AtomicUsize, Ordering};

const RECORD: usize = 0x7C;
const RECORDS: usize = 0x148;
const IS_VALID: usize = 0x64;
const IS_GAME_MEMBER: usize = 0x65;
const IS_BOT: usize = 0x69;
/// Bot records 0..16 as AI Enemy Battle sets them up (record 0, the player, is unused).
static TEMPLATE: &[u8; RECORD * 16] = include_bytes!("../data/bot_records.bin");

struct Globals {
    rule: usize,
    network: usize,
    game: usize,
}

fn globals(exe: Exe) -> Globals {
    match exe {
        Exe::Dx9 => Globals { rule: 0x0184_27B0, network: 0x0184_20A4, game: 0x0184_2020 },
        Exe::Dx11 => Globals { rule: 0x0185_2850, network: 0x0185_2144, game: 0x0185_20C0 },
    }
}

fn ptr(global: usize) -> Option<usize> {
    let mut b = [0u8; 4];
    read_mem(global, &mut b).then_some(())?;
    Some(u32::from_le_bytes(b) as usize).filter(|&p| p != 0)
}

fn byte(addr: usize) -> Option<u8> {
    let mut b = [0u8; 1];
    read_mem(addr, &mut b).then_some(b[0])
}

fn enabled() -> bool {
    MODS.ini.contains_key("bots")
}

/// The bot count in play: set by the hotkeys while hosting, or the joined host's.
static COUNT: AtomicUsize = AtomicUsize::new(0);

pub fn count() -> usize {
    COUNT.load(Ordering::Relaxed)
}

/// A new hosted lobby starts with no bots.
pub fn reset() {
    COUNT.store(0, Ordering::Relaxed);
}

/// In a joined lobby: use the host's count (from its published session).
pub fn follow(n: u8) {
    if enabled() && COUNT.swap((n as usize).min(MAX), Ordering::Relaxed) != n as usize {
        crate::log!("bots: host has {n}");
    }
}

/// Bots with a network puppet class (uHm60..6A), see the module docs.
const MAX: usize = 11;

/// The slot -> bot class jump table: entries for slots 5..15 become the ones for slots 1..11.
const fn slot_table(e: [u32; 15], shifted: bool) -> [u8; 60] {
    let mut out = [0u8; 60];
    let mut i = 0;
    while i < 60 {
        let slot = i / 4;
        let src = if shifted && slot >= 4 { slot - 4 } else { slot };
        out[i] = e[src].to_le_bytes()[i % 4];
        i += 1;
    }
    out
}
const DX11_CLASS_JUMPS: [u32; 15] = [
    0x7E_3378, 0x7E_3332, 0x7E_3337, 0x7E_333C, 0x7E_3341, 0x7E_3346, 0x7E_334B, 0x7E_3350, 0x7E_3355, 0x7E_335A,
    0x7E_335F, 0x7E_3364, 0x7E_3369, 0x7E_336E, 0x7E_3373,
];
const DX9_CLASS_JUMPS: [u32; 15] = [
    0x7D_44B8, 0x7D_4472, 0x7D_4477, 0x7D_447C, 0x7D_4481, 0x7D_4486, 0x7D_448B, 0x7D_4490, 0x7D_4495, 0x7D_449A,
    0x7D_449F, 0x7D_44A4, 0x7D_44A9, 0x7D_44AE, 0x7D_44B3,
];
const SLOT_CLASSES: [Patch; 2] = [
    Patch {
        name: "bot slot classes (DX9)",
        exe: Exe::Dx9,
        addr: 0x007D_44C0,
        original: &slot_table(DX9_CLASS_JUMPS, false),
        patched: &slot_table(DX9_CLASS_JUMPS, true),
    },
    Patch {
        name: "bot slot classes (DX11)",
        exe: Exe::Dx11,
        addr: 0x007E_3380,
        original: &slot_table(DX11_CLASS_JUMPS, false),
        patched: &slot_table(DX11_CLASS_JUMPS, true),
    },
];

/// Template record for slot `i`, as it should sit in slot `i`.
pub fn template(i: usize) -> [u8; RECORD] {
    TEMPLATE[i * RECORD..(i + 1) * RECORD].try_into().unwrap()
}

/// Bot `k` (1-based: botTableNo 23+k, class uHm60+k-1) moved to its member slot k+4.
fn bot_record(k: usize) -> [u8; RECORD] {
    let mut r = template(k);
    (r[0], r[0x3C]) = ((k + 4) as u8, (k + 4) as u8);
    r
}

/// An unused member slot.
const EMPTY: [u8; RECORD] = {
    let mut r = [0u8; RECORD];
    r[0] = 0xFF;
    r
};

/// The bots in play: the last `count` of 1..=MAX, so they take the top slots.
fn bots() -> std::ops::RangeInclusive<usize> {
    MAX + 1 - count()..=MAX
}

/// The game's "add an archive to load list N" (DX11 0x7d2630; thiscall this, list, path, 0, 2).
fn add_archive(exe: Exe) -> u32 {
    match exe {
        Exe::Dx9 => 0x007C_37C0,
        Exe::Dx11 => 0x007D_2630,
    }
}

/// A bot's class (`uHm60`.. for slots 1..) brings its model and AI (`tool\AI\Think\HumanBotBt`) in
/// `arcLp2Xml\autoArc\uHm6X`. The stage loader queues those only for the Training stages, so online
/// a bot's character fails its resource check on activation and deletes itself (then its slot).
/// When the stage queues its area archive (`autoArc\aArea_*`), the bot classes are queued with it.
extern "stdcall" fn queue_bot_classes(_addr: u32, r: &mut crate::hook::Regs) -> u32 {
    use std::sync::atomic::{AtomicBool, Ordering};
    static BUSY: AtomicBool = AtomicBool::new(false);
    let Some(exe) = *EXE else { return 0 };
    let mut path = [0u8; 64];
    read_mem(r.stack(2) as usize, &mut path);
    let path = &path[..path.iter().position(|&b| b == 0).unwrap_or(64)];
    let area = path.starts_with(br"arcLp2Xml\autoArc\aArea_") && !path.ends_with(b"_scr");
    if !area || BUSY.swap(true, Ordering::SeqCst) {
        return 0;
    }
    type AddArchive = unsafe extern "thiscall" fn(this: u32, list: u32, path: *const u8, a: u32, b: u32);
    let add: AddArchive = unsafe { std::mem::transmute(add_archive(exe) as usize) };
    // Kept for the life of the process in case the load list keeps the pointer.
    static NAMES: std::sync::LazyLock<Vec<std::ffi::CString>> = std::sync::LazyLock::new(|| {
        (0..15).map(|i| std::ffi::CString::new(format!(r"arcLp2Xml\autoArc\uHm{:02x}", 0x60 + i)).unwrap()).collect()
    });
    for name in bots().map(|k| &NAMES[k - 1]) {
        unsafe { add(r.ecx, r.stack(1), name.as_ptr() as *const u8, 0, 2) };
    }
    crate::log!("bots: queued {} bot classes with {}", count(), String::from_utf8_lossy(path));
    BUSY.store(false, Ordering::SeqCst);
    0
}

/// The lobby's player list shows member slots with `isValid` (row list 0xe96ef0, row contents
/// 0xe98510), which bots never have. Each `cmp byte [record+0x1ac],1` jumps to a stub that also
/// accepts `isBot` (+0x1b1). Bots sit in the top slots, so their rows come after the players'
/// and the cursor/gamer-card code (players only) still lines up.
const LIST_CMPS: [(Exe, usize, [u8; 8]); 4] = [
    (Exe::Dx11, 0x00E9_6EF6, [0x80, 0xBC, 0x2A, 0xAC, 0x01, 0x00, 0x00, 0x01]), // cmp byte [edx+ebp+0x1ac],1
    (Exe::Dx11, 0x00E9_8523, [0x80, 0xBC, 0x11, 0xAC, 0x01, 0x00, 0x00, 0x01]), // cmp byte [ecx+edx+0x1ac],1
    (Exe::Dx9, 0x00E8_6E26, [0x80, 0xBC, 0x2A, 0xAC, 0x01, 0x00, 0x00, 0x01]),
    (Exe::Dx9, 0x00E8_8453, [0x80, 0xBC, 0x11, 0xAC, 0x01, 0x00, 0x00, 0x01]),
];

/// Stub code at `at` for `cmp` at `addr`: the cmp, `je` past an isBot cmp, then back to addr+8.
fn or_bot_stub(addr: u32, cmp: [u8; 8], at: u32) -> Vec<u8> {
    let mut bot = cmp;
    bot[3] = 0xB1; // disp32 0x1ac (isValid) -> 0x1b1 (isBot)
    let mut c = [&cmp[..], &[0x74, 8], &bot[..], &[0xE9]].concat();
    c.extend((addr + 8).wrapping_sub(at + c.len() as u32 + 4).to_le_bytes());
    c
}

unsafe fn or_bot(addr: usize, cmp: [u8; 8]) {
    use windows_sys::Win32::System::Memory::{VirtualAlloc, MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE};
    let mut now = [0u8; 8];
    let mem = VirtualAlloc(std::ptr::null(), 32, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) as *mut u8;
    if !read_mem(addr, &mut now) || now != cmp || mem.is_null() {
        crate::log!("bots: lobby list {addr:#x}: bytes differ, not patched");
        return;
    }
    let code = or_bot_stub(addr as u32, cmp, mem as u32);
    std::ptr::copy_nonoverlapping(code.as_ptr(), mem, code.len());
    let mut jmp = [0x90u8; 8];
    jmp[0] = 0xE9;
    jmp[1..5].copy_from_slice(&(mem as u32).wrapping_sub(addr as u32 + 5).to_le_bytes());
    write_mem(addr, &jmp);
}

/// Called once at startup.
pub fn install() {
    let Some(exe) = *EXE else { return };
    if enabled() {
        SLOT_CLASSES.iter().filter(|p| p.exe == exe).for_each(|p| drop(crate::patch::apply(p)));
        LIST_CMPS.iter().filter(|c| c.0 == exe).for_each(|c| unsafe { or_bot(c.1, c.2) });
        // 5 bytes: mov eax,[esp+4]; push esi. Pops 0x10 bytes of arguments.
        unsafe { crate::hook::install(add_archive(exe), 5, 0x10, queue_bot_classes) };
        crate::botmaps::prepare();
    }
}

/// F7/F8 pressed since the last frame, while this game's window has focus.
fn hotkey() -> Option<bool> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_F7, VK_F8};
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    static HELD: AtomicUsize = AtomicUsize::new(0);
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(GetForegroundWindow(), &mut pid) };
    let down = |k| pid == std::process::id() && unsafe { GetAsyncKeyState(k as i32) } < 0;
    let now = down(VK_F7) as usize | (down(VK_F8) as usize) << 1;
    let pressed = now & !HELD.swap(now, Ordering::Relaxed);
    (pressed != 0).then_some(pressed & 1 != 0)
}

/// Shows the count in the window title (the lobby has no place for it).
fn show_count(n: usize) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, SetWindowTextW};
    let title: Vec<u16> = format!("Lost Planet 2 - bots: {n} (F7 add, F8 remove)\0").encode_utf16().collect();
    unsafe { SetWindowTextW(GetForegroundWindow(), title.as_ptr()) };
}

/// Main thread, every frame.
pub fn on_frame() {
    let Some(exe) = *EXE else { return };
    if !enabled() {
        return;
    }
    let g = globals(exe);
    let (Some(rule), Some(net), Some(game)) = (ptr(g.rule), ptr(g.network), ptr(g.game)) else { return };
    // Online (game manager +0x25f is the offline/Training flag).
    if byte(game + 0x25F) != Some(0) {
        return;
    }
    let mut records = vec![0u8; RECORD * 16];
    if !read_mem(rule + RECORDS, &mut records) {
        return;
    }
    let rec = |i: usize| &records[i * RECORD..(i + 1) * RECORD];
    // Only in a lobby (a player record exists) and before the match: then the players' records
    // become game members and the game owns the bot records too.
    if !(0..16).any(|i| rec(i)[IS_VALID] == 1) || (0..16).any(|i| rec(i)[IS_VALID] == 1 && rec(i)[IS_GAME_MEMBER] == 1) {
        return;
    }
    // Hosting (sGameNetwork.mIsHost): the hotkeys change the count, sessions publish it.
    if byte(net + 0x28) == Some(1) {
        if let Some(add) = hotkey() {
            let n = if add { (count() + 1).min(MAX) } else { count().saturating_sub(1) };
            COUNT.store(n, Ordering::Relaxed);
            show_count(n);
            crate::session::refresh();
        }
    }
    let mut changed = vec![];
    // shortcut: players + bots > 16 collide on a slot, cap bots by joiners if that matters.
    for k in 1..=MAX {
        let i = k + 4;
        let want = if bots().contains(&k) {
            bot_record(k)
        } else if rec(i)[IS_BOT] == 1 {
            EMPTY
        } else {
            continue;
        };
        if rec(i)[IS_VALID] == 0 && rec(i) != want && write_mem(rule + RECORDS + i * RECORD, &want) {
            changed.push(i);
        }
    }
    if !changed.is_empty() {
        crate::log!("bots: {} in play, slots {changed:?} changed", count());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_slots_are_bots() {
        for i in 1..16 {
            let t = template(i);
            assert_eq!(t[0] as usize, i, "slot index");
            assert_eq!(t[IS_BOT], 1, "isBot");
            assert_eq!(t[IS_VALID], 0, "bots are never valid players");
            assert!(t[0x2c] != 0, "has a name");
        }
    }

    #[test]
    fn bot_record_moves_only_the_slot_index() {
        let (t, r) = (template(11), bot_record(11));
        assert_eq!((r[0], r[0x3C], r[0x46]), (15, 15, 34), "slot 15, botTableNo of bot 11 (uHm6A)");
        assert!((0..RECORD).filter(|&o| o != 0 && o != 0x3C).all(|o| r[o] == t[o]));
    }

    #[test]
    fn or_bot_stub_checks_both_flags_and_returns() {
        let c = or_bot_stub(0x1000, LIST_CMPS[0].2, 0x5000);
        assert_eq!(&c[..8], &LIST_CMPS[0].2);
        assert_eq!((c[8], c[9]), (0x74, 8), "je over the isBot cmp");
        assert_eq!(c[13], 0xB1, "second cmp reads isBot");
        let back = (0x5000 + c.len() as u32).wrapping_add(u32::from_le_bytes(c[c.len() - 4..].try_into().unwrap()));
        assert_eq!(back, 0x1008);
    }

    #[test]
    fn slot_table_maps_slots_5_to_15_onto_classes_60_to_6a() {
        let t = slot_table(DX11_CLASS_JUMPS, true);
        let entry = |slot: usize| u32::from_le_bytes(t[(slot - 1) * 4..slot * 4].try_into().unwrap());
        assert_eq!(entry(1), DX11_CLASS_JUMPS[0]);
        assert_eq!(entry(5), DX11_CLASS_JUMPS[0], "slot 5 -> uHm60");
        assert_eq!(entry(15), DX11_CLASS_JUMPS[10], "slot 15 -> uHm6A");
    }
}

//! Versus bots. LP2 already has them: Training -> AI Enemy Battle fills the free member slots of
//! sNetRuleMediator with bot records (`isBot`, a `botTableNo` naming the bot's look and AI, its name
//! and custom parts) and the match spawns an AI-driven character for every such slot.
//!
//! `[bots] count = N` in lp2mods.ini: while this PC hosts an online lobby, member slots 1..=N that
//! no player uses get those same records (`data/bot_records.bin`, copied from an AI Enemy Battle).
//! A player joining later takes a slot as usual (its record has `isValid`, which bots never have).
//!
//! Member record (0x7c bytes at sNetRuleMediator+0x148+i*0x7c): +0x00 index, +0x2c name,
//! +0x3c index, +0x46 botTableNo, +0x64 isValid, +0x65 isGameMember, +0x66 isStartMember,
//! +0x69 isBot.
//!
//! Online also lacks what Training loads for them: the bot classes (queued below with the stage)
//! and the maps' AI navigation data (`botmaps`), so bots work on the 4 maps that have Training
//! versions (`botmaps::PAIRS`; Cube is one).

use crate::mods::{read_mem, write_mem, Exe, EXE, MODS};

const RECORD: usize = 0x7C;
const RECORDS: usize = 0x148;
const IS_VALID: usize = 0x64;
const IS_GAME_MEMBER: usize = 0x65;
#[cfg(test)]
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

pub fn count() -> usize {
    MODS.ini
        .get("bots")
        .and_then(|s| s.iter().find(|(k, _)| k == "count"))
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(0)
        .min(15)
}

/// Template record for slot `i`, as it should sit in slot `i`.
pub fn template(i: usize) -> [u8; RECORD] {
    TEMPLATE[i * RECORD..(i + 1) * RECORD].try_into().unwrap()
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
    for name in &NAMES[..count()] {
        unsafe { add(r.ecx, r.stack(1), name.as_ptr() as *const u8, 0, 2) };
    }
    crate::log!("bots: queued {} bot classes with {}", count(), String::from_utf8_lossy(path));
    BUSY.store(false, Ordering::SeqCst);
    0
}

/// Called once at startup.
pub fn install() {
    let Some(exe) = *EXE else { return };
    if count() > 0 {
        // 5 bytes: mov eax,[esp+4]; push esi. Pops 0x10 bytes of arguments.
        unsafe { crate::hook::install(add_archive(exe), 5, 0x10, queue_bot_classes) };
        crate::botmaps::prepare();
    }
}

/// Main thread, every frame.
pub fn on_frame() {
    let n = count();
    let Some(exe) = *EXE else { return };
    if n == 0 {
        return;
    }
    let g = globals(exe);
    let (Some(rule), Some(net), Some(game)) = (ptr(g.rule), ptr(g.network), ptr(g.game)) else { return };
    // Online (game manager +0x25f is the offline/Training flag) and hosting (sGameNetwork.mIsHost).
    if byte(game + 0x25F) != Some(0) || byte(net + 0x28) != Some(1) {
        return;
    }
    let mut records = vec![0u8; RECORD * 16];
    if !read_mem(rule + RECORDS, &mut records) {
        return;
    }
    let rec = |i: usize| &records[i * RECORD..(i + 1) * RECORD];
    // Only in the lobby: once the match starts the players' records become game members and the
    // game owns the bot records too.
    if (0..16).any(|i| rec(i)[IS_VALID] == 1 && rec(i)[IS_GAME_MEMBER] == 1) {
        return;
    }
    let mut placed = vec![];
    for i in 1..=n {
        let want = template(i);
        if rec(i)[IS_VALID] == 0 && rec(i) != want && write_mem(rule + RECORDS + i * RECORD, &want) {
            placed.push(i);
        }
    }
    if !placed.is_empty() {
        crate::log!("bots: placed in slots {placed:?}");
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
}

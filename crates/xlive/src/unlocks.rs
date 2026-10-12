//! Customization unlocks. LP2 keeps "owned" flags on the game manager (sGameManager, instance
//! pointer DX9 0x1842020 / DX11 0x18520c0) and copies them verbatim into the save
//! (research/SYSTEMS.md, "Save file").
//!
//! Parts: one bit per part variant, most significant bit first in each u32. A part's bit is its
//! index in the game's per-category table (DX11 lookup 0xf5c9a0); bits 180..249 are a shared table
//! of special parts. Titles: 2 bits per title id 0x30.., state 1 = earned, 2 = seen.
//!
//! Weapons (incl. grenades): 120 bits at 0x509c8; abilities: 42 bits at 0x501fc. Same MSB-first
//! layout. Both counts come from the game's own "own them all" achievement checks (DX11 0x7f9080
//! weapons, 0x7f5750 abilities); the 17 special weapons (ids at DX11 0x17450b0) are real items too.
//!
//! `[unlocks] parts / titles / weapons / abilities = true` mark every real item owned and clear bits
//! that do not name a real item (selecting such a phantom entry crashes the game).

use crate::mods::{read_mem, write_mem, Exe, EXE, MODS};

/// (flag offset, parts to unlock) for heads, bodies, backpacks, legs: the game's table sizes,
/// except bodies. Body table entries 123..172 (set 0x2d, `bd2dXX`) have no files on PC and loading
/// one fails ("Failed to open file ...\CustomParts\bd2d00.arc"), so only 0..122 are real.
/// Table id 0xAABBCCDD -> file `<prefix><set><item>[<variant>]`, set = 0x20 + DD (0x0f stays 0f),
/// item = BB, variant = CC; checked against nativePC\arcXmlTmpPC\CustomParts.
const PARTS: [(usize, u32); 4] = [(0x50710, 134), (0x50730, 123), (0x50770, 123), (0x50750, 123)];
/// Bits 180..250: special parts, any category; left as the game set them.
const SPECIAL: std::ops::Range<u32> = 180..250;
const TITLES_OFFSET: usize = 0x507EC;
const WEAPONS: (usize, u32) = (0x509C8, 120);
/// What My Page -> Customization -> Weapons actually offers: one byte per slot, bit n = variant n
/// owned (bit 0 is the default). 5 gun slots x 7 variants, then 5 grenade slots x 5 variants
/// (DX11 checks 0x7de8f0 / 0x7de920, used by the Weapons tab 0xeee250; the "Weapons %" is out of 60).
/// A slot only opens when it owns a variant besides the default.
const WEAPON_VARIANTS: usize = 0x5088C;
const ABILITIES: (usize, u32) = (0x501FC, 42);
/// Title ids 0x30..=0x20b (contiguous, from the game's title table at DX11 0x1744b90).
const TITLES: u32 = 476;

fn game_manager(exe: Exe) -> Option<usize> {
    let global = match exe {
        Exe::Dx9 => 0x0184_2020,
        Exe::Dx11 => 0x0185_20C0,
    };
    let mut b = [0u8; 4];
    read_mem(global, &mut b).then_some(())?;
    Some(u32::from_le_bytes(b) as usize).filter(|&p| p != 0)
}

fn enabled(key: &str) -> bool {
    MODS.ini.get("unlocks").is_some_and(|s| s.iter().any(|(k, v)| k == key && (v == "true" || v == "1")))
}

/// MSB-first bitfield: bits below `count` set, other bits cleared unless in `keep`.
pub fn unlock_bits(words: &mut [u32], count: u32, keep: std::ops::Range<u32>) {
    for i in 0..words.len() as u32 * 32 {
        let (w, mask) = ((i >> 5) as usize, 0x8000_0000u32 >> (i & 31));
        if i < count {
            words[w] |= mask;
        } else if !keep.contains(&i) {
            words[w] &= !mask;
        }
    }
}

/// Owned flags for one part category: real parts set, phantom bits cleared, specials kept.
pub fn unlock_parts(words: &mut [u32; 8], count: u32) {
    unlock_bits(words, count, SPECIAL);
}

/// Title states: every real title at least "earned" (1), slots past the last title cleared.
pub fn unlock_titles(words: &mut [u32; 32]) {
    for t in 0..512u32 {
        let (w, shift) = ((t >> 4) as usize, (t & 15) * 2);
        if t < TITLES {
            if (words[w] >> shift) & 3 == 0 {
                words[w] |= 1 << shift;
            }
        } else {
            words[w] &= !(3 << shift);
        }
    }
}

/// Weapon slot variants: 10 bytes from `WEAPON_VARIANTS` (in 3 words; the last 2 bytes are not
/// ours and are left alone).
pub fn unlock_variants(words: &mut [u32; 3]) {
    let mut b: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    b[..5].iter_mut().for_each(|x| *x |= 0x7F);
    b[5..10].iter_mut().for_each(|x| *x |= 0x1F);
    for (w, c) in words.iter_mut().zip(b.chunks(4)) {
        *w = u32::from_le_bytes(c.try_into().unwrap());
    }
}

/// Applies `f` to `N` u32s at `addr`; writes back only if something changed.
fn modify<const N: usize>(addr: usize, f: impl FnOnce(&mut [u32; N])) -> bool {
    let mut raw = vec![0u8; N * 4];
    if !read_mem(addr, &mut raw) {
        return false;
    }
    let mut words = [0u32; N];
    for (w, c) in words.iter_mut().zip(raw.chunks(4)) {
        *w = u32::from_le_bytes(c.try_into().unwrap());
    }
    let before = words;
    f(&mut words);
    if words == before {
        return false;
    }
    let out: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    write_mem(addr, &out)
}

/// Main thread, every frame. The game reloads these fields from the save (e.g. DX11 0x7e905c for
/// weapons) at times, so they are checked every frame; writing happens only when something differs.
pub fn on_frame(_frame: u32) {
    let Some(gm) = EXE.and_then(game_manager) else { return };
    let names = ["heads", "bodies", "backpacks", "legs"];
    let mut changed: Vec<&str> = vec![];
    if enabled("parts") {
        for ((off, n), name) in PARTS.iter().zip(names) {
            if modify::<8>(gm + off, |w| unlock_parts(w, *n)) {
                changed.push(name);
            }
        }
    }
    if enabled("titles") && modify::<32>(gm + TITLES_OFFSET, unlock_titles) {
        changed.push("titles");
    }
    if enabled("weapons") && modify::<4>(gm + WEAPONS.0, |w| unlock_bits(w, WEAPONS.1, 0..0)) {
        changed.push("weapons");
    }
    if enabled("weapons") && modify::<3>(gm + WEAPON_VARIANTS, unlock_variants) {
        changed.push("weapon variants");
    }
    if enabled("abilities") && modify::<2>(gm + ABILITIES.0, |w| unlock_bits(w, ABILITIES.1, 0..0)) {
        changed.push("abilities");
    }
    if !changed.is_empty() {
        // Rate-limited: a field the game resets every frame would otherwise flood the log.
        static LOGGED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        if LOGGED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 200 {
            crate::log!("unlocks: re-applied {}", changed.join(", "));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_set_real_and_clear_phantoms() {
        let mut w = [u32::MAX; 8];
        unlock_parts(&mut w, 134);
        let bit = |i: u32| w[(i >> 5) as usize] & (0x8000_0000 >> (i & 31)) != 0;
        assert!((0..134).all(bit));
        assert!(!(134..180).any(bit));
        assert!((180..250).all(bit), "special parts untouched");
        assert!(!(250..256).any(bit));
        let mut z = [0u32; 8];
        unlock_parts(&mut z, 1);
        assert_eq!(z[0], 0x8000_0000, "most significant bit first");
    }

    #[test]
    fn weapons_and_abilities_bounds() {
        let mut w = [u32::MAX; 4];
        unlock_bits(&mut w, 120, 0..0);
        assert_eq!(w, [u32::MAX, u32::MAX, u32::MAX, 0xFFFF_FF00]);
        let mut a = [0u32; 2];
        unlock_bits(&mut a, 42, 0..0);
        assert_eq!(a, [u32::MAX, 0xFFC0_0000]);
    }

    #[test]
    fn weapon_variants_fill_ten_slots_only() {
        let mut w = [0x0101_0101, 0x0101_0101, 0xAABB_0101];
        unlock_variants(&mut w);
        assert_eq!(w, [0x7F7F_7F7F, 0x1F1F_1F7F, 0xAABB_1F1F]);
    }

    #[test]
    fn titles_earned_and_phantoms_cleared() {
        let mut w = [u32::MAX; 32];
        w[0] = 0b10; // title 0x30 already "seen" only
        unlock_titles(&mut w);
        let state = |t: u32| (w[(t >> 4) as usize] >> ((t & 15) * 2)) & 3;
        assert_eq!(state(0), 2, "existing state kept");
        assert!((1..TITLES).all(|t| state(t) != 0));
        assert!((TITLES..512).all(|t| state(t) == 0));
    }
}

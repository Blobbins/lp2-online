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
//!
//! `weapons_by_level = N` / `abilities_by_level = N` instead unlock them with the online career level:
//! at level L, (L-1)/(N-1) of them (everything at level N). The original game only hands these out
//! at random on the campaign's chapter results (0x7fd2a0); this only adds to what a profile owns.
//! Career level: points at game manager +0x509fc, level = 1 + the per-level costs (table DX11
//! 0x170b0e0) the points cover, at most 99 (as the game's own 0xf03075).

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
const CAREER_POINTS: usize = 0x509FC;
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

/// `[unlocks] key = N` (a career level, 2..=99).
fn by_level(key: &str) -> Option<u32> {
    let v = MODS.ini.get("unlocks")?.iter().find(|(k, _)| k == key)?.1.parse::<u32>().ok()?;
    Some(v.clamp(2, 99))
}

fn level_costs(exe: Exe) -> Option<[u32; 100]> {
    let addr = match exe {
        Exe::Dx9 => 0x016F_B0E0,
        Exe::Dx11 => 0x0170_B0E0,
    };
    let mut raw = [0u8; 400];
    read_mem(addr, &mut raw).then(|| std::array::from_fn(|i| u32::from_le_bytes(raw[i * 4..i * 4 + 4].try_into().unwrap())))
}

/// The game's career level for `points` (cost of level n+1 at `costs[n]`).
pub fn career_level(points: u32, costs: &[u32; 100]) -> u32 {
    let (mut level, mut need) = (1usize, costs[1]);
    while need <= points && level < 99 {
        level += 1;
        need = need.saturating_add(costs[level]);
    }
    level as u32
}

/// Share of the items unlocked at `level` when all are unlocked at `by`.
pub fn share(level: u32, by: u32) -> f32 {
    ((level.saturating_sub(1)) as f32 / (by - 1) as f32).min(1.0)
}

/// Sets the first `k` bits (MSB-first), leaving the rest as they are.
pub fn grant_bits(words: &mut [u32], k: u32) {
    for i in 0..k.min(words.len() as u32 * 32) {
        words[(i >> 5) as usize] |= 0x8000_0000u32 >> (i & 31);
    }
}

/// Weapon slot variants for `share`: each slot gets that share of its non-default variants.
pub fn grant_variants(words: &mut [u32; 3], share: f32) {
    let mut b: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    for (i, x) in b[..10].iter_mut().enumerate() {
        let extra = if i < 5 { 6 } else { 4 }; // 7 gun / 5 grenade variants, bit 0 the default
        let n = (share * extra as f32).round() as u32;
        *x |= ((1u16 << (n + 1)) - 1) as u8;
    }
    for (w, c) in words.iter_mut().zip(b.chunks(4)) {
        *w = u32::from_le_bytes(c.try_into().unwrap());
    }
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
    let (wl, al) = (by_level("weapons_by_level").filter(|_| !enabled("weapons")), by_level("abilities_by_level").filter(|_| !enabled("abilities")));
    if wl.is_some() || al.is_some() {
        let mut points = [0u8; 4];
        let Some(costs) = EXE.and_then(level_costs).filter(|_| read_mem(gm + CAREER_POINTS, &mut points)) else { return };
        let level = career_level(u32::from_le_bytes(points), &costs);
        static SHOWN: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        if SHOWN.swap(level, std::sync::atomic::Ordering::Relaxed) != level {
            crate::log!("unlocks: career level {level}");
        }
        if let Some(by) = wl {
            let sh = share(level, by);
            if modify::<4>(gm + WEAPONS.0, |w| grant_bits(w, (sh * WEAPONS.1 as f32).round() as u32))
                | modify::<3>(gm + WEAPON_VARIANTS, |w| grant_variants(w, sh))
            {
                changed.push("weapons for the level");
            }
        }
        if let Some(by) = al {
            if modify::<2>(gm + ABILITIES.0, |w| grant_bits(w, (share(level, by) * ABILITIES.1 as f32).round() as u32)) {
                changed.push("abilities for the level");
            }
        }
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
    fn career_levels_follow_the_cost_table() {
        let mut costs = [0u32; 100];
        (1..100).for_each(|i| costs[i] = 900 + 100 * i as u32); // 1000, 1100, 1200, ...
        assert_eq!(career_level(200, &costs), 1, "200 pts is Lv.1, next at 1000");
        assert_eq!(career_level(1000, &costs), 2);
        assert_eq!(career_level(2099, &costs), 2);
        assert_eq!(career_level(2100, &costs), 3);
        assert_eq!(career_level(u32::MAX, &costs), 99);
    }

    #[test]
    fn level_grants_add_only() {
        assert_eq!((share(1, 20), share(20, 20), share(50, 20)), (0.0, 1.0, 1.0));
        let mut w = [0x0000_0001u32, 0];
        grant_bits(&mut w, 3);
        assert_eq!(w, [0xE000_0001, 0], "first 3 set, others kept");
        let mut v = [0u32; 3];
        grant_variants(&mut v, 0.0);
        assert_eq!(v[0].to_le_bytes(), [1, 1, 1, 1], "level 1: defaults only");
        grant_variants(&mut v, 1.0);
        let b: Vec<u8> = v.iter().flat_map(|w| w.to_le_bytes()).collect();
        assert_eq!(&b[..10], &[0x7F, 0x7F, 0x7F, 0x7F, 0x7F, 0x1F, 0x1F, 0x1F, 0x1F, 0x1F]);
    }

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

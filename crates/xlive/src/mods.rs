//! In-process changes to Lost Planet 2 itself (the "hybrid" rewrite layer).
//!
//! Tuning: the game registers ~2,500 named 4-byte globals (ability ratios, Vital Suit and
//! weapon parameters, data post ranges...). `research/tools/tuning.py` recovers name -> address
//! per exe into `data/tuning_*.txt`. This module dumps their live values to
//! `lp2tuning_dump.txt` and applies overrides from `lp2mods.ini`.

use crate::{exe_dir, log};
use std::collections::BTreeMap;
use std::sync::LazyLock;
use windows_sys::Win32::System::Diagnostics::Debug::{ReadProcessMemory, WriteProcessMemory};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Exe {
    Dx9,
    Dx11,
}

/// Each exe is identified by a class-name string at an address only that build has.
const SIGNATURES: [(Exe, usize); 2] = [(Exe::Dx9, 0x0163_DF94), (Exe::Dx11, 0x0164_E0D4)];

pub fn read_mem(addr: usize, buf: &mut [u8]) -> bool {
    // ReadProcessMemory fails cleanly on unmapped memory instead of crashing the game.
    unsafe { ReadProcessMemory(GetCurrentProcess(), addr as _, buf.as_mut_ptr() as _, buf.len(), std::ptr::null_mut()) != 0 }
}

pub fn write_mem(addr: usize, data: &[u8]) -> bool {
    unsafe { WriteProcessMemory(GetCurrentProcess(), addr as _, data.as_ptr() as _, data.len(), std::ptr::null_mut()) != 0 }
}

pub static EXE: LazyLock<Option<Exe>> = LazyLock::new(|| {
    let found = SIGNATURES.iter().find(|(_, addr)| {
        let mut b = [0u8; 9];
        read_mem(*addr, &mut b) && &b == b"cNetRule\0"
    });
    log!("game exe: {:?}", found.map(|f| f.0));
    found.map(|f| f.0)
});

fn tuning_table(exe: Exe) -> BTreeMap<&'static str, usize> {
    let text = match exe {
        Exe::Dx9 => include_str!("../data/tuning_dx9.txt"),
        Exe::Dx11 => include_str!("../data/tuning_dx11.txt"),
    };
    text.lines()
        .filter_map(|l| l.split_once(' '))
        .filter_map(|(name, addr)| Some((name, usize::from_str_radix(addr, 16).ok()?)))
        .collect()
}

/// A 4-byte value as written in lp2mods.ini: `1.5` / `2.0` is a float, `15` an integer.
pub fn parse_value(v: &str) -> Option<[u8; 4]> {
    let v = v.trim();
    if v.contains(['.', 'e', 'E']) {
        v.parse::<f32>().ok().filter(|f| f.is_finite()).map(f32::to_le_bytes)
    } else {
        v.parse::<i32>().ok().map(i32::to_le_bytes)
    }
}

/// Shows a value the way `parse_value` reads it back: floats keep a decimal point.
pub fn format_value(raw: [u8; 4]) -> String {
    let i = i32::from_le_bytes(raw);
    let f = f32::from_le_bytes(raw);
    // Small integers and zero read as integers; anything with a sane float exponent as a float.
    if i.unsigned_abs() < 0x0010_0000 {
        i.to_string()
    } else if f.is_finite() && f.abs() > 1e-6 && f.abs() < 1e9 {
        format!("{f:?}")
    } else {
        i.to_string()
    }
}

/// `key = value` lines grouped by `[section]`; lines before any section count as `[tuning]`.
pub fn parse_ini(text: &str) -> BTreeMap<String, Vec<(String, String)>> {
    let mut out: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    let mut section = "tuning".to_string();
    for line in text.lines().map(|l| l.split(['#', ';']).next().unwrap_or("").trim()) {
        if let Some(s) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = s.trim().to_ascii_lowercase();
            out.entry(section.clone()).or_default(); // a section can matter with no keys (`[bots]`)
        } else if let Some((k, v)) = line.split_once('=') {
            out.entry(section.clone()).or_default().push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    out
}

pub fn parse_overrides(entries: &[(String, String)]) -> Vec<(String, [u8; 4])> {
    entries
        .iter()
        .filter_map(|(k, v)| {
            let parsed = parse_value(v);
            if parsed.is_none() {
                log!("lp2mods.ini: bad value for {k}: {v:?}");
            }
            Some((k.clone(), parsed?))
        })
        .collect()
}

pub struct Mods {
    /// (address, value) to keep applied.
    overrides: Vec<(usize, [u8; 4])>,
    /// `[rules]`: match fields written while hosting.
    pub rules: Vec<(&'static crate::rules::Field, Vec<u8>)>,
    /// Every section of lp2mods.ini, for modules with their own settings (`[startup]`...).
    pub ini: BTreeMap<String, Vec<(String, String)>>,
    /// Identifies this mod set so only matching players see each other's lobbies.
    pub hash: u64,
}

pub static MODS: LazyLock<Mods> = LazyLock::new(|| {
    let text = std::fs::read_to_string(exe_dir().join("lp2mods.ini")).unwrap_or_default();
    let ini = parse_ini(&text);
    let wanted = parse_overrides(ini.get("tuning").map_or(&[][..], |v| v));
    let rules = crate::rules::parse(ini.get("rules").map_or(&[][..], |v| v));
    let table = EXE.map(tuning_table).unwrap_or_default();
    let mut overrides = vec![];
    for (name, value) in &wanted {
        match table.get(name.as_str()) {
            Some(&addr) => overrides.push((addr, *value)),
            None => log!("lp2mods.ini: unknown tuning name {name}"),
        }
    }
    // Every applied change, tuning and rules, decides who can play together.
    let mut all: Vec<(String, Vec<u8>)> = wanted.iter().map(|(k, v)| (k.clone(), v.to_vec())).collect();
    all.extend(rules.iter().map(|(f, v)| (format!("rules.{}", f.name), v.clone())));
    let hash = if overrides.is_empty() && rules.is_empty() { 0 } else { fnv64(&all) };
    log!("lp2mods.ini: {} tuning and {} rules override(s), mod set {hash:016x}", overrides.len(), rules.len());
    Mods { overrides, rules, ini, hash }
});

fn fnv64(items: &[(String, Vec<u8>)]) -> u64 {
    let mut sorted: Vec<_> = items.iter().collect();
    sorted.sort();
    let mut h = 0xCBF2_9CE4_8422_2325u64;
    for (k, v) in sorted {
        for b in k.bytes().chain(v.iter().copied()) {
            h = (h ^ b as u64).wrapping_mul(0x0100_0000_01B3);
        }
    }
    h
}

fn apply() {
    for (addr, value) in &MODS.overrides {
        write_mem(*addr, value);
    }
}

fn dump(exe: Exe) {
    let mut out = String::from("# Live tuning values. Copy lines into lp2mods.ini to change them.\n");
    for (name, addr) in tuning_table(exe) {
        let mut b = [0u8; 4];
        if read_mem(addr, &mut b) {
            out += &format!("{name} = {}\n", format_value(b));
        }
    }
    match std::fs::write(exe_dir().join("lp2tuning_dump.txt"), out) {
        Ok(()) => log!("wrote lp2tuning_dump.txt"),
        Err(e) => log!("lp2tuning_dump.txt: {e}"),
    }
}

/// Called every frame from XLiveRender (the game's main thread).
pub fn on_frame() {
    static FRAME: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let Some(exe) = *EXE else { return };
    let n = FRAME.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    crate::rules::on_frame(n);
    crate::unlocks::on_frame(n);
    crate::bots::on_frame();
    // shortcut: overrides are re-applied every ~5 s at 60 fps, so a value the game resets on a
    // level load is wrong for up to 5 s. Hook the reset site instead once it is found.
    if n % 300 == 0 {
        apply();
    }
    // By ~10 s the game has finished startup and set its defaults.
    if n == 600 {
        dump(exe);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_roundtrip_through_text() {
        for raw in [1.5f32.to_le_bytes(), 2.0f32.to_le_bytes(), 15i32.to_le_bytes(), (-3i32).to_le_bytes(), [0; 4]] {
            assert_eq!(parse_value(&format_value(raw)), Some(raw), "{}", format_value(raw));
        }
        assert_eq!(parse_value("nan"), None);
    }

    #[test]
    fn overrides_parse() {
        let ini = parse_ini("A = 1.5 # x\n[Rules]\nR = 2\n[tuning]\nB=7\nbad = x\n");
        assert_eq!(ini["rules"], vec![("R".to_string(), "2".to_string())]);
        let o = parse_overrides(&ini["tuning"]);
        assert_eq!(o, vec![("A".into(), 1.5f32.to_le_bytes()), ("B".into(), 7i32.to_le_bytes())]);
        let o: Vec<(String, Vec<u8>)> = o.into_iter().map(|(k, v)| (k, v.to_vec())).collect();
        assert_ne!(fnv64(&o), fnv64(&o[..1]));
    }

    #[test]
    fn keyless_section_is_kept() {
        let ini = parse_ini("[bots]
# comment only
[startup]
x = 1
");
        assert_eq!(ini["bots"], vec![]);
        assert!(!ini.contains_key("watch"));
    }

    #[test]
    fn tables_load() {
        let t = tuning_table(Exe::Dx9);
        assert_eq!(t.get("AbilityCritical_DamageUpRatio"), Some(&0x0184_1710));
        assert!(tuning_table(Exe::Dx11).len() >= t.len());
    }

    #[test]
    fn memory_access_is_safe() {
        let mut b = [0u8; 4];
        assert!(!read_mem(0x10, &mut b));
        let x = 5u32;
        assert!(read_mem(&x as *const u32 as usize, &mut b));
        assert_eq!(u32::from_le_bytes(b), 5);
    }
}

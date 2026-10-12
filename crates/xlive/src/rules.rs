//! Multiplayer match rules, read and changed through the game's own objects.
//!
//! LP2's multiplayer state lives in three engine singletons; their fields come from the game's
//! reflection data (`research/tools/props.py` -> `data/fields_mp.txt`, same offsets in DX9/DX11):
//! - `NetRule`     sNetRuleMediator: mission timer, game finished, active members, data posts...
//! - `InfoGame`    cInfoGame inside sInfoManager (+0x3e0): time limit, per-player kills/deaths...
//! - `GameNetwork` sGameNetwork: host flag, lobby size, network errors.
//!
//! `[rules]` entries in lp2mods.ini are written only while this player hosts (the host runs the match).

use crate::mods::{read_mem, write_mem, Exe, EXE};
use crate::{exe_dir, log};
use std::collections::BTreeMap;
use std::sync::{LazyLock, Mutex};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Owner {
    NetRule,
    InfoGame,
    GameNetwork,
}

#[derive(Debug, PartialEq)]
pub struct Field {
    pub name: &'static str,
    pub owner: Owner,
    pub offset: usize,
    /// MtProperty type: 3 bool, 4 u8, 5 u16, 6 u32, 7 u64, 8 s8, 9 s16, 10 s32, 11 s64, 12 f32.
    pub ty: u8,
}

impl Field {
    fn size(&self) -> usize {
        match self.ty {
            3 | 4 | 8 => 1,
            5 | 9 => 2,
            7 | 11 => 8,
            _ => 4,
        }
    }

    /// Text -> bytes for this field's type, or None if the value does not fit.
    pub fn encode(&self, v: &str) -> Option<Vec<u8>> {
        let v = v.trim();
        if self.ty == 12 {
            return v.parse::<f32>().ok().filter(|f| f.is_finite()).map(|f| f.to_le_bytes().to_vec());
        }
        let n: i64 = match v {
            "true" => 1,
            "false" => 0,
            _ => v.parse().ok()?,
        };
        let (lo, hi): (i64, i64) = match self.ty {
            3 => (0, 1),
            4 => (0, u8::MAX as i64),
            5 => (0, u16::MAX as i64),
            6 => (0, u32::MAX as i64),
            8 => (i8::MIN as i64, i8::MAX as i64),
            9 => (i16::MIN as i64, i16::MAX as i64),
            10 => (i32::MIN as i64, i32::MAX as i64),
            _ => (i64::MIN, i64::MAX),
        };
        (lo..=hi).contains(&n).then(|| n.to_le_bytes()[..self.size()].to_vec())
    }

    pub fn decode(&self, b: &[u8]) -> String {
        let mut buf = [0u8; 8];
        buf[..b.len()].copy_from_slice(b);
        let u = u64::from_le_bytes(buf);
        match self.ty {
            3 => (u != 0).to_string(),
            12 => format!("{:?}", f32::from_bits(u as u32)),
            8 => (u as i8).to_string(),
            9 => (u as i16).to_string(),
            10 => (u as i32).to_string(),
            11 => (u as i64).to_string(),
            _ => u.to_string(),
        }
    }
}

pub static FIELDS: LazyLock<Vec<Field>> = LazyLock::new(|| {
    include_str!("../data/fields_mp.txt")
        .lines()
        .filter_map(|l| {
            let mut it = l.split(' ');
            let (path, off, ty) = (it.next()?, it.next()?, it.next()?);
            let owner = match path.split_once('.')?.0 {
                "NetRule" => Owner::NetRule,
                "InfoGame" => Owner::InfoGame,
                "GameNetwork" => Owner::GameNetwork,
                _ => return None,
            };
            Some(Field { name: path, owner, offset: usize::from_str_radix(off, 16).ok()?, ty: ty.parse().ok()? })
        })
        .collect()
});

pub fn field(name: &str) -> Option<&'static Field> {
    FIELDS.iter().find(|f| f.name == name)
}

/// Global holding each singleton's instance pointer (the slot just before its MtDTI).
fn instance_global(exe: Exe, owner: Owner) -> usize {
    match (exe, owner) {
        (Exe::Dx9, Owner::NetRule) => 0x0184_27B0,
        (Exe::Dx9, Owner::InfoGame) => 0x0184_2198,
        (Exe::Dx9, Owner::GameNetwork) => 0x0184_20A4,
        (Exe::Dx11, Owner::NetRule) => 0x0185_2850,
        (Exe::Dx11, Owner::InfoGame) => 0x0185_2238,
        (Exe::Dx11, Owner::GameNetwork) => 0x0185_2144,
    }
}

fn base(exe: Exe, owner: Owner) -> Option<usize> {
    let mut b = [0u8; 4];
    read_mem(instance_global(exe, owner), &mut b).then_some(())?;
    let p = u32::from_le_bytes(b) as usize;
    // cInfoGame is embedded in sInfoManager.
    (p != 0).then(|| if owner == Owner::InfoGame { p + 0x3E0 } else { p })
}

pub fn read(exe: Exe, f: &Field) -> Option<Vec<u8>> {
    let mut b = vec![0u8; f.size()];
    read_mem(base(exe, f.owner)? + f.offset, &mut b).then_some(b)
}

fn write(exe: Exe, f: &Field, v: &[u8]) -> bool {
    base(exe, f.owner).is_some_and(|b| write_mem(b + f.offset, v))
}

/// `[rules]` lines -> (field, encoded value). Bad names/values are logged and skipped.
pub fn parse(entries: &[(String, String)]) -> Vec<(&'static Field, Vec<u8>)> {
    entries
        .iter()
        .filter_map(|(k, v)| {
            let Some(f) = field(k) else {
                log!("lp2mods.ini [rules]: unknown field {k} (see lp2rules_dump.txt)");
                return None;
            };
            let enc = f.encode(v);
            if enc.is_none() {
                log!("lp2mods.ini [rules]: {k} = {v:?} does not fit type {}", f.ty);
            }
            Some((f, enc?))
        })
        .collect()
}

/// What the match monitor logs when it changes (timers excluded: they tick every frame).
const WATCH: [&str; 6] = [
    "GameNetwork.mIsHost",
    "GameNetwork.mSessionMaxJoin",
    "NetRule.mActiveMemberNum",
    "NetRule.isGameFinished",
    "InfoGame.mNetWork.gameTimerMax",
    "GameNetwork.mNetworkError.no",
];

fn is_host(exe: Exe) -> bool {
    field("GameNetwork.mIsHost").and_then(|f| read(exe, f)).is_some_and(|b| b[0] != 0)
}

fn dump(exe: Exe) {
    let mut out = String::from("# Live multiplayer rule fields. Copy lines into the [rules] section of lp2mods.ini.\n");
    for f in FIELDS.iter() {
        if let Some(b) = read(exe, f) {
            out += &format!("{} = {}\n", f.name, f.decode(&b));
        }
    }
    match std::fs::write(exe_dir().join("lp2rules_dump.txt"), out) {
        Ok(()) => log!("wrote lp2rules_dump.txt"),
        Err(e) => log!("lp2rules_dump.txt: {e}"),
    }
}

/// Main thread, every frame.
pub fn on_frame(frame: u32) {
    static LAST: Mutex<BTreeMap<&'static str, String>> = Mutex::new(BTreeMap::new());
    static DUMPED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let Some(exe) = *EXE else { return };
    if frame % 30 != 0 {
        return;
    }
    if is_host(exe) {
        for (f, v) in &crate::mods::MODS.rules {
            write(exe, f, v);
        }
    }
    let now: BTreeMap<&'static str, String> =
        WATCH.iter().filter_map(|n| Some((*n, field(n).and_then(|f| Some(f.decode(&read(exe, f)?)))?))).collect();
    let mut last = LAST.lock().unwrap();
    if *last != now {
        let timer = field("NetRule.missionTimer").and_then(|f| Some(f.decode(&read(exe, f)?)));
        log!("match: {now:?} missionTimer={timer:?}");
        // Once a match is running, write the full field list for modders.
        let members = now.get("NetRule.mActiveMemberNum").and_then(|v| v.parse::<i32>().ok()).unwrap_or(0);
        if members > 0 && !DUMPED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            dump(exe);
        }
        *last = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_loads() {
        let f = field("InfoGame.mNetWork.gameTimerMax").unwrap();
        assert_eq!((f.owner, f.offset, f.ty), (Owner::InfoGame, 0x358, 12));
        assert_eq!(field("GameNetwork.mSessionMaxJoin").unwrap().offset, 0x3C);
        assert!(FIELDS.len() >= 90);
    }

    #[test]
    fn values_encode_by_type() {
        let f = |ty| Field { name: "x", owner: Owner::NetRule, offset: 0, ty };
        assert_eq!(f(12).encode("600"), Some(600f32.to_le_bytes().to_vec()));
        assert_eq!(f(10).encode("-3"), Some((-3i32).to_le_bytes().to_vec()));
        assert_eq!(f(3).encode("true"), Some(vec![1]));
        assert_eq!(f(4).encode("300"), None);
        assert_eq!(f(10).encode("1.5"), None);
        assert_eq!(f(12).decode(&f(12).encode("2.5").unwrap()), "2.5");
        assert_eq!(f(9).decode(&f(9).encode("-7").unwrap()), "-7");
    }
}

//! The one local player: always signed in to "Live" as the gamertag from lp2net.ini.

use crate::types::*;
use crate::{log, new_handle, CONFIG, ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS};
use lp2_proto::{Property, MAX_PROP_BYTES};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
use windows_sys::Win32::System::Threading::{ResetEvent, SetEvent};

pub const SIGNED_IN_TO_LIVE: u32 = 2;
pub const XN_SYS_SIGNINCHANGED: u32 = 0x0000_000A;
pub const XN_LIVE_CONNECTIONCHANGED: u32 = 0x0200_0001;
const XONLINE_S_LOGON_CONNECTION_ESTABLISHED: u32 = 0x0015_10F0;
const XUSER_DATA_TYPE_BINARY: u8 = 6;
const XUSER_DATA_TYPE_UNICODE: u8 = 4;
const XSOURCE_NO_VALUE: u32 = 0;
const XSOURCE_DEFAULT: u32 = 1;
const XSOURCE_TITLE: u32 = 2;
const PROFILE_SETTING_MAX: usize = 1000;

pub fn xuid() -> u64 {
    // FNV-1a of the gamertag, in the online XUID range.
    let h = CONFIG.name.bytes().fold(0x811C_9DC5u32, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193));
    0x0009_0000_0000_0000 | h as u64
}

pub fn signin_info() -> XUserSigninInfo {
    let mut name = [0u8; 16];
    name[..CONFIG.name.len()].copy_from_slice(CONFIG.name.as_bytes());
    XUserSigninInfo { xuid: xuid().to_le_bytes(), flags: 1, state: SIGNED_IN_TO_LIVE, name, ..Default::default() }
}

// ---- contexts and properties (what the host advertises with its session) ----

static CONTEXTS: LazyLock<Mutex<HashMap<u32, u32>>> = LazyLock::new(Mutex::default);
static PROPERTIES: LazyLock<Mutex<HashMap<u32, Property>>> = LazyLock::new(Mutex::default);

pub fn set_context(id: u32, value: u32) {
    CONTEXTS.lock().unwrap().insert(id, value);
}

pub fn context(id: u32) -> u32 {
    CONTEXTS.lock().unwrap().get(&id).copied().unwrap_or(0)
}

pub unsafe fn set_property(id: u32, size: u32, value: *const u8) {
    if value.is_null() || size as usize > MAX_PROP_BYTES {
        log!("XUserSetProperty {id:#x}: ignored ({size} bytes)");
        return;
    }
    let data = std::slice::from_raw_parts(value, size as usize).to_vec();
    PROPERTIES.lock().unwrap().insert(id, Property { id, kind: (id >> 28) as u8 & 0xF, data });
}

pub fn snapshot() -> (Vec<(u32, u32)>, Vec<Property>) {
    let mut c: Vec<_> = CONTEXTS.lock().unwrap().iter().map(|(k, v)| (*k, *v)).collect();
    let mut p: Vec<_> = PROPERTIES.lock().unwrap().values().cloned().collect();
    c.sort();
    p.sort_by_key(|p| p.id);
    (c, p)
}

/// Writes an XUSER_PROPERTY whose string/blob lives later in `out`.
pub fn put_property(out: &mut Out, array: usize, i: usize, p: &Property) {
    let mut d = XUserData { kind: p.kind, ..Default::default() };
    if p.kind == XUSER_DATA_TYPE_UNICODE || p.kind == XUSER_DATA_TYPE_BINARY {
        let ptr = out.bytes(&p.data, 2) as u32;
        d.value[..4].copy_from_slice(&(p.data.len() as u32).to_le_bytes());
        d.value[4..].copy_from_slice(&ptr.to_le_bytes());
    } else {
        let n = p.data.len().min(8);
        d.value[..n].copy_from_slice(&p.data[..n]);
    }
    unsafe { set(array, i, XUserProperty { id: p.id, _pad: 0, data: d }) };
}

// ---- profile settings ----
// Same files as the xlive-dummy this replaces, so saves carry over both ways.

fn title_slot(id: u32) -> Option<u32> {
    match id {
        0x63E8_3FFF => Some(1),
        0x63E8_3FFE => Some(2),
        0x63E8_3FFD => Some(3),
        _ => None,
    }
}

fn profile_path(slot: u32) -> PathBuf {
    crate::instance_file("ProflieSetting", "").join(format!("title_specific{slot}.bin"))
}

fn read_slot(slot: u32) -> Option<Vec<u8>> {
    let mut d = std::fs::read(profile_path(slot)).ok()?;
    d.truncate(PROFILE_SETTING_MAX);
    Some(d)
}

fn write_slot(slot: u32, data: &[u8]) -> std::io::Result<()> {
    let path = profile_path(slot);
    std::fs::create_dir_all(path.parent().unwrap())?;
    // Write then rename so a crash mid-write cannot leave a truncated save.
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(tmp, path)
}

/// One requested setting: `(setting id, user index or XUID bytes, is the local user)`.
pub type SettingReq = (u32, [u8; 8], bool);

/// XUSER_READ_PROFILE_SETTING_RESULT for `reqs`.
fn profile_results(out: &mut Out, reqs: &[SettingReq]) {
    let header = out.put(CountPtr::default());
    let arr = out.array::<XUserProfileSetting>(reqs.len());
    for (i, &(id, user, mine)) in reqs.iter().enumerate() {
        let kind = (id >> 28) as u8;
        let mut s = XUserProfileSetting { source: XSOURCE_DEFAULT, user, id, ..Default::default() };
        s.data.kind = kind;
        if let Some(slot) = title_slot(id) {
            s.source = XSOURCE_NO_VALUE;
            if let Some(blob) = mine.then(|| read_slot(slot)).flatten() {
                s.source = XSOURCE_TITLE;
                let ptr = out.bytes(&blob, 4) as u32;
                s.data.value[..4].copy_from_slice(&(blob.len() as u32).to_le_bytes());
                s.data.value[4..].copy_from_slice(&ptr.to_le_bytes());
            }
        }
        unsafe { set(arr, i, s) };
    }
    unsafe { set(header, 0, CountPtr { count: reqs.len() as u32, ptr: arr as u32 }) };
}

pub unsafe fn read_profile(reqs: &[SettingReq], size: *mut u32, buf: *mut u8) -> u32 {
    let mut m = Out::measure();
    profile_results(&mut m, reqs);
    if buf.is_null() || *size < m.len as u32 {
        *size = m.len as u32;
        return ERROR_INSUFFICIENT_BUFFER;
    }
    profile_results(&mut Out::new(buf), reqs);
    ERROR_SUCCESS
}

pub unsafe fn write_profile(settings: &[XUserProfileSetting]) {
    for s in settings {
        let Some(slot) = title_slot(s.id) else { continue };
        if s.data.kind != XUSER_DATA_TYPE_BINARY {
            continue;
        }
        let len = (u32::from_le_bytes(s.data.value[..4].try_into().unwrap()) as usize).min(PROFILE_SETTING_MAX);
        let ptr = u32::from_le_bytes(s.data.value[4..].try_into().unwrap()) as *const u8;
        if ptr.is_null() {
            continue;
        }
        if let Err(e) = write_slot(slot, std::slice::from_raw_parts(ptr, len)) {
            log!("saving profile slot {slot} failed: {e}");
        }
    }
}

// ---- notifications ----

struct Listener {
    queue: VecDeque<(u32, u32)>,
}

static LISTENERS: LazyLock<Mutex<HashMap<usize, Listener>>> = LazyLock::new(Mutex::default);

pub fn create_listener(areas: u64) -> usize {
    let h = new_handle();
    let mut queue = VecDeque::new();
    if areas & 1 != 0 {
        queue.push_back((XN_SYS_SIGNINCHANGED, 1));
    }
    if areas & 2 != 0 {
        queue.push_back((XN_LIVE_CONNECTIONCHANGED, XONLINE_S_LOGON_CONNECTION_ESTABLISHED));
    }
    if !queue.is_empty() {
        unsafe { SetEvent(h as _) };
    }
    LISTENERS.lock().unwrap().insert(h, Listener { queue });
    h
}

pub fn next_notification(h: usize, filter: u32) -> Option<(u32, u32)> {
    let mut ls = LISTENERS.lock().unwrap();
    let l = ls.get_mut(&h)?;
    let i = l.queue.iter().position(|(id, _)| filter == 0 || *id == filter)?;
    let n = l.queue.remove(i);
    if l.queue.is_empty() {
        unsafe { ResetEvent(h as _) };
    }
    n
}

pub fn close_listener(h: usize) {
    LISTENERS.lock().unwrap().remove(&h);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notifications_drain() {
        let h = create_listener(3);
        assert_eq!(next_notification(h, XN_LIVE_CONNECTIONCHANGED).map(|n| n.0), Some(XN_LIVE_CONNECTIONCHANGED));
        assert_eq!(next_notification(h, 0).map(|n| n.0), Some(XN_SYS_SIGNINCHANGED));
        assert_eq!(next_notification(h, 0), None);
    }

    #[test]
    fn profile_buffer_roundtrip() {
        let reqs = [(0x63E8_3FFD, [0; 8], false), (0x1004_0003, [0; 8], false)];
        let mut size = 0;
        assert_eq!(unsafe { read_profile(&reqs, &mut size, std::ptr::null_mut()) }, ERROR_INSUFFICIENT_BUFFER);
        let mut buf = vec![0u64; size as usize / 8 + 1];
        assert_eq!(unsafe { read_profile(&reqs, &mut size, buf.as_mut_ptr() as _) }, ERROR_SUCCESS);
        let h = unsafe { *(buf.as_ptr() as *const CountPtr) };
        assert_eq!(h.count, 2);
        let s = unsafe { *(h.ptr as *const XUserProfileSetting) };
        assert_eq!((s.id, s.source, s.data.kind), (0x63E8_3FFD, XSOURCE_NO_VALUE, 6));
    }
}

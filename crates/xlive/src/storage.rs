//! XStorage (Live cloud storage) backed by `storage\` next to the game.
//! LP2 opens it before matchmaking (`NetworkStorageLive::open`) and drops online play if it fails.
//!
//! Server paths are what we hand the game from XStorageBuildServerPath and get back later:
//! `title/<item>` (shared title files) or `user/<xuid>/<item>` (per-player files).

use crate::{exe_dir, log, ERROR_INSUFFICIENT_BUFFER, ERROR_INVALID_PARAMETER, ERROR_SUCCESS};
use std::path::PathBuf;

pub const XSTORAGE_FACILITY_GAME_CLIP: u32 = 1;
pub const XSTORAGE_FACILITY_PER_TITLE: u32 = 2;
pub const XSTORAGE_FACILITY_PER_USER_TITLE: u32 = 3;
pub const E_FILE_NOT_FOUND: u32 = 0x8015_C004;
pub const E_INVALID_STORAGE_PATH: u32 = 0x8015_C008;
pub const E_INVALID_FACILITY: u32 = 0x8015_C009;
const MAX_FILE: usize = 100_000_000;

/// Keeps a name usable as one path component (no separators, no `..`).
fn sanitize(item: &str) -> String {
    let s: String = item.chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '_' }).collect();
    if s.is_empty() || s.chars().all(|c| c == '.') { "_".into() } else { s }
}

pub fn server_path(facility: u32, xuid: u64, item: &str) -> Result<String, u32> {
    match facility {
        XSTORAGE_FACILITY_PER_TITLE => Ok(format!("title/{}", sanitize(item))),
        XSTORAGE_FACILITY_PER_USER_TITLE | XSTORAGE_FACILITY_GAME_CLIP => Ok(format!("user/{xuid:016x}/{}", sanitize(item))),
        _ => Err(E_INVALID_FACILITY),
    }
}

/// Maps a server path back to a file, rejecting anything we did not build.
pub fn file_for(path: &str) -> Result<PathBuf, u32> {
    let parts: Vec<&str> = path.split('/').collect();
    let ok = match parts.as_slice() {
        ["title", item] => sanitize(item) == *item,
        ["user", xuid, item] => xuid.len() == 16 && xuid.chars().all(|c| c.is_ascii_hexdigit()) && sanitize(item) == *item,
        _ => false,
    };
    if !ok {
        log!("XStorage: rejected path {path:?}");
        return Err(E_INVALID_STORAGE_PATH);
    }
    Ok(parts.iter().fold(exe_dir().join("storage"), |p, c| p.join(c)))
}

pub fn read(path: &str) -> Result<Vec<u8>, u32> {
    let f = file_for(path)?;
    match std::fs::read(&f) {
        Ok(d) => Ok(d),
        Err(_) => {
            log!("XStorage download {path}: not found");
            Err(E_FILE_NOT_FOUND)
        }
    }
}

pub fn write(path: &str, data: &[u8]) -> Result<(), u32> {
    if data.len() > MAX_FILE {
        return Err(ERROR_INVALID_PARAMETER);
    }
    let f = file_for(path)?;
    let tmp = f.with_extension("tmp");
    // Write then rename, so a crash cannot leave a half-written save.
    let r = std::fs::create_dir_all(f.parent().unwrap()).and_then(|_| std::fs::write(&tmp, data)).and_then(|_| std::fs::rename(&tmp, &f));
    log!("XStorage upload {path}: {} bytes {}", data.len(), if r.is_ok() { "saved" } else { "FAILED" });
    r.map_err(|_| E_INVALID_STORAGE_PATH)
}

pub fn delete(path: &str) -> Result<(), u32> {
    let f = file_for(path)?;
    std::fs::remove_file(f).map_err(|_| E_FILE_NOT_FOUND)
}

pub unsafe fn wide(p: *const u16) -> Option<String> {
    if p.is_null() {
        return None;
    }
    let mut n = 0;
    while *p.add(n) != 0 && n < 1024 {
        n += 1;
    }
    Some(String::from_utf16_lossy(std::slice::from_raw_parts(p, n)))
}

/// Writes `s` as a NUL-terminated wide string; `len` is the buffer size in characters on input
/// and the written length including the NUL on output.
pub unsafe fn write_wide(s: &str, out: *mut u16, len: *mut u32) -> u32 {
    let w: Vec<u16> = s.encode_utf16().chain([0]).collect();
    if out.is_null() || len.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    if (*len as usize) < w.len() {
        *len = w.len() as u32;
        return ERROR_INSUFFICIENT_BUFFER;
    }
    std::ptr::copy_nonoverlapping(w.as_ptr(), out, w.len());
    *len = w.len() as u32;
    ERROR_SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_cannot_escape() {
        assert_eq!(server_path(2, 1, "../../evil").unwrap(), "title/.._.._evil");
        assert_eq!(server_path(3, 0xAB, "save.bin").unwrap(), "user/00000000000000ab/save.bin");
        assert!(file_for("title/..").is_err());
        assert!(file_for("user/x/save").is_err());
        assert!(file_for("../x").is_err());
        assert!(file_for("title/ok.bin").is_ok());
        assert_eq!(server_path(9, 0, "a"), Err(E_INVALID_FACILITY));
    }

    #[test]
    fn wide_buffer_sizes() {
        let mut buf = [0u16; 4];
        let mut len = 2;
        assert_eq!(unsafe { write_wide("abc", buf.as_mut_ptr(), &mut len) }, ERROR_INSUFFICIENT_BUFFER);
        assert_eq!(len, 4);
        assert_eq!(unsafe { write_wide("abc", buf.as_mut_ptr(), &mut len) }, ERROR_SUCCESS);
        assert_eq!(unsafe { wide(buf.as_ptr()) }.as_deref(), Some("abc"));
    }
}

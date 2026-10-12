//! Drop-in `xlive.dll` (Games for Windows - LIVE) for Lost Planet 2.
//!
//! The game keeps its own netcode; this DLL replaces what GFWL gave it:
//! a signed-in profile, XNet sockets that reach other players (direct or through the master's
//! relay), and session publish/search through `lp2-master`.
//!
//! Struct layouts here are the 32-bit ones the game uses, so the crate is empty on other targets.
#![cfg(target_arch = "x86")]
#![allow(non_snake_case, clippy::missing_safety_doc, clippy::too_many_arguments)]

pub mod exports;
pub mod botmaps;
pub mod bots;
pub mod gamelog;
pub mod hook;
pub mod mods;
pub mod net;
pub mod patch;
pub mod rules;
pub mod session;
pub mod startup;
pub mod stats;
pub mod storage;
pub mod types;
pub mod unlocks;
pub mod user;
pub mod watch;
pub mod xhv;

use std::fs::File;
use std::io::Write;
use std::net::{SocketAddr, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::Threading::{CreateEventW, SetEvent};

pub struct Config {
    pub name: String,
    pub master: Option<SocketAddr>,
    /// This PC accepts packets from anyone on `port` (forwarded or no NAT). Otherwise peers reach us via the master.
    pub direct: bool,
    pub port: u16,
    pub log: bool,
    /// Debug: send everything through the master's relay, even to peers on this PC or LAN.
    pub relay: bool,
    /// The server directory (used when `master` is not set).
    pub directory: String,
}

impl Config {
    pub fn parse(text: &str) -> Config {
        let mut c = Config { name: "Player".into(), master: None, direct: false, port: 0, log: true, relay: false, directory: lp2_proto::DIRECTORY_URL.into() };
        for line in text.lines() {
            let line = line.split(['#', ';']).next().unwrap_or("");
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim();
            match k.trim().to_ascii_lowercase().as_str() {
                // XUSER_NAME_SIZE is 16 including the NUL; gamertags are ASCII.
                "name" if !v.is_empty() => c.name = v.chars().filter(char::is_ascii).take(15).collect(),
                "master" if !v.is_empty() => c.master = resolve(v),
                "direct" => c.direct = v.eq_ignore_ascii_case("true") || v == "1",
                "port" => c.port = v.parse().unwrap_or(0),
                "log" => c.log = v.eq_ignore_ascii_case("true") || v == "1",
                "relay" => c.relay = v.eq_ignore_ascii_case("true") || v == "1",
                "directory" if !v.is_empty() => c.directory = v.trim_end_matches('/').into(),
                _ => {}
            }
        }
        c
    }
}

fn resolve(host: &str) -> Option<SocketAddr> {
    let with_port = if host.contains(':') { host.to_string() } else { format!("{host}:{}", lp2_proto::DEFAULT_MASTER_PORT) };
    // The wire format carries IPv4 only.
    with_port.to_socket_addrs().ok()?.find(SocketAddr::is_ipv4)
}

pub fn exe_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(Into::into)).unwrap_or_else(|| ".".into())
}

/// Which copy of the game this is on this PC (0 = first). Copies after the first play as
/// "<name> <n>" with their own player id, profile folder and log, so two can share a lobby.
pub static INSTANCE: LazyLock<u32> = LazyLock::new(|| {
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows_sys::Win32::System::Threading::CreateMutexW;
    for i in 0..8u32 {
        let name: Vec<u16> = format!(r"Local\LP2NET_INSTANCE_{i}").encode_utf16().chain([0]).collect();
        // The handle is kept open for the life of the process: it is what marks the slot taken.
        let h = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
        if !h.is_null() && unsafe { GetLastError() } != ERROR_ALREADY_EXISTS {
            return i;
        }
        unsafe { CloseHandle(h) };
    }
    0
});

/// `name` for the first copy, `name2` for the second...: per-instance file names.
pub fn instance_file(stem: &str, ext: &str) -> PathBuf {
    match *INSTANCE {
        0 => exe_dir().join(format!("{stem}{ext}")),
        n => exe_dir().join(format!("{stem}{}{ext}", n + 1)),
    }
}

pub static CONFIG: LazyLock<Config> = LazyLock::new(|| {
    let mut c = Config::parse(&std::fs::read_to_string(exe_dir().join("lp2net.ini")).unwrap_or_default());
    if *INSTANCE > 0 {
        let suffix = format!(" {}", *INSTANCE + 1);
        c.name = format!("{}{suffix}", &c.name[..c.name.len().min(15 - suffix.len())]);
        // A fixed port can only be used by one copy.
        c.port = 0;
    }
    c
});

static LOG: LazyLock<Mutex<Option<File>>> = LazyLock::new(|| {
    let path = instance_file("lp2net", ".log");
    // Keep the previous run: the interesting one is often the run before the current launch.
    let _ = std::fs::rename(&path, instance_file("lp2net", ".prev.log"));
    Mutex::new(CONFIG.log.then(|| File::create(path).ok()).flatten())
});

pub fn log_args(args: std::fmt::Arguments) {
    if let Some(f) = LOG.lock().unwrap().as_mut() {
        let _ = writeln!(f, "{args}");
    }
}

/// Logs the first 30 calls of each export, to see what the game asks for.
pub fn trace(name: &'static str) {
    static COUNTS: LazyLock<Mutex<std::collections::HashMap<&'static str, u32>>> = LazyLock::new(Mutex::default);
    let n = {
        let mut c = COUNTS.lock().unwrap();
        let n = c.entry(name).or_insert(0);
        *n += 1;
        *n
    };
    if n <= 30 {
        log_args(format_args!("call {name}{}", if n == 30 { " (further calls not logged)" } else { "" }));
    }
}

#[macro_export]
macro_rules! log {
    ($($t:tt)*) => { $crate::log_args(format_args!($($t)*)) };
}

/// Non-cryptographic randomness for ids and keys (no game secret depends on it).
pub fn random_u64() -> u64 {
    use std::hash::{BuildHasher, RandomState};
    RandomState::new().hash_one((std::time::SystemTime::now(), std::process::id()))
}

/// A real, closable kernel handle used as the identity of sessions, listeners and enumerators.
pub fn new_handle() -> usize {
    unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) as usize }
}

pub const ERROR_SUCCESS: u32 = 0;
pub const ERROR_IO_PENDING: u32 = 997;
pub const ERROR_IO_INCOMPLETE: u32 = 996;
pub const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
pub const ERROR_INVALID_PARAMETER: u32 = 87;
pub const ERROR_NO_MORE_FILES: u32 = 18;
pub const ERROR_FUNCTION_FAILED: u32 = 1627;
pub const ERROR_NOT_FOUND: u32 = 1168;
pub const ERROR_NO_SUCH_USER: u32 = 1317;
pub const ERROR_NOT_LOGGED_ON: u32 = 1245;

#[repr(C)]
pub struct XOverlapped {
    pub internal_low: u32,
    pub internal_high: u32,
    pub internal_context: u32,
    pub event: HANDLE,
    pub completion_routine: usize,
    pub completion_context: usize,
    pub extended_error: u32,
}

/// Finishes an XLive call. Everything here completes immediately, so with an XOVERLAPPED
/// the result goes into it and the caller gets ERROR_IO_PENDING, as real GFWL does.
pub unsafe fn complete(ov: *mut XOverlapped, result: u32, info: u32) -> u32 {
    let Some(ov) = ov.as_mut() else { return result };
    if ov.completion_routine != 0 {
        log!("XOVERLAPPED completion routines are not supported; using the event only");
    }
    ov.extended_error = if result == 0 { 0 } else { 0x8007_0000 | (result & 0xFFFF) };
    ov.internal_high = info;
    ov.internal_low = result;
    if !ov.event.is_null() {
        SetEvent(ov.event);
    }
    ERROR_IO_PENDING
}

/// Like `complete`, for failures that carry their own extended error (e.g. XONLINE_E_STORAGE_*):
/// the overlapped result is ERROR_FUNCTION_FAILED and `extended` goes to dwExtendedError.
pub unsafe fn fail(ov: *mut XOverlapped, extended: u32) -> u32 {
    let Some(o) = ov.as_mut() else { return extended };
    complete(o, ERROR_FUNCTION_FAILED, 0);
    o.extended_error = extended;
    ERROR_IO_PENDING
}

/// Runs when the game loads this DLL, before the game's own startup code: the place for
/// patches that must be in before WinMain (the single-instance check).
#[no_mangle]
pub extern "system" fn DllMain(_module: HANDLE, reason: u32, _reserved: *mut std::ffi::c_void) -> i32 {
    const DLL_PROCESS_ATTACH: u32 = 1;
    if reason == DLL_PROCESS_ATTACH {
        startup::early();
    }
    1
}

/// Background work: keep our master registration alive and re-publish hosted sessions.
pub fn start_heartbeat() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        std::thread::spawn(|| {
            for tick in 0u64.. {
                // No server was online at startup: look again every minute.
                if tick % 12 == 11 && CONFIG.master.is_none() && net::NET.master().is_none() {
                    net::find_master();
                }
                net::NET.hello();
                session::refresh();
                std::thread::sleep(std::time::Duration::from_secs(5));
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_parses() {
        let c = Config::parse("name = A very long gamertag name\nmaster = 127.0.0.1\ndirect=true # comment\nport=3074\n");
        assert_eq!(c.name, "A very long gam");
        assert_eq!(c.master, Some("127.0.0.1:27900".parse().unwrap()));
        assert!(c.direct);
        assert_eq!(c.port, 3074);
    }

    #[test]
    fn complete_fills_overlapped() {
        let mut ov: XOverlapped = unsafe { std::mem::zeroed() };
        assert_eq!(unsafe { complete(&mut ov, 0, 5) }, ERROR_IO_PENDING);
        assert_eq!((ov.internal_low, ov.internal_high), (0, 5));
        assert_eq!(unsafe { complete(std::ptr::null_mut(), 18, 0) }, 18);
    }
}

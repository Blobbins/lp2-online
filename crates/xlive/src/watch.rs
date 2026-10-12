//! Debug: hardware write watchpoints. Finds the code that writes a field when nothing in a
//! disassembly points at it (the address only exists at run time).
//!
//! `[watch]` in lp2mods.ini:
//! ```text
//! at  = 0x941442:6      ; hook this instruction (6 bytes of whole instructions)
//! esi = 4,8,10,a0       ; when it runs, watch esi+these (hex, 4-byte aligned, up to 4)
//! ```
//! Every write to a watched dword is logged with the address of the instruction after it.

use crate::hook::Regs;
use crate::log;
use crate::mods::MODS;
use std::sync::atomic::{AtomicU32, Ordering};
use windows_sys::Win32::Foundation::{CloseHandle, EXCEPTION_SINGLE_STEP};
use windows_sys::Win32::System::Diagnostics::Debug::{
    AddVectoredExceptionHandler, GetThreadContext, SetThreadContext, CONTEXT, CONTEXT_DEBUG_REGISTERS_X86, EXCEPTION_POINTERS,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcessId, GetCurrentThreadId, OpenThread, ResumeThread, SuspendThread, THREAD_GET_CONTEXT, THREAD_SET_CONTEXT,
    THREAD_SUSPEND_RESUME,
};

static WATCHED: [AtomicU32; 4] = [const { AtomicU32::new(0) }; 4];
static HITS: AtomicU32 = AtomicU32::new(0);

unsafe extern "system" fn on_exception(info: *mut EXCEPTION_POINTERS) -> i32 {
    let (rec, ctx) = ((*info).ExceptionRecord, (*info).ContextRecord);
    if (*rec).ExceptionCode != EXCEPTION_SINGLE_STEP || (*ctx).Dr6 & 0xF == 0 {
        return 0; // EXCEPTION_CONTINUE_SEARCH
    }
    for i in 0..4 {
        if (*ctx).Dr6 & (1 << i) != 0 && HITS.fetch_add(1, Ordering::Relaxed) < 300 {
            let addr = WATCHED[i].load(Ordering::Relaxed);
            let value = std::ptr::read_volatile(addr as *const u32);
            log!("watch {addr:#x} = {value:#x} written before eip {:#x}", (*rec).ExceptionAddress as usize);
        }
    }
    (*ctx).Dr6 = 0;
    -1 // EXCEPTION_CONTINUE_EXECUTION
}

/// Arms write watchpoints on `addrs` in every thread but the caller's helper.
fn arm(addrs: Vec<u32>) {
    for (slot, a) in WATCHED.iter().zip(addrs.iter().chain(std::iter::repeat(&0))) {
        slot.store(*a, Ordering::Relaxed);
    }
    // A thread cannot set its own debug registers while running, so a helper does it for all.
    std::thread::spawn(move || unsafe {
        let me = GetCurrentThreadId();
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        let mut te: THREADENTRY32 = std::mem::zeroed();
        te.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
        let mut ok = Thread32First(snap, &mut te) != 0;
        while ok {
            if te.th32OwnerProcessID == GetCurrentProcessId() && te.th32ThreadID != me {
                let t = OpenThread(THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT | THREAD_SET_CONTEXT, 0, te.th32ThreadID);
                if !t.is_null() {
                    SuspendThread(t);
                    let mut c: CONTEXT = std::mem::zeroed();
                    c.ContextFlags = CONTEXT_DEBUG_REGISTERS_X86;
                    if GetThreadContext(t, &mut c) != 0 {
                        let mut dr7 = 0u32;
                        for (i, a) in addrs.iter().take(4).enumerate() {
                            match i {
                                0 => c.Dr0 = *a,
                                1 => c.Dr1 = *a,
                                2 => c.Dr2 = *a,
                                _ => c.Dr3 = *a,
                            }
                            // Local enable, break on write (01), length 4 (11).
                            dr7 |= (1 << (2 * i)) | (0b1101 << (16 + 4 * i));
                        }
                        c.Dr7 = dr7;
                        SetThreadContext(t, &c);
                    }
                    ResumeThread(t);
                    CloseHandle(t);
                }
            }
            ok = Thread32Next(snap, &mut te) != 0;
        }
        CloseHandle(snap);
    })
    .join()
    .ok();
}

static OFFSETS: std::sync::OnceLock<Vec<u32>> = std::sync::OnceLock::new();

extern "stdcall" fn on_hit(_addr: u32, r: &mut Regs) -> u32 {
    let addrs: Vec<u32> = OFFSETS.get().unwrap().iter().map(|o| r.esi + o).collect();
    log!("watch: arming esi={:#x} {:x?}", r.esi, addrs);
    arm(addrs);
    0
}

/// Installs the `[watch]` hook from lp2mods.ini, if any.
pub fn apply() {
    let Some(s) = MODS.ini.get("watch") else { return };
    let get = |k: &str| s.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
    let (Some(at), Some(esi)) = (get("at"), get("esi")) else { return };
    let Some((addr, len)) = at.split_once(':') else { return };
    let (Ok(addr), Ok(len)) = (u32::from_str_radix(addr.trim_start_matches("0x"), 16), len.parse::<usize>()) else { return };
    let offsets = esi.split(',').filter_map(|o| u32::from_str_radix(o.trim(), 16).ok()).take(4).collect();
    OFFSETS.set(offsets).ok();
    unsafe {
        AddVectoredExceptionHandler(1, Some(on_exception));
        crate::hook::install(addr, len, 0, on_hit);
    }
}

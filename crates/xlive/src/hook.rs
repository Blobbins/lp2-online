//! Function entry hooks: the game's function at `addr` gets a 5-byte `jmp` to a small stub that
//! saves the registers, calls a Rust handler with them, restores them, runs the overwritten
//! instructions and jumps back. The handler sees the call as it starts (registers + stack).
//!
//! `[trace]` in lp2mods.ini: `0x416650 = 6` logs every call to that function. The number is how
//! many bytes of whole instructions to move into the stub (at least 5, no relative jumps/calls in
//! them) - read them off a disassembly.

use crate::log;
use crate::mods::{read_mem, EXE, MODS};
use windows_sys::Win32::System::Diagnostics::Debug::FlushInstructionCache;
use windows_sys::Win32::System::Memory::{VirtualAlloc, VirtualProtect, MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

/// Registers as `pushfd` after `pushad` leaves them on the stack.
#[repr(C)]
pub struct Regs {
    pub eflags: u32,
    pub edi: u32,
    pub esi: u32,
    pub ebp: u32,
    /// Stack pointer at the hooked function's first instruction: [esp] is the return address.
    pub esp: u32,
    pub ebx: u32,
    pub edx: u32,
    pub ecx: u32,
    pub eax: u32,
}

impl Regs {
    /// Stack dword `i` (0 = return address, 1 = first stack argument...).
    pub fn stack(&self, i: usize) -> u32 {
        let mut b = [0u8; 4];
        read_mem(self.esp as usize + i * 4, &mut b);
        u32::from_le_bytes(b)
    }
}

/// Returns 0 to run the original function, or 1 to skip it: the hooked function then returns
/// at once with `regs.eax` (popping `ret_bytes` of stack arguments, as the original would).
pub type Handler = extern "stdcall" fn(addr: u32, regs: &mut Regs) -> u32;

/// Stub bytes for a hook at `addr` that calls `handler`, given the stub's own address.
pub fn stub(addr: u32, handler: u32, stolen: &[u8], ret_bytes: u16, at: u32) -> Vec<u8> {
    let mut c = vec![0x60, 0x9C, 0x54, 0x68]; // pushad; pushfd; push esp; push addr
    c.extend(addr.to_le_bytes());
    c.push(0xE8); // call handler (stdcall: pops both arguments)
    c.extend(handler.wrapping_sub(at + c.len() as u32 + 4).to_le_bytes());
    c.extend([0x85, 0xC0, 0x75]); // test eax,eax; jnz skip
    c.push((2 + stolen.len() + 5) as u8);
    c.extend([0x9D, 0x61]); // popfd; popad
    c.extend(stolen);
    c.push(0xE9); // jmp back past the stolen bytes
    c.extend((addr + stolen.len() as u32).wrapping_sub(at + c.len() as u32 + 4).to_le_bytes());
    c.extend([0x9D, 0x61, 0xC2]); // skip: popfd; popad; ret ret_bytes
    c.extend(ret_bytes.to_le_bytes());
    c
}

/// Installs `handler` at `addr`, moving `len` bytes of whole instructions into the stub.
/// `ret_bytes`: stack bytes the hooked function pops on return (used when the handler skips it).
pub unsafe fn install(addr: u32, len: usize, ret_bytes: u16, handler: Handler) -> bool {
    let mut stolen = vec![0u8; len];
    // E8/E9/EB/7x/0F 8x first bytes are relative branches that would break when moved.
    if len < 5 || !read_mem(addr as usize, &mut stolen) || matches!(stolen[0], 0xE8 | 0xE9 | 0xEB | 0x70..=0x7F) {
        log!("hook {addr:#x}: refused (bad length or starts with a branch)");
        return false;
    }
    let mem = VirtualAlloc(std::ptr::null(), 64, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) as *mut u8;
    if mem.is_null() {
        return false;
    }
    let code = stub(addr, handler as usize as u32, &stolen, ret_bytes, mem as u32);
    std::ptr::copy_nonoverlapping(code.as_ptr(), mem, code.len());
    let mut jmp = vec![0x90u8; len]; // jmp stub, rest nop
    jmp[0] = 0xE9;
    jmp[1..5].copy_from_slice(&(mem as u32).wrapping_sub(addr + 5).to_le_bytes());
    let mut old = 0;
    if VirtualProtect(addr as _, len, PAGE_EXECUTE_READWRITE, &mut old) == 0 {
        return false;
    }
    std::ptr::copy_nonoverlapping(jmp.as_ptr(), addr as *mut u8, len);
    VirtualProtect(addr as _, len, old, &mut old);
    FlushInstructionCache(GetCurrentProcess(), addr as _, len);
    log!("hook {addr:#x}: installed ({} bytes moved: {})", len, stolen.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" "));
    true
}

/// Logs a call: arguments, `this`-style registers and the code addresses found on the stack
/// (a rough backtrace: return addresses into the exe).
extern "stdcall" fn trace_call(addr: u32, r: &mut Regs) -> u32 {
    static COUNTS: std::sync::Mutex<Vec<(u32, u32)>> = std::sync::Mutex::new(Vec::new());
    let n = {
        let mut c = COUNTS.lock().unwrap();
        match c.iter_mut().find(|e| e.0 == addr) {
            Some(e) => {
                e.1 += 1;
                e.1
            }
            None => {
                c.push((addr, 1));
                1
            }
        }
    };
    if n > 2000 {
        return 0;
    }
    let args: Vec<String> = (1..=4).map(|i| arg_text(r.stack(i))).collect();
    let trail: Vec<String> = (0..64).map(|i| r.stack(i)).filter(|v| (0x40_1000..0x0130_0000).contains(v)).take(8).map(|v| format!("{v:#x}")).collect();
    log!(
        "trace {addr:#x} #{n}: args [{}] ecx={:#x} esi={:#x} edi={:#x} | stack code {}",
        args.join(", "),
        r.ecx,
        r.esi,
        r.edi,
        trail.join(" ")
    );
    0
}

/// An argument as hex, plus the text it points to when that is a short printable C string.
fn arg_text(v: u32) -> String {
    let mut buf = [0u8; 64];
    if v >= 0x10000 && read_mem(v as usize, &mut buf) {
        let len = buf.iter().position(|&b| b == 0).unwrap_or(0);
        if len >= 3 && buf[..len].iter().all(|b| b.is_ascii_graphic() || *b == b' ') {
            return format!("{v:#x} \"{}\"", String::from_utf8_lossy(&buf[..len]));
        }
    }
    format!("{v:#x}")
}

/// Installs the `[trace]` hooks from lp2mods.ini. Called once, early.
pub fn apply() {
    if EXE.is_none() {
        return;
    }
    for (k, v) in MODS.ini.get("trace").map_or(&[][..], |s| s) {
        let addr = u32::from_str_radix(k.trim_start_matches("0x"), 16);
        match (addr, v.parse::<usize>()) {
            (Ok(a), Ok(len)) => unsafe {
                install(a, len, 0, trace_call);
            },
            _ => log!("trace: bad entry '{k} = {v}' (want 0xADDR = bytes)"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_layout() {
        let s = stub(0x0041_6650, 0x1000_0000, &[0x8B, 0x86, 0x80, 0, 0, 0], 4, 0x2000_0000);
        assert_eq!(&s[..4], &[0x60, 0x9C, 0x54, 0x68]);
        assert_eq!(&s[4..8], &0x0041_6650u32.to_le_bytes());
        // call rel32 ends at stub+13
        assert_eq!(u32::from_le_bytes(s[9..13].try_into().unwrap()), 0x1000_0000u32.wrapping_sub(0x2000_000D));
        assert_eq!(&s[13..17], &[0x85, 0xC0, 0x75, 13]); // jnz over popfd/popad, stolen, jmp
        assert_eq!(&s[17..19], &[0x9D, 0x61]);
        assert_eq!(&s[19..25], &[0x8B, 0x86, 0x80, 0, 0, 0]);
        assert_eq!(s[25], 0xE9);
        let back = 0x2000_0000u32 + 30;
        assert_eq!(u32::from_le_bytes(s[26..30].try_into().unwrap()), 0x0041_6656u32.wrapping_sub(back));
        assert_eq!(&s[30..], &[0x9D, 0x61, 0xC2, 4, 0]); // skip target: 17 + 13 = 30
    }
}

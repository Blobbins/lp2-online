//! LP2's own debug messages ("%s: XUserReadStats(3) failed Return=%d ...") go to a logger object
//! whose print methods are empty in the retail build. This swaps that object's vtable for a copy
//! whose print slots format the message into lp2net.log.

use crate::mods::{read_mem, write_mem, Exe, EXE};
use crate::log;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::Mutex;

const SLOTS: usize = 64;
/// vtable slots used for printing: +0x20 (this, level, fmt, ...), +0x24 and +0x28 (this, fmt, ...).
const PRINT_LEVEL: usize = 8;
const PRINT_A: usize = 9;
const PRINT_B: usize = 10;
const MAX_LINES: u32 = 20_000;

fn logger_global(exe: Exe) -> usize {
    match exe {
        Exe::Dx9 => 0x018C_E2D8,
        Exe::Dx11 => 0x018D_E398,
    }
}

static OUR_VTABLE: AtomicUsize = AtomicUsize::new(0);
static LINES: AtomicU32 = AtomicU32::new(0);
static LAST: Mutex<(String, u32)> = Mutex::new((String::new(), 0));

fn read_u32(addr: usize) -> Option<u32> {
    let mut b = [0u8; 4];
    read_mem(addr, &mut b).then(|| u32::from_le_bytes(b))
}

/// Reads a NUL-terminated string from game memory without trusting the pointer.
fn read_cstr(addr: usize, wide: bool) -> String {
    let mut out = Vec::new();
    let unit = if wide { 2 } else { 1 };
    let mut chunk = [0u8; 64];
    let mut a = addr;
    while out.len() < 512 && read_mem(a, &mut chunk) {
        for c in chunk.chunks(unit) {
            let ch = if wide { u16::from_le_bytes([c[0], c[1]]) as u32 } else { c[0] as u32 };
            if ch == 0 {
                return String::from_utf8_lossy(&out).into_owned();
            }
            out.push(if ch < 0x80 { ch as u8 } else { b'?' });
        }
        a += chunk.len();
    }
    if out.is_empty() { format!("<bad ptr {addr:#x}>") } else { String::from_utf8_lossy(&out).into_owned() }
}

/// printf over 32-bit stack slots (enough for the game's %s %d %u %x %X %08X %f %c %p %S).
pub fn format(fmt: &str, args: &[u32]) -> String {
    let mut out = String::new();
    let mut next = 0;
    let mut arg = || {
        let v = args.get(next).copied().unwrap_or(0);
        next += 1;
        v
    };
    let mut chars = fmt.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let mut spec = String::new();
        while let Some(&f) = chars.peek() {
            if f.is_ascii_digit() || "-+ #.lhI6".contains(f) {
                spec.push(f);
                chars.next();
            } else {
                break;
            }
        }
        let zero = spec.starts_with('0');
        let width: usize = spec.trim_start_matches(['-', '+', ' ', '#', '0']).split(['.', 'l', 'h', 'I']).next().and_then(|w| w.parse().ok()).unwrap_or(0);
        let wide64 = spec.contains("ll") || spec.contains("I64");
        let pad = |s: String| if zero { format!("{s:0>width$}") } else { format!("{s:>width$}") };
        match chars.next() {
            Some('%') => out.push('%'),
            Some('d' | 'i') if wide64 => out += &pad((arg() as u64 | (arg() as u64) << 32).to_string()),
            Some('d' | 'i') => out += &pad((arg() as i32).to_string()),
            Some('u') => out += &pad(arg().to_string()),
            Some('x') if wide64 => out += &pad(format!("{:x}", arg() as u64 | (arg() as u64) << 32)),
            Some('X') if wide64 => out += &pad(format!("{:X}", arg() as u64 | (arg() as u64) << 32)),
            Some('x') => out += &pad(format!("{:x}", arg())),
            Some('X') => out += &pad(format!("{:X}", arg())),
            Some('p') => out += &format!("{:08X}", arg()),
            Some('c') => out.push(char::from_u32(arg() & 0xFF).unwrap_or('?')),
            Some('s') => out += &read_cstr(arg() as usize, spec.contains('l')),
            Some('S') => out += &read_cstr(arg() as usize, true),
            Some('f' | 'g' | 'e') => out += &format!("{}", f64::from_bits(arg() as u64 | (arg() as u64) << 32)),
            Some(other) => {
                out.push('%');
                out += &spec;
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
}

fn emit(fmt_ptr: u32, args: &[u32]) {
    if LINES.load(Ordering::Relaxed) >= MAX_LINES {
        return;
    }
    let msg = format(&read_cstr(fmt_ptr as usize, false), args);
    let msg = msg.trim_end().to_string();
    let mut last = LAST.lock().unwrap();
    if last.0 == msg {
        last.1 += 1; // collapse repeats; the count is written when the message changes
        return;
    }
    if last.1 > 0 {
        log!("[game] (previous line repeated {} more times)", last.1);
    }
    *last = (msg.clone(), 0);
    LINES.fetch_add(1, Ordering::Relaxed);
    log!("[game] {msg}");
}

// cdecl: the caller pops, so reading more slots than were pushed is harmless.
unsafe extern "C" fn print_level(_this: usize, _level: u32, fmt: u32, a0: u32, a1: u32, a2: u32, a3: u32, a4: u32, a5: u32, a6: u32, a7: u32, a8: u32, a9: u32) -> u32 {
    emit(fmt, &[a0, a1, a2, a3, a4, a5, a6, a7, a8, a9]);
    0
}

unsafe extern "C" fn print(_this: usize, fmt: u32, a0: u32, a1: u32, a2: u32, a3: u32, a4: u32, a5: u32, a6: u32, a7: u32, a8: u32, a9: u32) -> u32 {
    emit(fmt, &[a0, a1, a2, a3, a4, a5, a6, a7, a8, a9]);
    0
}

/// Installs (or re-installs, if the game recreated its logger) our vtable. Main thread, per frame.
pub fn on_frame() {
    let Some(exe) = *EXE else { return };
    let Some(obj) = read_u32(logger_global(exe)).filter(|&o| o != 0) else { return };
    let Some(vt) = read_u32(obj as usize) else { return };
    if vt as usize == OUR_VTABLE.load(Ordering::Relaxed) {
        return;
    }
    let mut table = vec![0usize; SLOTS];
    for (i, slot) in table.iter_mut().enumerate() {
        *slot = read_u32(vt as usize + 4 * i).unwrap_or(0) as usize;
    }
    type PrintLevel = unsafe extern "C" fn(usize, u32, u32, u32, u32, u32, u32, u32, u32, u32, u32, u32, u32) -> u32;
    type Print = unsafe extern "C" fn(usize, u32, u32, u32, u32, u32, u32, u32, u32, u32, u32, u32) -> u32;
    table[PRINT_LEVEL] = print_level as PrintLevel as usize;
    table[PRINT_A] = print as Print as usize;
    table[PRINT_B] = print as Print as usize;
    // Leaked on purpose: the game holds a pointer to it for the rest of the process.
    let ours = Box::leak(table.into_boxed_slice()).as_ptr() as usize;
    if write_mem(obj as usize, &(ours as u32).to_le_bytes()) {
        OUR_VTABLE.store(ours, Ordering::Relaxed);
        log!("game debug log enabled (logger {obj:#x})");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printf_subset() {
        let s = b"abc\0";
        let args = [5u32, 0xAB, s.as_ptr() as u32];
        assert_eq!(format("n=%d x=0x%08X s=%s %%", &args), "n=5 x=0x000000AB s=abc %");
        assert_eq!(format("%f", &[0, 0x3FF8_0000]), "1.5");
        assert_eq!(format("%s", &[0x10]), "<bad ptr 0x10>");
    }
}

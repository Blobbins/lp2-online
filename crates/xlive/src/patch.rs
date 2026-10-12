//! Verified code patches: a patch is written only if the bytes at its address are exactly the
//! original ones, so an unknown exe version is left untouched instead of being corrupted.

use crate::log;
use crate::mods::{read_mem, Exe};
use windows_sys::Win32::System::Diagnostics::Debug::FlushInstructionCache;
use windows_sys::Win32::System::Memory::{VirtualProtect, PAGE_EXECUTE_READWRITE};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

pub struct Patch {
    pub name: &'static str,
    pub exe: Exe,
    pub addr: usize,
    pub original: &'static [u8],
    pub patched: &'static [u8],
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Applied,
    AlreadyApplied,
    /// The bytes are neither original nor patched: different exe build or someone else's patch.
    Mismatch,
}

pub fn apply(p: &Patch) -> Outcome {
    assert_eq!(p.original.len(), p.patched.len(), "{}: patch size", p.name);
    let mut now = vec![0u8; p.original.len()];
    let outcome = if !read_mem(p.addr, &mut now) {
        Outcome::Mismatch
    } else if now == p.patched {
        Outcome::AlreadyApplied
    } else if now != p.original {
        Outcome::Mismatch
    } else {
        unsafe {
            let mut old = 0;
            if VirtualProtect(p.addr as _, p.patched.len(), PAGE_EXECUTE_READWRITE, &mut old) == 0 {
                return Outcome::Mismatch;
            }
            std::ptr::copy_nonoverlapping(p.patched.as_ptr(), p.addr as *mut u8, p.patched.len());
            VirtualProtect(p.addr as _, p.patched.len(), old, &mut old);
            FlushInstructionCache(GetCurrentProcess(), p.addr as _, p.patched.len());
        }
        Outcome::Applied
    };
    log!("patch {}: {outcome:?}", p.name);
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patches_only_matching_bytes() {
        // A writable-and-executable copy is not needed: test on heap memory.
        let mut buf = vec![1u8, 2, 3, 4];
        let p = Patch { name: "t", exe: Exe::Dx9, addr: buf.as_mut_ptr() as usize, original: &[2, 3], patched: &[9, 9] };
        let p = Patch { addr: p.addr + 1, ..p };
        assert_eq!(apply(&p), Outcome::Applied);
        assert_eq!(buf, [1, 9, 9, 4]);
        assert_eq!(apply(&p), Outcome::AlreadyApplied);
        let q = Patch { name: "t", exe: Exe::Dx9, addr: buf.as_mut_ptr() as usize, original: &[7], patched: &[8] };
        assert_eq!(apply(&q), Outcome::Mismatch);
        assert_eq!(buf[0], 1);
    }
}

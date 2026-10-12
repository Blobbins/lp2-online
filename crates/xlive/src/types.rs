//! 32-bit GFWL structs as the game lays them out (MSVC x86: 8-byte members are 8-aligned).
//! Pointers inside game-owned buffers are written as `u32`.

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct XnAddr {
    pub ina: [u8; 4],
    pub ina_online: [u8; 4],
    /// Network byte order.
    pub port_online: [u8; 2],
    pub enet: [u8; 6],
    pub online: [u8; 20],
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct XSessionInfo {
    pub id: [u8; 8],
    pub host: XnAddr,
    pub key: [u8; 16],
}

#[repr(C, align(8))]
#[derive(Clone, Copy, Default)]
pub struct XUserData {
    pub kind: u8,
    pub _pad: [u8; 7],
    /// The union: a number, or `cbData u32` + `pointer u32` for strings and blobs.
    pub value: [u8; 8],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct XUserProperty {
    pub id: u32,
    pub _pad: u32,
    pub data: XUserData,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct XUserContext {
    pub id: u32,
    pub value: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct XSessionSearchResult {
    pub info: XSessionInfo,
    pub open_public: u32,
    pub open_private: u32,
    pub filled_public: u32,
    pub filled_private: u32,
    pub property_count: u32,
    pub context_count: u32,
    pub properties: u32,
    pub contexts: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct XUserProfileSetting {
    pub source: u32,
    pub _pad: u32,
    /// dwUserIndex or XUID.
    pub user: [u8; 8],
    pub id: u32,
    pub _pad2: u32,
    pub data: XUserData,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct XUserSigninInfo {
    pub xuid: [u8; 8],
    pub flags: u32,
    pub state: u32,
    pub guest: u32,
    pub sponsor: u32,
    pub name: [u8; 16],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct XnQosInfo {
    pub flags: u8,
    pub _reserved: u8,
    pub probes_sent: u16,
    pub probes_received: u16,
    pub data_size: u16,
    pub data: u32,
    pub rtt_min: u16,
    pub rtt_median: u16,
    pub up_bps: u32,
    pub down_bps: u32,
}

#[repr(C, align(8))]
#[derive(Clone, Copy, Default)]
pub struct XSessionRegistrant {
    pub machine_id: u64,
    pub trustworthiness: u32,
    pub user_count: u32,
    pub users: u32,
}

/// `{ count, pointer }` headers: XSESSION_SEARCHRESULT_HEADER, XNQOS (count, pending),
/// XUSER_READ_PROFILE_SETTING_RESULT, XSESSION_REGISTRATION_RESULTS.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct CountPtr {
    pub count: u32,
    pub ptr: u32,
}

const _: () = {
    use std::mem::size_of;
    assert!(size_of::<XnAddr>() == 36);
    assert!(size_of::<XSessionInfo>() == 60);
    assert!(size_of::<XUserData>() == 16);
    assert!(size_of::<XUserProperty>() == 24);
    assert!(size_of::<XUserContext>() == 8);
    assert!(size_of::<XSessionSearchResult>() == 92);
    assert!(size_of::<XUserProfileSetting>() == 40);
    assert!(size_of::<XUserSigninInfo>() == 40);
    assert!(size_of::<XnQosInfo>() == 24);
    assert!(size_of::<XSessionRegistrant>() == 24);
};

/// Lays structs out in a caller-supplied buffer. With no buffer it only measures,
/// assuming worst-case alignment padding so the measured size always fits.
pub struct Out {
    base: *mut u8,
    pub len: usize,
}

impl Out {
    pub fn measure() -> Out {
        Out { base: std::ptr::null_mut(), len: 0 }
    }

    pub fn new(base: *mut u8) -> Out {
        Out { base, len: 0 }
    }

    /// Reserves `size` bytes; returns their address (0 while measuring).
    pub fn alloc(&mut self, size: usize, align: usize) -> usize {
        if self.base.is_null() {
            self.len += align - 1 + size;
            return 0;
        }
        let addr = self.base as usize + self.len;
        let addr = addr.next_multiple_of(align);
        self.len = addr - self.base as usize + size;
        addr
    }

    pub fn put<T: Copy>(&mut self, v: T) -> usize {
        let addr = self.alloc(size_of::<T>(), align_of::<T>());
        if addr != 0 {
            unsafe { (addr as *mut T).write(v) };
        }
        addr
    }

    pub fn bytes(&mut self, b: &[u8], align: usize) -> usize {
        let addr = self.alloc(b.len(), align);
        if addr != 0 {
            unsafe { std::ptr::copy_nonoverlapping(b.as_ptr(), addr as *mut u8, b.len()) };
        }
        addr
    }

    /// Reserves an array of `n` `T`s to fill later.
    pub fn array<T: Copy>(&mut self, n: usize) -> usize {
        self.alloc(size_of::<T>() * n, align_of::<T>())
    }
}

/// Writes `values[i]` into an array reserved with `Out::array`.
pub unsafe fn set<T: Copy>(array: usize, i: usize, v: T) {
    if array != 0 {
        (array as *mut T).add(i).write(v);
    }
}

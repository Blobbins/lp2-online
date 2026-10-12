//! IXHVEngine (voice chat) that accepts every call and never has audio.
//! shortcut: no voice chat; implement capture/playback here if wanted later.

type This = *mut Engine;
const S_OK: i32 = 0;
const E_PENDING: i32 = 0x8000_000Au32 as i32;

#[repr(C)]
pub struct VTable {
    add_ref: unsafe extern "system" fn(This) -> i32,
    release: unsafe extern "system" fn(This) -> i32,
    lock: unsafe extern "system" fn(This, u32) -> i32,
    start_local: unsafe extern "system" fn(This, u32, *const u32, u32) -> i32,
    stop_local: unsafe extern "system" fn(This, u32, *const u32, u32) -> i32,
    start_remote: unsafe extern "system" fn(This, u64, *const u32, u32) -> i32,
    stop_remote: unsafe extern "system" fn(This, u64, *const u32, u32) -> i32,
    set_max_decode_packets: unsafe extern "system" fn(This, u32) -> i32,
    register_local: unsafe extern "system" fn(This, u32) -> i32,
    unregister_local: unsafe extern "system" fn(This, u32) -> i32,
    register_remote: unsafe extern "system" fn(This, u64, usize, usize, usize) -> i32,
    unregister_remote: unsafe extern "system" fn(This, u64) -> i32,
    get_remote_talkers: unsafe extern "system" fn(This, *mut u32, *mut u64) -> i32,
    is_headset_present: unsafe extern "system" fn(This, u32) -> i32,
    is_local_talking: unsafe extern "system" fn(This, u32) -> i32,
    is_remote_talking: unsafe extern "system" fn(This, u64) -> i32,
    get_data_ready_flags: unsafe extern "system" fn(This) -> u32,
    get_local_chat_data: unsafe extern "system" fn(This, u32, *mut u8, *mut u32, *mut u32) -> i32,
    set_playback_priority: unsafe extern "system" fn(This, u64, u32, u32) -> i32,
    submit_incoming_chat_data: unsafe extern "system" fn(This, u64, *const u8, *mut u32) -> i32,
}

#[repr(C)]
pub struct Engine {
    vtable: &'static VTable,
}

unsafe extern "system" fn one(_: This) -> i32 {
    1
}
unsafe extern "system" fn ok_u32(_: This, _: u32) -> i32 {
    S_OK
}
unsafe extern "system" fn no_u32(_: This, _: u32) -> i32 {
    0
}
unsafe extern "system" fn ok_modes(_: This, _: u32, _: *const u32, _: u32) -> i32 {
    S_OK
}
unsafe extern "system" fn ok_remote_modes(_: This, _: u64, _: *const u32, _: u32) -> i32 {
    S_OK
}
unsafe extern "system" fn ok_register_remote(_: This, _: u64, _: usize, _: usize, _: usize) -> i32 {
    S_OK
}
unsafe extern "system" fn ok_u64(_: This, _: u64) -> i32 {
    S_OK
}
unsafe extern "system" fn no_u64(_: This, _: u64) -> i32 {
    0
}
unsafe extern "system" fn remote_talkers(_: This, count: *mut u32, _: *mut u64) -> i32 {
    if let Some(c) = count.as_mut() {
        *c = 0;
    }
    S_OK
}
unsafe extern "system" fn ready_flags(_: This) -> u32 {
    0
}
unsafe extern "system" fn local_chat(_: This, _: u32, _: *mut u8, size: *mut u32, packets: *mut u32) -> i32 {
    for p in [size, packets] {
        if let Some(p) = p.as_mut() {
            *p = 0;
        }
    }
    E_PENDING
}
unsafe extern "system" fn priority(_: This, _: u64, _: u32, _: u32) -> i32 {
    S_OK
}
unsafe extern "system" fn submit(_: This, _: u64, _: *const u8, _: *mut u32) -> i32 {
    S_OK
}

static VTABLE: VTable = VTable {
    add_ref: one,
    release: one,
    lock: ok_u32,
    start_local: ok_modes,
    stop_local: ok_modes,
    start_remote: ok_remote_modes,
    stop_remote: ok_remote_modes,
    set_max_decode_packets: ok_u32,
    register_local: ok_u32,
    unregister_local: ok_u32,
    register_remote: ok_register_remote,
    unregister_remote: ok_u64,
    get_remote_talkers: remote_talkers,
    is_headset_present: no_u32,
    is_local_talking: no_u32,
    is_remote_talking: no_u64,
    get_data_ready_flags: ready_flags,
    get_local_chat_data: local_chat,
    set_playback_priority: priority,
    submit_incoming_chat_data: submit,
};

/// One shared object; Release never frees it.
pub fn engine() -> *mut Engine {
    static ENGINE: Engine = Engine { vtable: &VTABLE };
    &ENGINE as *const Engine as *mut Engine
}

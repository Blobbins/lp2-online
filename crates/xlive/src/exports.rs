//! Every function LP2 imports from xlive.dll (ordinals in `xlive.def`).
//! These are `__stdcall`: the argument list must match exactly or the game's stack is corrupted.
//! Functions the game only needs to "not fail" return what the original xlive-dummy returned.

use crate::net::{self, ip_str, Net, NET};
use crate::session::{self, SESSIONS};
use crate::storage;
use crate::types::*;
use crate::{complete, log, new_handle, user, xhv, XOverlapped, *};
use std::ffi::c_void;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::Networking::WinSock as ws;
use windows_sys::Win32::System::Threading::SetEvent;

type Ov = *mut XOverlapped;
const SOCK_DGRAM: i32 = 2;
const IPPROTO_VDP: i32 = 254;
const FIONBIO: i32 = 0x8004_667Eu32 as i32;
const FIONREAD: i32 = 0x4004_667F;
const S_OK: i32 = 0;
const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
const E_OUTOFMEMORY: i32 = 0x8007_000Eu32 as i32;
const LOCAL_USER: u32 = 0;

unsafe fn wsa_fail(e: i32) -> i32 {
    ws::WSASetLastError(e);
    -1
}

/// `(raw ip, host-order port)` of a sockaddr_in.
unsafe fn read_sin(a: *const u8, len: i32) -> Option<(u32, u16)> {
    if a.is_null() || len < 8 {
        return None;
    }
    let port = u16::from_be_bytes([*a.add(2), *a.add(3)]);
    Some(((a.add(4) as *const u32).read_unaligned(), port))
}

unsafe fn write_sin(a: *mut u8, len: *mut i32, ip: u32, port: u16) {
    if a.is_null() || len.is_null() || *len < 16 {
        return;
    }
    std::ptr::write_bytes(a, 0, 16);
    a.cast::<u16>().write_unaligned(ws::AF_INET);
    a.add(2).cast::<[u8; 2]>().write_unaligned(port.to_be_bytes());
    a.add(4).cast::<u32>().write_unaligned(ip);
    *len = 16;
}

fn result(r: Result<i32, i32>) -> i32 {
    r.unwrap_or_else(|e| unsafe { wsa_fail(e) })
}

// ---------------------------------------------------------------- sockets (1-40)

#[no_mangle]
pub unsafe extern "system" fn XWSAStartup(version: u16, data: *mut ws::WSADATA) -> i32 {
    crate::trace("XWSAStartup");
    ws::WSAStartup(version, data)
}

#[no_mangle]
pub unsafe extern "system" fn XWSACleanup() -> i32 {
    crate::trace("XWSACleanup");
    ws::WSACleanup()
}

#[no_mangle]
pub unsafe extern "system" fn XSocketCreate(af: i32, ty: i32, protocol: i32) -> usize {
    crate::trace("XSocketCreate");
    if ty == SOCK_DGRAM {
        let h = NET.create();
        log!("XSocketCreate udp/vdp (protocol {protocol}) -> {h:#x}");
        return h;
    }
    let s = ws::socket(af, ty, if protocol == IPPROTO_VDP { 0 } else { protocol });
    log!("XSocketCreate real af {af} type {ty} protocol {protocol} -> {s:#x}");
    s
}

#[no_mangle]
pub unsafe extern "system" fn XSocketClose(s: usize) -> i32 {
    crate::trace("XSocketClose");
    if Net::is_virtual(s) {
        return result(NET.close(s).map(|_| 0));
    }
    ws::closesocket(s)
}

#[no_mangle]
pub unsafe extern "system" fn XSocketIOCTLSocket(s: usize, cmd: i32, arg: *mut u32) -> i32 {
    crate::trace("XSocketIOCTLSocket");
    if !Net::is_virtual(s) {
        return ws::ioctlsocket(s, cmd, arg);
    }
    if arg.is_null() {
        return wsa_fail(net::WSAEINVAL);
    }
    result(NET.with(s, |v| {
        match cmd {
            FIONBIO => v.nonblocking = *arg != 0,
            FIONREAD => *arg = v.queue.front().map_or(0, |p| p.data.len() as u32),
            _ => log!("XSocketIOCTLSocket: ignoring cmd {cmd:#x}"),
        }
        0
    }))
}

#[no_mangle]
pub unsafe extern "system" fn XSocketSetSockOpt(s: usize, level: i32, name: i32, value: *const u8, size: i32) -> i32 {
    crate::trace("XSocketSetSockOpt");
    if !Net::is_virtual(s) {
        return ws::setsockopt(s, level, name, value, size);
    }
    // Buffer sizes, broadcast, etc. mean nothing for a virtual socket.
    result(NET.with(s, |_| 0))
}

#[no_mangle]
pub unsafe extern "system" fn XSocketGetSockName(s: usize, addr: *mut u8, len: *mut i32) -> i32 {
    crate::trace("XSocketGetSockName");
    if !Net::is_virtual(s) {
        return ws::getsockname(s, addr.cast(), len);
    }
    result(NET.with(s, |v| v.port).map(|port| {
        write_sin(addr, len, 0, port);
        0
    }))
}

#[no_mangle]
pub unsafe extern "system" fn XSocketGetPeerName(s: usize, addr: *mut u8, len: *mut i32) -> i32 {
    crate::trace("XSocketGetPeerName");
    if !Net::is_virtual(s) {
        return ws::getpeername(s, addr.cast(), len);
    }
    result(NET.with(s, |v| v.peer).and_then(|p| p.ok_or(net::WSAENOTCONN)).map(|(ip, port)| {
        write_sin(addr, len, ip, port);
        0
    }))
}

#[no_mangle]
pub unsafe extern "system" fn XSocketBind(s: usize, addr: *const u8, len: i32) -> i32 {
    crate::trace("XSocketBind");
    if !Net::is_virtual(s) {
        return ws::bind(s, addr.cast(), len);
    }
    let Some((_, port)) = read_sin(addr, len) else { return wsa_fail(net::WSAEINVAL) };
    result(NET.bind(s, port).map(|_| 0))
}

#[no_mangle]
pub unsafe extern "system" fn XSocketConnect(s: usize, addr: *const u8, len: i32) -> i32 {
    crate::trace("XSocketConnect");
    let Some((ip, port)) = read_sin(addr, len) else { return wsa_fail(net::WSAEINVAL) };
    if Net::is_virtual(s) {
        return result(NET.with(s, |v| v.peer = Some((ip, port))).map(|_| 0));
    }
    // TCP needs a direct path to the peer's real address.
    let Some(real) = NET.real_ip(ip) else {
        log!("XSocketConnect tcp to {}:{port}: no direct route", ip_str(ip));
        return wsa_fail(10065); // WSAEHOSTUNREACH
    };
    log!("XSocketConnect tcp {}:{port} -> {real}:{port}", ip_str(ip));
    let mut sin = [0u8; 16];
    let mut n = 16;
    write_sin(sin.as_mut_ptr(), &mut n, u32::from_ne_bytes(real.octets()), port);
    ws::connect(s, sin.as_ptr().cast(), 16)
}

#[no_mangle]
pub unsafe extern "system" fn XSocketListen(s: usize, backlog: i32) -> i32 {
    crate::trace("XSocketListen");
    if Net::is_virtual(s) {
        return wsa_fail(10045); // WSAEOPNOTSUPP
    }
    ws::listen(s, backlog)
}

#[no_mangle]
pub unsafe extern "system" fn XSocketAccept(s: usize, addr: *mut u8, len: *mut i32) -> usize {
    crate::trace("XSocketAccept");
    if Net::is_virtual(s) {
        wsa_fail(10045);
        return ws::INVALID_SOCKET;
    }
    ws::accept(s, addr.cast(), len)
}

#[no_mangle]
pub unsafe extern "system" fn XSocketRecv(s: usize, buf: *mut u8, len: i32, flags: i32) -> i32 {
    crate::trace("XSocketRecv");
    if !Net::is_virtual(s) {
        return ws::recv(s, buf, len, flags);
    }
    XSocketRecvFrom(s, buf, len, flags, std::ptr::null_mut(), std::ptr::null_mut())
}

#[no_mangle]
pub unsafe extern "system" fn XSocketRecvFrom(s: usize, buf: *mut u8, len: i32, flags: i32, from: *mut u8, from_len: *mut i32) -> i32 {
    crate::trace("XSocketRecvFrom");
    if !Net::is_virtual(s) {
        return ws::recvfrom(s, buf, len, flags, from.cast(), from_len);
    }
    if buf.is_null() || len < 0 {
        return wsa_fail(net::WSAEINVAL);
    }
    let out = std::slice::from_raw_parts_mut(buf, len as usize);
    result(NET.recv_from(s, out).map(|(n, src, port)| {
        write_sin(from, from_len, src, port);
        n as i32
    }))
}

#[no_mangle]
pub unsafe extern "system" fn XSocketSend(s: usize, buf: *const u8, len: i32, flags: i32) -> i32 {
    crate::trace("XSocketSend");
    if !Net::is_virtual(s) {
        return ws::send(s, buf, len, flags);
    }
    let peer = match NET.with(s, |v| v.peer) {
        Ok(Some(p)) => p,
        Ok(None) => return wsa_fail(net::WSAENOTCONN),
        Err(e) => return wsa_fail(e),
    };
    let mut sin = [0u8; 16];
    let mut n = 16;
    write_sin(sin.as_mut_ptr(), &mut n, peer.0, peer.1);
    XSocketSendTo(s, buf, len, flags, sin.as_ptr(), 16)
}

#[no_mangle]
pub unsafe extern "system" fn XSocketSendTo(s: usize, buf: *const u8, len: i32, flags: i32, to: *const u8, to_len: i32) -> i32 {
    crate::trace("XSocketSendTo");
    if !Net::is_virtual(s) {
        return ws::sendto(s, buf, len, flags, to.cast(), to_len);
    }
    let Some((ip, port)) = read_sin(to, to_len) else { return wsa_fail(net::WSAEINVAL) };
    if buf.is_null() || len < 0 {
        return wsa_fail(net::WSAEINVAL);
    }
    result(NET.send_to(s, ip, port, std::slice::from_raw_parts(buf, len as usize)).map(|n| n as i32))
}

#[no_mangle]
pub unsafe extern "system" fn XSocketWSAGetLastError() -> i32 {
    crate::trace("XSocketWSAGetLastError");
    ws::WSAGetLastError()
}

#[no_mangle]
pub extern "system" fn XSocketNTOHS(v: u16) -> u16 {
    crate::trace("XSocketNTOHS");
    u16::from_be(v)
}

#[no_mangle]
pub extern "system" fn XSocketHTONS(v: u16) -> u16 {
    crate::trace("XSocketHTONS");
    v.to_be()
}

// ---------------------------------------------------------------- XNet (51-84)

#[no_mangle]
pub unsafe extern "system" fn XNetStartup(params: *const u8) -> i32 {
    crate::trace("XNetStartup");
    if !params.is_null() {
        let n = (*params as usize).clamp(1, 13);
        let mut p = STARTUP_PARAMS.lock().unwrap();
        p[..n].copy_from_slice(std::slice::from_raw_parts(params, n));
        p[0] = 13;
        // Zero means "use the default" for every field.
        for (v, d) in p.iter_mut().zip(DEFAULT_STARTUP_PARAMS).skip(1) {
            if *v == 0 {
                *v = d;
            }
        }
        log!("XNetStartup params {:?}", *p);
    }
    std::sync::LazyLock::force(&NET);
    0
}

/// XNetStartupParams defaults (cfgQosDataLimitDiv4 = 16: lobby data up to 64 bytes; LP2 sends 56).
const DEFAULT_STARTUP_PARAMS: [u8; 13] = [13, 0, 8, 32, 16, 16, 8, 32, 16, 2, 3, 8, 2];
static STARTUP_PARAMS: std::sync::Mutex<[u8; 13]> = std::sync::Mutex::new(DEFAULT_STARTUP_PARAMS);

#[no_mangle]
pub extern "system" fn XNetRegisterKey(_id: *const c_void, _key: *const c_void) -> i32 {
    crate::trace("XNetRegisterKey");
    0
}

#[no_mangle]
pub extern "system" fn XNetUnregisterKey(_id: *const c_void) -> i32 {
    crate::trace("XNetUnregisterKey");
    0
}

#[no_mangle]
pub unsafe extern "system" fn XNetXnAddrToInAddr(addr: *const XnAddr, _id: *const c_void, out: *mut u32) -> i32 {
    crate::trace("XNetXnAddrToInAddr");
    let (Some(a), false) = (addr.as_ref(), out.is_null()) else { return net::WSAEINVAL };
    match NET.register(a) {
        Some(ip) => {
            *out = ip;
            0
        }
        None => {
            log!("XNetXnAddrToInAddr: not an LP2N address {a:?}");
            net::WSAEINVAL
        }
    }
}

#[no_mangle]
pub extern "system" fn XNetUnregisterInAddr(_ip: u32) -> i32 {
    crate::trace("XNetUnregisterInAddr");
    0
}

#[no_mangle]
pub unsafe extern "system" fn XNetXnAddrToMachineId(addr: *const XnAddr, out: *mut u64) -> i32 {
    crate::trace("XNetXnAddrToMachineId");
    let (Some(a), false) = (addr.as_ref(), out.is_null()) else { return net::WSAEINVAL };
    *out = 0xFA00_0000_0000_0000 | u32::from_le_bytes(a.online[..4].try_into().unwrap()) as u64;
    0
}

#[no_mangle]
pub extern "system" fn XNetConnect(_ip: u32) -> i32 {
    crate::trace("XNetConnect");
    0
}

#[no_mangle]
pub extern "system" fn XNetGetConnectStatus(_ip: u32) -> i32 {
    crate::trace("XNetGetConnectStatus");
    2 // XNET_CONNECT_STATUS_CONNECTED
}

#[no_mangle]
pub unsafe extern "system" fn XNetDnsLookup(_host: *const u8, _event: HANDLE, out: *mut *mut c_void) -> i32 {
    crate::trace("XNetDnsLookup");
    // shortcut: no DNS (the dummy had none either); implement if the log shows LP2 needs it.
    if !out.is_null() {
        *out = std::ptr::null_mut();
    }
    1
}

#[no_mangle]
pub extern "system" fn XNetDnsRelease(_dns: *mut c_void) -> i32 {
    crate::trace("XNetDnsRelease");
    0
}

#[no_mangle]
pub unsafe extern "system" fn XNetQosListen(id: *const [u8; 8], data: *const u8, size: u32, _bps: u32, flags: u32) -> i32 {
    crate::trace("XNetQosListen");
    const SET_DATA: u32 = 0x04;
    const RELEASE: u32 = 0x10;
    let Some(id) = id.as_ref().map(|i| u64::from_le_bytes(*i)) else { return net::WSAEINVAL };
    if flags & RELEASE != 0 {
        session::set_qos_listen(id, None);
    } else if flags & SET_DATA != 0 {
        let d = if data.is_null() { vec![] } else { std::slice::from_raw_parts(data, size.min(lp2_proto::MAX_QOS as u32) as usize).to_vec() };
        log!("XNetQosListen {id:016x}: {} byte(s)", d.len());
        session::set_qos_listen(id, Some(d));
    }
    0
}

#[no_mangle]
pub unsafe extern "system" fn XNetQosLookup(
    count: u32,
    _addrs: *const *const XnAddr,
    ids: *const *const [u8; 8],
    _keys: *const *const c_void,
    _service_count: u32,
    _service_ips: *const u32,
    _service_ids: *const u32,
    _probes: u32,
    _bps: u32,
    _flags: u32,
    event: HANDLE,
    out: *mut usize,
) -> i32 {
    crate::trace("XNetQosLookup");
    if out.is_null() {
        return net::WSAEINVAL;
    }
    let ids: Vec<Option<u64>> = (0..count as usize)
        .map(|i| if ids.is_null() { None } else { (*ids.add(i)).as_ref().map(|k| u64::from_le_bytes(*k)) })
        .collect();
    *out = session::qos_lookup(&ids);
    if !event.is_null() {
        SetEvent(event);
    }
    0
}

#[no_mangle]
pub unsafe extern "system" fn XNetQosServiceLookup(_flags: u32, event: HANDLE, out: *mut usize) -> i32 {
    crate::trace("XNetQosServiceLookup");
    if out.is_null() {
        return net::WSAEINVAL;
    }
    *out = session::qos_lookup(&[]);
    if !event.is_null() {
        SetEvent(event);
    }
    0
}

#[no_mangle]
pub extern "system" fn XNetQosRelease(qos: usize) -> i32 {
    crate::trace("XNetQosRelease");
    session::qos_release(qos);
    0
}

#[no_mangle]
pub unsafe extern "system" fn XNetGetTitleXnAddr(out: *mut XnAddr) -> u32 {
    crate::trace("XNetGetTitleXnAddr");
    if let Some(o) = out.as_mut() {
        *o = NET.xnaddr();
    }
    0x08 | 0x80 // XNET_GET_XNADDR_DHCP | XNET_GET_XNADDR_ONLINE
}

#[no_mangle]
pub extern "system" fn XNetGetEthernetLinkStatus() -> u32 {
    crate::trace("XNetGetEthernetLinkStatus");
    1 // XNET_ETHERNET_LINK_ACTIVE
}

#[no_mangle]
pub extern "system" fn XNetGetBroadcastVersionStatus(_reset: i32) -> u32 {
    crate::trace("XNetGetBroadcastVersionStatus");
    0
}

#[no_mangle]
pub unsafe extern "system" fn XNetGetOpt(id: u32, value: *mut u8, size: *mut u32) -> i32 {
    crate::trace("XNetGetOpt");
    const XNET_OPTID_STARTUP_PARAMS: u32 = 1;
    let (Some(size), false) = (size.as_mut(), value.is_null()) else { return 10014 }; // WSAEFAULT
    // Traffic counters (2..=9) are 8-byte totals for byte ids, 4-byte for frame ids; report zero.
    let data: Vec<u8> = match id {
        XNET_OPTID_STARTUP_PARAMS => STARTUP_PARAMS.lock().unwrap().to_vec(),
        2 | 4 | 6 | 8 => vec![0; 8],
        3 | 5 | 7 | 9 => vec![0; 4],
        _ => {
            log!("XNetGetOpt {id:#x}: unsupported");
            return net::WSAEINVAL;
        }
    };
    if (*size as usize) < data.len() {
        *size = data.len() as u32;
        return net::WSAEMSGSIZE;
    }
    std::ptr::copy_nonoverlapping(data.as_ptr(), value, data.len());
    *size = data.len() as u32;
    0
}

#[no_mangle]
pub unsafe extern "system" fn XNetGetXnAddrPlatform(_addr: *const XnAddr, platform: *mut u32) -> i32 {
    crate::trace("XNetGetXnAddrPlatform");
    if let Some(p) = platform.as_mut() {
        *p = 2; // XNET_XNADDR_PLATFORM_WINPC
    }
    0
}

#[no_mangle]
pub unsafe extern "system" fn XNetGetSystemLinkPort(port: *mut u16) -> i32 {
    crate::trace("XNetGetSystemLinkPort");
    if let Some(p) = port.as_mut() {
        *p = 3074u16.to_be();
    }
    0
}

#[no_mangle]
pub extern "system" fn XNetSetSystemLinkPort(_port: u16) -> i32 {
    crate::trace("XNetSetSystemLinkPort");
    0
}

// ---------------------------------------------------------------- notify / overlapped

#[no_mangle]
pub unsafe extern "system" fn XNotifyGetNext(h: usize, filter: u32, id: *mut u32, param: *mut u32) -> i32 {
    crate::trace("XNotifyGetNext");
    let Some((n, p)) = user::next_notification(h, filter) else { return 0 };
    log!("XNotifyGetNext -> {n:#x} ({p:#x})");
    if let Some(id) = id.as_mut() {
        *id = n;
    }
    if let Some(param) = param.as_mut() {
        *param = p;
    }
    1
}

#[no_mangle]
pub unsafe extern "system" fn XGetOverlappedExtendedError(ov: Ov) -> u32 {
    crate::trace("XGetOverlappedExtendedError");
    ov.as_ref().map_or(0, |o| o.extended_error)
}

#[no_mangle]
pub unsafe extern "system" fn XGetOverlappedResult(ov: Ov, result: *mut u32, _wait: i32) -> u32 {
    crate::trace("XGetOverlappedResult");
    let Some(o) = ov.as_ref() else { return ERROR_INVALID_PARAMETER };
    if o.internal_low == ERROR_IO_PENDING {
        return ERROR_IO_INCOMPLETE;
    }
    if let Some(r) = result.as_mut() {
        *r = o.internal_high;
    }
    o.internal_low
}

#[no_mangle]
pub unsafe extern "system" fn XNotifyCreateListener(areas: u64) -> usize {
    crate::trace("XNotifyCreateListener");
    user::create_listener(areas)
}

#[no_mangle]
pub unsafe extern "system" fn XCloseHandle(h: usize) -> i32 {
    crate::trace("XCloseHandle");
    user::close_listener(h);
    session::close(h);
    if h != 0 && h != usize::MAX {
        CloseHandle(h as HANDLE);
    }
    1
}

#[no_mangle]
pub extern "system" fn XCancelOverlapped(_ov: Ov) -> u32 {
    crate::trace("XCancelOverlapped");
    0
}

#[no_mangle]
pub unsafe extern "system" fn XEnumerate(_h: usize, _buf: *mut u8, _size: u32, count: *mut u32, ov: Ov) -> u32 {
    crate::trace("XEnumerate");
    if let Some(c) = count.as_mut() {
        *c = 0;
    }
    complete(ov, ERROR_NO_MORE_FILES, 0)
}

// ---------------------------------------------------------------- XLive core (5002-5038)

#[no_mangle]
pub extern "system" fn XLiveRender() -> i32 {
    crate::trace("XLiveRender");
    crate::gamelog::on_frame();
    crate::mods::on_frame();
    S_OK
}

#[no_mangle]
pub extern "system" fn XLiveUninitialize() {}

#[no_mangle]
pub extern "system" fn XLiveOnResetDevice(_params: *mut c_void) -> i32 {
    crate::trace("XLiveUninitialize");
    S_OK
}

#[no_mangle]
pub unsafe extern "system" fn XHVCreateEngine(_params: *const c_void, thread: *mut HANDLE, engine: *mut *mut xhv::Engine) -> i32 {
    crate::trace("XHVCreateEngine");
    if engine.is_null() {
        return E_INVALIDARG;
    }
    if let Some(t) = thread.as_mut() {
        *t = std::ptr::null_mut();
    }
    *engine = xhv::engine();
    S_OK
}

#[no_mangle]
pub unsafe extern "system" fn XLivePBufferAllocate(size: u32, out: *mut *mut u8) -> i32 {
    crate::trace("XLivePBufferAllocate");
    if out.is_null() {
        return E_INVALIDARG;
    }
    // { size: u32, data... } like the dummy; the size lets XLivePBufferFree rebuild the layout.
    let Ok(layout) = std::alloc::Layout::from_size_align(size as usize + 16, 8) else { return E_OUTOFMEMORY };
    let p = std::alloc::alloc_zeroed(layout);
    if p.is_null() {
        return E_OUTOFMEMORY;
    }
    p.cast::<u32>().write(size);
    *out = p;
    S_OK
}

#[no_mangle]
pub unsafe extern "system" fn XLivePBufferFree(p: *mut u8) -> i32 {
    crate::trace("XLivePBufferFree");
    if !p.is_null() {
        let size = p.cast::<u32>().read() as usize;
        std::alloc::dealloc(p, std::alloc::Layout::from_size_align_unchecked(size + 16, 8));
    }
    S_OK
}

#[no_mangle]
pub extern "system" fn XLivePreTranslateMessage(_msg: *const c_void) -> i32 {
    crate::trace("XLivePreTranslateMessage");
    0
}

/// Plain copy, exactly like the xlive-dummy, so saves written by either DLL load in both.
unsafe fn copy_protected(src: *const u8, src_size: u32, dst: *mut u8, dst_size: *mut u32) -> i32 {
    if dst_size.is_null() {
        return E_INVALIDARG;
    }
    if dst.is_null() || *dst_size < src_size {
        *dst_size = src_size;
        return 0x8007_007Au32 as i32; // HRESULT_FROM_WIN32(ERROR_INSUFFICIENT_BUFFER)
    }
    std::ptr::copy(src, dst, src_size as usize);
    *dst_size = src_size;
    S_OK
}

#[no_mangle]
pub unsafe extern "system" fn XLiveProtectData(src: *const u8, src_size: u32, dst: *mut u8, dst_size: *mut u32, _h: HANDLE) -> i32 {
    crate::trace("XLiveProtectData");
    copy_protected(src, src_size, dst, dst_size)
}

#[no_mangle]
pub unsafe extern "system" fn XLiveUnprotectData(src: *const u8, src_size: u32, dst: *mut u8, dst_size: *mut u32, _h: *mut HANDLE) -> i32 {
    crate::trace("XLiveUnprotectData");
    copy_protected(src, src_size, dst, dst_size)
}

#[no_mangle]
pub unsafe extern "system" fn XLiveCreateProtectedDataContext(_info: *const c_void, h: *mut usize) -> i32 {
    crate::trace("XLiveCreateProtectedDataContext");
    if h.is_null() {
        return E_INVALIDARG;
    }
    *h = new_handle();
    S_OK
}

#[no_mangle]
pub unsafe extern "system" fn XLiveQueryProtectedDataInformation(_h: HANDLE, info: *mut [u32; 2]) -> i32 {
    crate::trace("XLiveQueryProtectedDataInformation");
    if let Some(i) = info.as_mut() {
        i[1] = 0; // dwFlags
    }
    S_OK
}

#[no_mangle]
pub unsafe extern "system" fn XLiveCloseProtectedDataContext(h: HANDLE) -> i32 {
    crate::trace("XLiveCloseProtectedDataContext");
    CloseHandle(h);
    S_OK
}

// ---------------------------------------------------------------- UI (no overlay; dummy results)

#[no_mangle]
pub extern "system" fn XShowGameInviteUI(_user: u32, _xuids: *const u64, _count: u32, _reserved: *const c_void) -> u32 {
    crate::trace("XShowGameInviteUI");
    1
}

#[no_mangle]
pub extern "system" fn XShowMessageComposeUI(_user: u32, _xuids: *const u64, _count: u32, _text: *const u16) -> u32 {
    crate::trace("XShowMessageComposeUI");
    1
}

#[no_mangle]
pub extern "system" fn XShowGuideUI(_user: u32) -> u32 {
    crate::trace("XShowGuideUI");
    0
}

#[no_mangle]
pub unsafe extern "system" fn XShowKeyboardUI(
    _user: u32,
    _flags: u32,
    default_text: *const u16,
    _title: *const u16,
    _description: *const u16,
    result: *mut u16,
    result_len: u32,
    ov: Ov,
) -> u32 {
    crate::trace("XShowKeyboardUI");
    if ov.is_null() || result.is_null() || result_len == 0 {
        return ERROR_INVALID_PARAMETER;
    }
    // No on-screen keyboard: accept the default text.
    let mut n = 0;
    while !default_text.is_null() && n + 1 < result_len as usize && *default_text.add(n) != 0 {
        *result.add(n) = *default_text.add(n);
        n += 1;
    }
    *result.add(n) = 0;
    complete(ov, ERROR_SUCCESS, 0)
}

#[no_mangle]
pub extern "system" fn XShowGamerCardUI(_user: u32, _xuid: u64) -> u32 {
    crate::trace("XShowGamerCardUI");
    0
}

#[no_mangle]
pub extern "system" fn XShowSigninUI(_panes: u32, _flags: u32) -> u32 {
    crate::trace("XShowSigninUI");
    0
}

#[no_mangle]
pub unsafe extern "system" fn XShowMessageBoxUI(
    _user: u32,
    _title: *const u16,
    _text: *const u16,
    _button_count: u32,
    _buttons: *const *const u16,
    focused: u32,
    _flags: u32,
    result: *mut u32,
    ov: Ov,
) -> u32 {
    crate::trace("XShowMessageBoxUI");
    if let Some(r) = result.as_mut() {
        *r = focused; // as if the focused button was pressed
    }
    complete(ov, ERROR_SUCCESS, 0)
}

#[no_mangle]
pub extern "system" fn XShowFriendsUI(_user: u32) -> u32 {
    crate::trace("XShowFriendsUI");
    0
}

#[no_mangle]
pub extern "system" fn XShowMarketplaceUI(_user: u32, _entry: u32, _offer: u64, _categories: u32) -> u32 {
    crate::trace("XShowMarketplaceUI");
    1
}

// ---------------------------------------------------------------- users

#[no_mangle]
pub extern "system" fn XUserGetSigninState(user: u32) -> u32 {
    crate::trace("XUserGetSigninState");
    if user == LOCAL_USER { user::SIGNED_IN_TO_LIVE } else { 0 }
}

#[no_mangle]
pub unsafe extern "system" fn XUserGetName(user: u32, out: *mut u8, size: u32) -> u32 {
    crate::trace("XUserGetName");
    if user != LOCAL_USER {
        return ERROR_NOT_LOGGED_ON;
    }
    if out.is_null() || size == 0 {
        return ERROR_INVALID_PARAMETER;
    }
    let name = CONFIG.name.as_bytes();
    let n = name.len().min(size as usize - 1);
    std::ptr::copy_nonoverlapping(name.as_ptr(), out, n);
    *out.add(n) = 0;
    ERROR_SUCCESS
}

#[no_mangle]
pub unsafe extern "system" fn XUserAreUsersFriends(_user: u32, _xuids: *const u64, _count: u32, result: *mut i32, ov: Ov) -> u32 {
    crate::trace("XUserAreUsersFriends");
    if let Some(r) = result.as_mut() {
        *r = 0;
    }
    complete(ov, ERROR_SUCCESS, 0)
}

#[no_mangle]
pub unsafe extern "system" fn XUserCheckPrivilege(user: u32, privilege: u32, result: *mut i32) -> u32 {
    crate::trace("XUserCheckPrivilege");
    const XUSER_INDEX_ANY: u32 = 0xFF;
    const FRIENDS_ONLY: [u32; 4] = [251, 248, 246, 243];
    let Some(r) = result.as_mut() else { return ERROR_INVALID_PARAMETER };
    if user != LOCAL_USER && user != XUSER_INDEX_ANY {
        *r = 0;
        return ERROR_NOT_LOGGED_ON;
    }
    *r = !FRIENDS_ONLY.contains(&privilege) as i32;
    log!("XUserCheckPrivilege user {user} privilege {privilege} -> {}", *r);
    ERROR_SUCCESS
}

#[no_mangle]
pub unsafe extern "system" fn XUserGetSigninInfo(user: u32, _flags: u32, out: *mut XUserSigninInfo) -> u32 {
    crate::trace("XUserGetSigninInfo");
    let Some(o) = out.as_mut() else { return ERROR_INVALID_PARAMETER };
    if user != LOCAL_USER {
        *o = XUserSigninInfo::default();
        return ERROR_NO_SUCH_USER;
    }
    *o = user::signin_info();
    ERROR_SUCCESS
}

#[no_mangle]
pub unsafe extern "system" fn XUserSetProperty(_user: u32, id: u32, size: u32, value: *const u8) {
    crate::trace("XUserSetProperty");
    user::set_property(id, size, value);
}

#[no_mangle]
pub extern "system" fn XUserSetContext(_user: u32, id: u32, value: u32) {
    crate::trace("XUserSetContext");
    log!("XUserSetContext {id:#x} = {value}");
    user::set_context(id, value);
}

#[no_mangle]
pub unsafe extern "system" fn XUserGetContext(_user: u32, ctx: *mut XUserContext, ov: Ov) -> u32 {
    crate::trace("XUserGetContext");
    let Some(c) = ctx.as_mut() else { return ERROR_INVALID_PARAMETER };
    c.value = user::context(c.id);
    complete(ov, ERROR_SUCCESS, 0)
}

#[no_mangle]
pub unsafe extern "system" fn XUserWriteAchievements(_count: u32, _achievements: *const c_void, ov: Ov) -> u32 {
    crate::trace("XUserWriteAchievements");
    complete(ov, ERROR_SUCCESS, 0)
}

#[no_mangle]
pub unsafe extern "system" fn XUserReadStats(
    _title: u32,
    xuid_count: u32,
    xuids: *const u64,
    spec_count: u32,
    specs: *const crate::stats::Spec,
    size: *mut u32,
    results: *mut u8,
    ov: Ov,
) -> u32 {
    crate::trace("XUserReadStats");
    if size.is_null() || (xuid_count > 0 && xuids.is_null()) || (spec_count > 0 && specs.is_null()) {
        return ERROR_INVALID_PARAMETER;
    }
    let xuids = if xuid_count == 0 { &[][..] } else { std::slice::from_raw_parts(xuids, xuid_count as usize) };
    let specs = if spec_count == 0 { &[][..] } else { std::slice::from_raw_parts(specs, spec_count as usize) };
    match crate::stats::read(xuids, specs, size, results) {
        ERROR_SUCCESS => complete(ov, ERROR_SUCCESS, 0),
        e => e,
    }
}

#[no_mangle]
pub unsafe extern "system" fn XUserCreateStatsEnumeratorByRank(
    _title: u32,
    _rank_start: u32,
    _rows: u32,
    _spec_count: u32,
    _specs: *const c_void,
    size: *mut u32,
    h: *mut usize,
) -> u32 {
    crate::trace("XUserCreateStatsEnumeratorByRank");
    if let Some(s) = size.as_mut() {
        *s = 0;
    }
    if let Some(h) = h.as_mut() {
        *h = usize::MAX;
    }
    1
}

#[no_mangle]
pub unsafe extern "system" fn XUserEstimateRankForRating(_count: u32, _requests: *const c_void, _size: u32, _result: *mut c_void, ov: Ov) -> u32 {
    crate::trace("XUserEstimateRankForRating");
    complete(ov, ERROR_FUNCTION_FAILED, 0)
}

#[no_mangle]
pub unsafe extern "system" fn XUserMuteListQuery(_user: u32, _xuid: u64, muted: *mut i32) -> u32 {
    crate::trace("XUserMuteListQuery");
    if let Some(m) = muted.as_mut() {
        *m = 0;
    }
    0
}

#[no_mangle]
pub unsafe extern "system" fn XUserReadProfileSettings(
    _title: u32,
    user: u32,
    count: u32,
    ids: *const u32,
    size: *mut u32,
    results: *mut u8,
    ov: Ov,
) -> u32 {
    crate::trace("XUserReadProfileSettings");
    if ids.is_null() || size.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    let reqs: Vec<user::SettingReq> =
        std::slice::from_raw_parts(ids, count as usize).iter().map(|&id| (id, (user as u64).to_le_bytes(), user == LOCAL_USER)).collect();
    match user::read_profile(&reqs, size, results) {
        ERROR_SUCCESS => complete(ov, ERROR_SUCCESS, 0),
        e => e,
    }
}

#[no_mangle]
pub unsafe extern "system" fn XUserReadProfileSettingsByXuid(
    _title: u32,
    _requester: u32,
    xuid_count: u32,
    xuids: *const u64,
    count: u32,
    ids: *const u32,
    size: *mut u32,
    results: *mut u8,
    ov: Ov,
) -> u32 {
    crate::trace("XUserReadProfileSettingsByXuid");
    if ids.is_null() || xuids.is_null() || size.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    let ids = std::slice::from_raw_parts(ids, count as usize);
    let mut reqs = vec![];
    for &x in std::slice::from_raw_parts(xuids, xuid_count as usize) {
        reqs.extend(ids.iter().map(|&id| (id, x.to_le_bytes(), x == user::xuid())));
    }
    match user::read_profile(&reqs, size, results) {
        ERROR_SUCCESS => complete(ov, ERROR_SUCCESS, 0),
        e => e,
    }
}

#[no_mangle]
pub unsafe extern "system" fn XUserWriteProfileSettings(_user: u32, count: u32, settings: *const XUserProfileSetting, ov: Ov) -> u32 {
    crate::trace("XUserWriteProfileSettings");
    if !settings.is_null() {
        user::write_profile(std::slice::from_raw_parts(settings, count as usize));
    }
    complete(ov, ERROR_SUCCESS, 0)
}

#[no_mangle]
pub unsafe extern "system" fn XLiveInitializeEx(_info: *mut c_void, version: u32) -> i32 {
    crate::trace("XLiveInitializeEx");
    log!("XLiveInitializeEx (title xlive version {version:#x}) as '{}' xuid {:016x}", CONFIG.name, user::xuid());
    std::sync::LazyLock::force(&crate::mods::MODS);
    crate::startup::apply();
    S_OK
}

#[no_mangle]
pub unsafe extern "system" fn XStringVerify(
    _flags: u32,
    _locale: *const u8,
    count: u32,
    _strings: *const c_void,
    size: u32,
    response: *mut u8,
    ov: Ov,
) -> u32 {
    crate::trace("XStringVerify");
    // STRING_VERIFY_RESPONSE { u16 wNumStrings; HRESULT* pStringResult } + one S_OK per string.
    let need = 8 + 4 * count;
    if response.is_null() || size < need {
        return ERROR_INSUFFICIENT_BUFFER;
    }
    response.cast::<u16>().write(count as u16);
    response.add(4).cast::<u32>().write(response.add(8) as u32);
    std::ptr::write_bytes(response.add(8), 0, 4 * count as usize);
    complete(ov, ERROR_SUCCESS, 0)
}

#[no_mangle]
pub extern "system" fn XOnlineStartup() -> u32 {
    crate::trace("XOnlineStartup");
    0
}

#[no_mangle]
pub extern "system" fn XOnlineCleanup() -> u32 {
    crate::trace("XOnlineCleanup");
    0
}

#[no_mangle]
pub unsafe extern "system" fn XFriendsCreateEnumerator(_user: u32, _start: u32, count: u32, size: *mut u32, h: *mut usize) -> u32 {
    crate::trace("XFriendsCreateEnumerator");
    let (Some(size), Some(h)) = (size.as_mut(), h.as_mut()) else { return ERROR_INVALID_PARAMETER };
    // Room for `count` XONLINE_FRIENDs (under 0x200 bytes each); XEnumerate always returns none.
    *size = count.min(100) * 0x200;
    *h = new_handle();
    0
}

#[no_mangle]
pub extern "system" fn XInviteGetAcceptedInfo(_user: u32, _info: *mut c_void) -> u32 {
    crate::trace("XInviteGetAcceptedInfo");
    1
}

#[no_mangle]
pub extern "system" fn XOnlineGetNatType() -> u32 {
    crate::trace("XOnlineGetNatType");
    1 // XONLINE_NAT_OPEN
}

// ---------------------------------------------------------------- storage / content (none)

unsafe fn progress(pct: *mut u32, num: *mut u64, den: *mut u64) -> u32 {
    // Everything completes immediately.
    if let Some(p) = pct.as_mut() {
        *p = 100;
    }
    for v in [num, den] {
        if let Some(v) = v.as_mut() {
            *v = 1;
        }
    }
    ERROR_SUCCESS
}

#[no_mangle]
pub unsafe extern "system" fn XStorageUploadFromMemoryGetProgress(_ov: Ov, pct: *mut u32, num: *mut u64, den: *mut u64) -> u32 {
    crate::trace("XStorageUploadFromMemoryGetProgress");
    progress(pct, num, den)
}

#[no_mangle]
pub unsafe extern "system" fn XStorageUploadFromMemory(_user: u32, path: *const u16, size: u32, data: *const u8, ov: Ov) -> u32 {
    crate::trace("XStorageUploadFromMemory");
    let Some(path) = storage::wide(path) else { return ERROR_INVALID_PARAMETER };
    if data.is_null() && size > 0 {
        return ERROR_INVALID_PARAMETER;
    }
    let bytes = if size == 0 { &[][..] } else { std::slice::from_raw_parts(data, size as usize) };
    match storage::write(&path, bytes) {
        Ok(()) => complete(ov, ERROR_SUCCESS, 0),
        Err(e) => crate::fail(ov, e),
    }
}

#[no_mangle]
pub unsafe extern "system" fn XStorageEnumerate(_user: u32, path: *const u16, _start: u32, _max: u32, size: u32, results: *mut [u32; 3], ov: Ov) -> u32 {
    crate::trace("XStorageEnumerate");
    // shortcut: always reports no files; list storage\ here if LP2 turns out to enumerate.
    log!("XStorageEnumerate {:?}", storage::wide(path));
    match results.as_mut() {
        Some(r) if size >= 12 => *r = [0, 0, 0],
        _ => return ERROR_INVALID_PARAMETER,
    }
    complete(ov, ERROR_SUCCESS, 0)
}

#[no_mangle]
pub unsafe extern "system" fn XStorageDownloadToMemoryGetProgress(_ov: Ov, pct: *mut u32, num: *mut u64, den: *mut u64) -> u32 {
    crate::trace("XStorageDownloadToMemoryGetProgress");
    progress(pct, num, den)
}

#[no_mangle]
pub unsafe extern "system" fn XStorageDelete(_user: u32, path: *const u16, ov: Ov) -> u32 {
    crate::trace("XStorageDelete");
    let Some(path) = storage::wide(path) else { return ERROR_INVALID_PARAMETER };
    match storage::delete(&path) {
        Ok(()) => complete(ov, ERROR_SUCCESS, 0),
        Err(e) => crate::fail(ov, e),
    }
}

unsafe fn build_path(xuid: u64, facility: u32, item: *const u16, out: *mut u16, len: *mut u32) -> u32 {
    let Some(item) = storage::wide(item).filter(|i| !i.is_empty()) else { return ERROR_INVALID_PARAMETER };
    match storage::server_path(facility, xuid, &item) {
        Ok(p) => {
            log!("XStorageBuildServerPath {item} -> {p}");
            storage::write_wide(&p, out, len)
        }
        Err(e) => e,
    }
}

#[no_mangle]
pub unsafe extern "system" fn XStorageBuildServerPathByXuid(
    xuid: u64,
    facility: u32,
    _info: *const c_void,
    _info_size: u32,
    item: *const u16,
    out: *mut u16,
    out_size: *mut u32,
) -> u32 {
    crate::trace("XStorageBuildServerPathByXuid");
    build_path(xuid, facility, item, out, out_size)
}

#[no_mangle]
pub unsafe extern "system" fn XStorageBuildServerPath(
    user: u32,
    facility: u32,
    _info: *const c_void,
    _info_size: u32,
    item: *const u16,
    out: *mut u16,
    out_size: *mut u32,
) -> u32 {
    crate::trace("XStorageBuildServerPath");
    if user != LOCAL_USER {
        return ERROR_NO_SUCH_USER;
    }
    build_path(user::xuid(), facility, item, out, out_size)
}

#[no_mangle]
pub unsafe extern "system" fn XStorageDownloadToMemory(
    _user: u32,
    path: *const u16,
    size: u32,
    buf: *mut u8,
    info_size: u32,
    info: *mut u8,
    ov: Ov,
) -> u32 {
    crate::trace("XStorageDownloadToMemory");
    let Some(path) = storage::wide(path) else { return ERROR_INVALID_PARAMETER };
    if buf.is_null() || info.is_null() || info_size < 20 {
        return ERROR_INVALID_PARAMETER;
    }
    // XSTORAGE_DOWNLOAD_TO_MEMORY_RESULTS is packed in the GFWL SDK (LP2 passes 20 bytes):
    // { u32 dwBytesTotal; XUID xuidOwner @4; FILETIME ftCreated @12 }
    std::ptr::write_bytes(info, 0, 20);
    match storage::read(&path) {
        Ok(d) => {
            info.cast::<u32>().write(d.len() as u32);
            info.add(4).cast::<u64>().write_unaligned(user::xuid());
            if d.len() > size as usize {
                return crate::fail(ov, 0x8015_C003); // XONLINE_E_STORAGE_FILE_IS_TOO_BIG
            }
            std::ptr::copy_nonoverlapping(d.as_ptr(), buf, d.len());
            log!("XStorage download {path}: {} bytes", d.len());
            complete(ov, ERROR_SUCCESS, d.len() as u32)
        }
        Err(e) => crate::fail(ov, e),
    }
}

#[no_mangle]
pub unsafe extern "system" fn XLiveProtectedCreateFile(
    _content: HANDLE,
    _reserved: *mut c_void,
    _path: *const u16,
    _access: u32,
    _share: u32,
    _security: *mut c_void,
    _disposition: u32,
    _flags: u32,
    out: *mut usize,
) -> i32 {
    crate::trace("XLiveProtectedCreateFile");
    if let Some(o) = out.as_mut() {
        *o = usize::MAX;
    }
    1
}

#[no_mangle]
pub unsafe extern "system" fn XLiveContentCreateAccessHandle(
    _user: u32,
    _info: *mut c_void,
    _license_version: u32,
    _pbuffer: *mut c_void,
    _offset: u32,
    out: *mut usize,
    _ov: Ov,
) -> i32 {
    crate::trace("XLiveContentCreateAccessHandle");
    if let Some(o) = out.as_mut() {
        *o = usize::MAX;
    }
    E_OUTOFMEMORY
}

#[no_mangle]
pub unsafe extern "system" fn XLiveContentGetPath(_user: u32, _info: *mut c_void, path: *mut u16, len: *mut u32) -> i32 {
    crate::trace("XLiveContentGetPath");
    if let Some(l) = len.as_mut() {
        *l = 0;
    }
    if let Some(p) = path.as_mut() {
        *p = 0;
    }
    S_OK
}

#[no_mangle]
pub unsafe extern "system" fn XLiveContentCreateEnumerator(_count: u32, _info: *mut c_void, _size: *mut u32, h: *mut usize) -> u32 {
    crate::trace("XLiveContentCreateEnumerator");
    if let Some(h) = h.as_mut() {
        *h = usize::MAX;
    }
    1
}

// ---------------------------------------------------------------- sessions (5300-5336)

#[no_mangle]
pub unsafe extern "system" fn XSessionCreate(
    flags: u32,
    _user: u32,
    max_public: u32,
    max_private: u32,
    nonce: *mut u64,
    info: *mut XSessionInfo,
    ov: Ov,
    h: *mut usize,
) -> u32 {
    crate::trace("XSessionCreate");
    let (Some(info), Some(h)) = (info.as_mut(), h.as_mut()) else { return ERROR_INVALID_PARAMETER };
    if let Some(n) = nonce.as_mut() {
        *n = random_u64();
    }
    *h = new_handle();
    session::create(*h, flags, max_public, max_private, info);
    complete(ov, ERROR_SUCCESS, 0)
}

#[no_mangle]
pub unsafe extern "system" fn XSessionStart(h: usize, _flags: u32, ov: Ov) -> u32 {
    crate::trace("XSessionStart");
    log!("XSessionStart {h:#x}");
    complete(ov, ERROR_SUCCESS, 0)
}

#[no_mangle]
pub unsafe extern "system" fn XSessionEnd(h: usize, ov: Ov) -> u32 {
    crate::trace("XSessionEnd");
    log!("XSessionEnd {h:#x}");
    complete(ov, ERROR_SUCCESS, 0)
}

#[no_mangle]
pub unsafe extern "system" fn XSessionWriteStats(_h: usize, xuid: u64, count: u32, views: *const crate::stats::ViewProperties, ov: Ov) -> u32 {
    crate::trace("XSessionWriteStats");
    if !views.is_null() {
        crate::stats::log_write(xuid, std::slice::from_raw_parts(views, count as usize));
    }
    complete(ov, ERROR_SUCCESS, 0)
}

#[no_mangle]
pub unsafe extern "system" fn XSessionSearchEx(
    procedure: u32,
    _user: u32,
    max_results: u32,
    _local_users: u32,
    _property_count: u16,
    _context_count: u16,
    _properties: *const XUserProperty,
    _contexts: *const XUserContext,
    size: *mut u32,
    results: *mut u8,
    ov: Ov,
) -> u32 {
    crate::trace("XSessionSearchEx");
    if size.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    log!("XSessionSearchEx procedure {procedure} max {max_results}");
    let found = session::search(None, max_results as usize);
    match session::write_search(&found, size, results) {
        ERROR_SUCCESS => complete(ov, ERROR_SUCCESS, 0),
        e => e,
    }
}

#[no_mangle]
pub unsafe extern "system" fn XSessionSearchByID(id: u64, _user: u32, size: *mut u32, results: *mut u8, ov: Ov) -> u32 {
    crate::trace("XSessionSearchByID");
    if size.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    let found = session::search(Some(id), 1);
    match session::write_search(&found, size, results) {
        ERROR_SUCCESS => complete(ov, ERROR_SUCCESS, 0),
        e => e,
    }
}

#[no_mangle]
pub unsafe extern "system" fn XSessionModify(h: usize, flags: u32, max_public: u32, max_private: u32, ov: Ov) -> u32 {
    crate::trace("XSessionModify");
    let ok = session::update(h, |l| {
        // Only the modifier bits can change after creation.
        l.flags = (l.flags & !0xF80) | (flags & 0xF80);
        l.max_public = max_public;
        l.max_private = max_private;
    });
    complete(ov, if ok { ERROR_SUCCESS } else { ERROR_INVALID_PARAMETER }, 0)
}

#[no_mangle]
pub unsafe extern "system" fn XSessionMigrateHost(h: usize, user: u32, info: *mut XSessionInfo, ov: Ov) -> u32 {
    crate::trace("XSessionMigrateHost");
    let Some(info) = info.as_mut() else { return ERROR_INVALID_PARAMETER };
    // XUSER_INDEX_NONE (0xFE) means another machine becomes host.
    let ok = session::migrate(h, user == LOCAL_USER, info);
    log!("XSessionMigrateHost {h:#x} user {user}");
    complete(ov, if ok { ERROR_SUCCESS } else { ERROR_INVALID_PARAMETER }, 0)
}

unsafe fn join(h: usize, xuids: Vec<u64>, private: *const i32, ov: Ov) -> u32 {
    let ok = session::update(h, |l| {
        for (i, x) in xuids.into_iter().enumerate() {
            let p = !private.is_null() && *private.add(i) != 0;
            if !l.members.iter().any(|m| m.0 == x) {
                l.members.push((x, p));
            }
        }
    });
    complete(ov, if ok { ERROR_SUCCESS } else { ERROR_INVALID_PARAMETER }, 0)
}

unsafe fn leave(h: usize, xuids: Vec<u64>, ov: Ov) -> u32 {
    let ok = session::update(h, |l| l.members.retain(|m| !xuids.contains(&m.0)));
    complete(ov, if ok { ERROR_SUCCESS } else { ERROR_INVALID_PARAMETER }, 0)
}

unsafe fn xuid_list(count: u32, xuids: *const u64) -> Vec<u64> {
    if xuids.is_null() { vec![] } else { std::slice::from_raw_parts(xuids, count as usize).to_vec() }
}

#[no_mangle]
pub unsafe extern "system" fn XSessionJoinLocal(h: usize, count: u32, _users: *const u32, private: *const i32, ov: Ov) -> u32 {
    crate::trace("XSessionJoinLocal");
    join(h, if count > 0 { vec![user::xuid()] } else { vec![] }, private, ov)
}

#[no_mangle]
pub unsafe extern "system" fn XSessionJoinRemote(h: usize, count: u32, xuids: *const u64, private: *const i32, ov: Ov) -> u32 {
    crate::trace("XSessionJoinRemote");
    join(h, xuid_list(count, xuids), private, ov)
}

#[no_mangle]
pub unsafe extern "system" fn XSessionLeaveLocal(h: usize, _count: u32, _users: *const u32, ov: Ov) -> u32 {
    crate::trace("XSessionLeaveLocal");
    leave(h, vec![user::xuid()], ov)
}

#[no_mangle]
pub unsafe extern "system" fn XSessionLeaveRemote(h: usize, count: u32, xuids: *const u64, ov: Ov) -> u32 {
    crate::trace("XSessionLeaveRemote");
    leave(h, xuid_list(count, xuids), ov)
}

#[no_mangle]
pub unsafe extern "system" fn XSessionDelete(h: usize, ov: Ov) -> u32 {
    crate::trace("XSessionDelete");
    log!("XSessionDelete {h:#x}");
    session::delete(h);
    complete(ov, ERROR_SUCCESS, 0)
}

#[no_mangle]
pub unsafe extern "system" fn XSessionArbitrationRegister(h: usize, _flags: u32, _nonce: u64, size: *mut u32, results: *mut u8, ov: Ov) -> u32 {
    crate::trace("XSessionArbitrationRegister");
    if size.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    if !SESSIONS.lock().unwrap().contains_key(&h) {
        return ERROR_INVALID_PARAMETER;
    }
    match session::write_registration(h, size, results) {
        ERROR_SUCCESS => complete(ov, ERROR_SUCCESS, 0),
        e => e,
    }
}

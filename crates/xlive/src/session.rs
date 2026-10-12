//! XSession*: local session bookkeeping, published to / searched on the master server.

use crate::net::NET;
use crate::types::*;
use crate::user::{self, put_property};
use crate::{log, random_u64, CONFIG, ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS};
use lp2_proto::*;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

pub const XSESSION_CREATE_HOST: u32 = 0x01;
pub const XSESSION_CREATE_USES_MATCHMAKING: u32 = 0x08;
const XNET_XNQOSINFO_COMPLETE: u8 = 0x01;
const XNET_XNQOSINFO_TARGET_CONTACTED: u8 = 0x02;
const XNET_XNQOSINFO_DATA_RECEIVED: u8 = 0x08;

pub struct Local {
    pub flags: u32,
    pub info: XSessionInfo,
    pub max_public: u32,
    pub max_private: u32,
    /// (xuid, in a private slot)
    pub members: Vec<(u64, bool)>,
}

impl Local {
    fn published(&self) -> bool {
        self.flags & XSESSION_CREATE_HOST != 0 && self.flags & XSESSION_CREATE_USES_MATCHMAKING != 0
    }

    fn id(&self) -> u64 {
        u64::from_le_bytes(self.info.id)
    }

    fn to_proto(&self) -> Session {
        let filled_private = self.members.iter().filter(|m| m.1).count() as u32;
        let filled_public = self.members.len() as u32 - filled_private;
        let (contexts, properties) = user::snapshot();
        Session {
            id: self.id(),
            host_addr: bytes_of(&self.info.host).to_vec(),
            key: self.info.key.to_vec(),
            open_public: self.max_public.saturating_sub(filled_public),
            open_private: self.max_private.saturating_sub(filled_private),
            filled_public,
            filled_private,
            contexts,
            properties,
            qos: QOS_LISTEN.lock().unwrap().get(&self.id()).cloned().unwrap_or_default(),
            mods: crate::mods::MODS.hash,
            bots: crate::bots::count() as u8,
        }
    }
}

pub static SESSIONS: LazyLock<Mutex<HashMap<usize, Local>>> = LazyLock::new(Mutex::default);
/// Data the host set with XNetQosListen, by session id.
static QOS_LISTEN: LazyLock<Mutex<HashMap<u64, Vec<u8>>>> = LazyLock::new(Mutex::default);
/// Sessions seen in searches, for later QoS lookups.
static SEEN: LazyLock<Mutex<HashMap<u64, Session>>> = LazyLock::new(Mutex::default);
/// XNQOS blocks handed to the game, freed by XNetQosRelease.
static QOS_BLOCKS: LazyLock<Mutex<HashMap<usize, Vec<u64>>>> = LazyLock::new(Mutex::default);

fn bytes_of<T: Copy>(v: &T) -> &[u8] {
    unsafe { std::slice::from_raw_parts(v as *const T as *const u8, size_of::<T>()) }
}

pub fn master_request(req: &Request) -> Option<Response> {
    let master = CONFIG.master?;
    let run = || -> std::io::Result<Response> {
        let s = TcpStream::connect_timeout(&master, Duration::from_secs(3))?;
        s.set_read_timeout(Some(Duration::from_secs(3)))?;
        s.set_write_timeout(Some(Duration::from_secs(3)))?;
        let mut line = serde_json::to_vec(req)?;
        line.push(b'\n');
        (&s).write_all(&line)?;
        let mut resp = String::new();
        BufReader::new((&s).take(MAX_LINE as u64)).read_line(&mut resp)?;
        Ok(serde_json::from_str(&resp)?)
    };
    match run() {
        Ok(Response::Error { msg }) => {
            log!("master refused {req:?}: {msg}");
            None
        }
        Ok(r) => Some(r),
        Err(e) => {
            log!("master {master} unreachable: {e}");
            None
        }
    }
}

/// Publish/remove requests go through one worker so they reach the master in order
/// without blocking the game thread.
fn send_async(req: Request) {
    static WORKER: LazyLock<Mutex<std::sync::mpsc::Sender<Request>>> = LazyLock::new(|| {
        let (tx, rx) = std::sync::mpsc::channel::<Request>();
        std::thread::spawn(move || rx.into_iter().for_each(|r| drop(master_request(&r))));
        Mutex::new(tx)
    });
    let _ = WORKER.lock().unwrap().send(req);
}

fn publish(s: Session) {
    send_async(Request::Publish { session: s });
}

/// Called from the heartbeat: keeps hosted sessions alive on the master, and follows the bot
/// count of a joined one.
pub fn refresh() {
    let (hosted, joined) = {
        let sessions = SESSIONS.lock().unwrap();
        let hosted: Vec<Session> = sessions.values().filter(|l| l.published()).map(Local::to_proto).collect();
        (hosted, sessions.values().find(|l| l.flags & XSESSION_CREATE_HOST == 0).map(Local::id))
    };
    hosted.into_iter().for_each(publish);
    if let Some(Response::Sessions { sessions }) = joined.and_then(|id| master_request(&Request::Get { id })) {
        if let Some(s) = sessions.first() {
            crate::bots::follow(s.bots);
        }
    }
}

/// Applies `f` to a session, then re-publishes it if it is advertised.
pub fn update(h: usize, f: impl FnOnce(&mut Local)) -> bool {
    let s = {
        let mut sessions = SESSIONS.lock().unwrap();
        let Some(l) = sessions.get_mut(&h) else { return false };
        f(l);
        l.published().then(|| l.to_proto())
    };
    if let Some(s) = s {
        publish(s);
    }
    true
}

pub fn create(h: usize, flags: u32, max_public: u32, max_private: u32, info: &mut XSessionInfo) {
    if flags & XSESSION_CREATE_HOST != 0 {
        let mut id = random_u64().to_le_bytes();
        id[0] = (id[0] & 0x0F) | 0x80; // XNET_XNKID_ONLINE_PEER
        let mut key = [0u8; 16];
        key[..8].copy_from_slice(&random_u64().to_le_bytes());
        key[8..].copy_from_slice(&random_u64().to_le_bytes());
        *info = XSessionInfo { id, host: NET.xnaddr(), key };
    }
    log!("XSessionCreate flags {flags:#x} slots {max_public}+{max_private} id {:016x}", u64::from_le_bytes(info.id));
    if let Some(found) = SEEN.lock().unwrap().get(&u64::from_le_bytes(info.id)).filter(|_| flags & XSESSION_CREATE_HOST == 0) {
        crate::bots::follow(found.bots);
    }
    let l = Local { flags, info: *info, max_public, max_private, members: vec![] };
    let publish_now = l.published().then(|| l.to_proto());
    SESSIONS.lock().unwrap().insert(h, l);
    if let Some(s) = publish_now {
        publish(s);
    }
}

pub fn delete(h: usize) {
    let id = {
        let mut sessions = SESSIONS.lock().unwrap();
        let Some(l) = sessions.get_mut(&h) else { return };
        let id = l.published().then(|| l.id());
        l.flags &= !XSESSION_CREATE_HOST;
        id
    };
    if let Some(id) = id {
        send_async(Request::Remove { id });
    }
}

pub fn close(h: usize) {
    delete(h);
    SESSIONS.lock().unwrap().remove(&h);
}

pub fn migrate(h: usize, become_host: bool, info: &mut XSessionInfo) -> bool {
    update(h, |l| {
        if become_host {
            l.flags |= XSESSION_CREATE_HOST;
            l.info.host = NET.xnaddr();
            *info = l.info;
        } else {
            l.flags &= !XSESSION_CREATE_HOST;
            l.info = *info;
        }
    })
}

pub fn set_qos_listen(id: u64, data: Option<Vec<u8>>) {
    {
        let mut q = QOS_LISTEN.lock().unwrap();
        match data {
            Some(d) => q.insert(id, d),
            None => q.remove(&id),
        };
    }
    // The lobby info usually arrives after XSessionCreate; advertise it right away.
    let s = SESSIONS.lock().unwrap().values().find(|l| l.id() == id && l.published()).map(Local::to_proto);
    if let Some(s) = s {
        publish(s);
    }
}

// ---- search ----

/// Sessions from the master (or one by id), excluding ones we host.
pub fn search(id: Option<u64>, max: usize) -> Vec<Session> {
    let req = id.map_or(Request::List, |id| Request::Get { id });
    let Some(Response::Sessions { sessions }) = master_request(&req) else { return vec![] };
    let mine = SESSIONS.lock().unwrap().values().map(Local::id).collect::<Vec<_>>();
    // shortcut: no matchmaking-query filtering (game mode, map); every open lobby is returned.
    let found: Vec<Session> = sessions
        .into_iter()
        // Different tuning would desync a peer-to-peer match.
        .filter(|s| s.validate().is_ok() && !mine.contains(&s.id) && s.mods == crate::mods::MODS.hash)
        .take(max)
        .collect();
    let mut seen = SEEN.lock().unwrap();
    for s in &found {
        seen.insert(s.id, s.clone());
    }
    log!("session search: {} result(s)", found.len());
    found
}

fn search_results(out: &mut Out, sessions: &[Session]) {
    let header = out.put(CountPtr::default());
    let arr = out.array::<XSessionSearchResult>(sessions.len());
    for (i, s) in sessions.iter().enumerate() {
        let mut info = XSessionInfo { id: s.id.to_le_bytes(), ..Default::default() };
        unsafe {
            std::ptr::copy_nonoverlapping(s.host_addr.as_ptr(), &mut info.host as *mut XnAddr as *mut u8, 36);
        }
        info.key.copy_from_slice(&s.key);
        let props = out.array::<XUserProperty>(s.properties.len());
        for (j, p) in s.properties.iter().enumerate() {
            put_property(out, props, j, p);
        }
        let ctxs = out.array::<XUserContext>(s.contexts.len());
        for (j, &(id, value)) in s.contexts.iter().enumerate() {
            unsafe { set(ctxs, j, XUserContext { id, value }) };
        }
        let r = XSessionSearchResult {
            info,
            open_public: s.open_public,
            open_private: s.open_private,
            filled_public: s.filled_public,
            filled_private: s.filled_private,
            property_count: s.properties.len() as u32,
            context_count: s.contexts.len() as u32,
            properties: props as u32,
            contexts: ctxs as u32,
        };
        unsafe { set(arr, i, r) };
    }
    unsafe { set(header, 0, CountPtr { count: sessions.len() as u32, ptr: arr as u32 }) };
}

/// Fills an XSESSION_SEARCHRESULT_HEADER, or reports the size needed.
pub unsafe fn write_search(sessions: &[Session], size: *mut u32, buf: *mut u8) -> u32 {
    let mut m = Out::measure();
    search_results(&mut m, sessions);
    if buf.is_null() || *size < m.len as u32 {
        *size = m.len as u32;
        return ERROR_INSUFFICIENT_BUFFER;
    }
    search_results(&mut Out::new(buf), sessions);
    ERROR_SUCCESS
}

// ---- QoS ----

fn qos_data(id: u64) -> Option<Vec<u8>> {
    if let Some(d) = QOS_LISTEN.lock().unwrap().get(&id) {
        return Some(d.clone());
    }
    if let Some(s) = SEEN.lock().unwrap().get(&id) {
        return Some(s.qos.clone());
    }
    search(Some(id), 1).pop().map(|s| s.qos)
}

/// Builds an XNQOS for the given session ids. Every target counts as reachable with a
/// nominal ping; the real latency is not measured.
pub fn qos_lookup(ids: &[Option<u64>]) -> usize {
    let datas: Vec<Option<Vec<u8>>> = ids.iter().map(|id| id.and_then(qos_data)).collect();
    let fill = |out: &mut Out| {
        let header = out.put(CountPtr { count: ids.len() as u32, ptr: 0 }); // ptr = cxnqosPending
        let arr = out.array::<XnQosInfo>(ids.len());
        for (i, d) in datas.iter().enumerate() {
            let d = d.as_deref().unwrap_or(&[]);
            let mut q = XnQosInfo {
                flags: XNET_XNQOSINFO_COMPLETE | XNET_XNQOSINFO_TARGET_CONTACTED,
                probes_sent: 4,
                probes_received: 4,
                rtt_min: 40,
                rtt_median: 50,
                up_bps: 4_000_000,
                down_bps: 4_000_000,
                ..Default::default()
            };
            if !d.is_empty() {
                q.flags |= XNET_XNQOSINFO_DATA_RECEIVED;
                q.data_size = d.len() as u16;
                q.data = out.bytes(d, 1) as u32;
            }
            unsafe { set(arr, i, q) };
        }
        header
    };
    let mut m = Out::measure();
    fill(&mut m);
    let mut block = vec![0u64; m.len / 8 + 1];
    let ptr = block.as_mut_ptr() as usize;
    fill(&mut Out::new(ptr as *mut u8));
    QOS_BLOCKS.lock().unwrap().insert(ptr, block);
    ptr
}

pub fn qos_release(ptr: usize) -> bool {
    QOS_BLOCKS.lock().unwrap().remove(&ptr).is_some()
}

// ---- arbitration ----

fn registration(out: &mut Out, xuids: &[u64]) {
    let header = out.put(CountPtr::default());
    let arr = out.array::<XSessionRegistrant>(xuids.len());
    for (i, x) in xuids.iter().enumerate() {
        let users = out.put(*x) as u32;
        // shortcut: one machine per player; we do not know which players share a PC.
        unsafe { set(arr, i, XSessionRegistrant { machine_id: *x, trustworthiness: 1, user_count: 1, users }) };
    }
    unsafe { set(header, 0, CountPtr { count: xuids.len() as u32, ptr: arr as u32 }) };
}

pub unsafe fn write_registration(h: usize, size: *mut u32, buf: *mut u8) -> u32 {
    let xuids: Vec<u64> = SESSIONS.lock().unwrap().get(&h).map(|l| l.members.iter().map(|m| m.0).collect()).unwrap_or_default();
    let mut m = Out::measure();
    registration(&mut m, &xuids);
    if buf.is_null() || *size < m.len as u32 {
        *size = m.len as u32;
        return ERROR_INSUFFICIENT_BUFFER;
    }
    registration(&mut Out::new(buf), &xuids);
    ERROR_SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_buffer_layout() {
        let s = Session {
            id: 0x80AB,
            host_addr: vec![7; 36],
            key: vec![1; 16],
            open_public: 3,
            contexts: vec![(0x800A, 1)],
            properties: vec![Property { id: 0x4000_0001, kind: 4, data: vec![b'h', 0, 0, 0] }],
            qos: vec![9, 9],
            ..Default::default()
        };
        let mut size = 0;
        assert_eq!(unsafe { write_search(&[s.clone()], &mut size, std::ptr::null_mut()) }, ERROR_INSUFFICIENT_BUFFER);
        let mut buf = vec![0u64; size as usize / 8 + 1];
        assert_eq!(unsafe { write_search(&[s], &mut size, buf.as_mut_ptr() as _) }, ERROR_SUCCESS);
        unsafe {
            let h = *(buf.as_ptr() as *const CountPtr);
            let r = *(h.ptr as *const XSessionSearchResult);
            assert_eq!((h.count, u64::from_le_bytes(r.info.id), r.open_public), (1, 0x80AB, 3));
            assert_eq!(r.info.host.ina, [7; 4]);
            assert_eq!(*(r.contexts as *const XUserContext), XUserContext { id: 0x800A, value: 1 });
            let p = *(r.properties as *const XUserProperty);
            let str_ptr = u32::from_le_bytes(p.data.value[4..].try_into().unwrap()) as *const u8;
            assert_eq!(*str_ptr, b'h');
            assert!((str_ptr as usize) < buf.as_ptr() as usize + size as usize);
        }
    }

    #[test]
    fn qos_block_has_data() {
        set_qos_listen(42, Some(vec![1, 2, 3]));
        let p = qos_lookup(&[Some(42)]);
        unsafe {
            let h = *(p as *const CountPtr);
            let q = *((p + 8) as *const XnQosInfo);
            assert_eq!((h.count, q.data_size, *(q.data as *const u8)), (1, 3, 1));
        }
        assert!(qos_release(p));
    }
}

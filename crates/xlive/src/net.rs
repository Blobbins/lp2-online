//! XNet: the game's UDP/VDP sockets are virtual. Every packet travels over one real UDP socket,
//! wrapped in a `lp2_proto` DATA header naming source/destination instance and virtual port.
//!
//! Each running game is an "instance" with a random id that doubles as the fake IPv4 address
//! (10.x.y.z) the game sees for that player. XNADDRs carry the id plus how to reach the player.

use crate::types::XnAddr;
use crate::{log, CONFIG};
use lp2_proto::*;
use std::collections::{HashMap, VecDeque};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::sync::{Condvar, LazyLock, Mutex};
use std::time::Duration;

pub const VSOCK_BASE: usize = 0x4C32_0000;
const QUEUE_MAX: usize = 512;
const XNADDR_MAGIC: &[u8; 4] = b"LP2N";
pub const LOOPBACK: u32 = u32::from_ne_bytes([127, 0, 0, 1]);
pub const BROADCAST: u32 = u32::MAX;

pub const WSAEWOULDBLOCK: i32 = 10035;
pub const WSAENOTSOCK: i32 = 10038;
pub const WSAEMSGSIZE: i32 = 10040;
pub const WSAEADDRINUSE: i32 = 10048;
pub const WSAENOTCONN: i32 = 10057;
pub const WSAEINVAL: i32 = 10022;

pub struct Packet {
    pub src: u32,
    pub src_port: u16,
    pub data: Vec<u8>,
}

#[derive(Default)]
pub struct VSock {
    /// Host byte order; 0 = not bound yet.
    pub port: u16,
    pub nonblocking: bool,
    pub queue: VecDeque<Packet>,
    pub peer: Option<(u32, u16)>,
}

#[derive(Default)]
pub struct VState {
    next: usize,
    pub socks: HashMap<usize, VSock>,
}

impl VState {
    fn port_free(&self, port: u16) -> bool {
        !self.socks.values().any(|s| s.port == port)
    }

    fn ephemeral(&self) -> u16 {
        (49152..=u16::MAX).find(|p| self.port_free(*p)).unwrap_or(0)
    }

    fn ensure_bound(&mut self, h: usize) -> Result<u16, i32> {
        let port = self.ephemeral();
        let s = self.socks.get_mut(&h).ok_or(WSAENOTSOCK)?;
        if s.port == 0 {
            s.port = port;
        }
        Ok(s.port)
    }
}

pub struct Net {
    sock: UdpSocket,
    /// Our fake IN_ADDR, as raw `s_addr`.
    pub inst: u32,
    pub local: SocketAddrV4,
    pub public: Mutex<Option<SocketAddrV4>>,
    /// The master in use: lp2net.ini's, or one from the directory (switched to the joined lobby's).
    master: Mutex<Option<SocketAddr>>,
    /// instance -> real endpoint (direct, or the master when relayed).
    peers: Mutex<HashMap<u32, SocketAddr>>,
    pub vs: Mutex<VState>,
    pub cv: Condvar,
}

pub static NET: LazyLock<Net> = LazyLock::new(Net::start);

pub fn ip_str(raw: u32) -> Ipv4Addr {
    Ipv4Addr::from(raw.to_ne_bytes())
}

/// The listed server that answers fastest (no ini `master`): where we publish lobbies we host.
fn home_master() -> Option<SocketAddr> {
    let found = crate::session::directory_masters();
    let (tx, rx) = std::sync::mpsc::channel();
    for m in found {
        let tx = tx.clone();
        std::thread::spawn(move || {
            if std::net::TcpStream::connect_timeout(&m, Duration::from_secs(2)).is_ok() {
                let _ = tx.send(m);
            }
        });
    }
    drop(tx);
    let best = rx.recv_timeout(Duration::from_secs(3)).ok();
    log!("master: {}", best.map_or("no server online in the directory".into(), |m| format!("{m} (fastest listed server)")));
    best
}

fn local_ip(toward: Option<SocketAddr>) -> Ipv4Addr {
    // Connecting a UDP socket sends nothing; it just picks the outgoing interface.
    let probe = || -> std::io::Result<Ipv4Addr> {
        let s = UdpSocket::bind("0.0.0.0:0")?;
        s.connect(toward.unwrap_or_else(|| "8.8.8.8:53".parse().unwrap()))?;
        match s.local_addr()?.ip() {
            std::net::IpAddr::V4(ip) => Ok(ip),
            _ => Ok(Ipv4Addr::LOCALHOST),
        }
    };
    probe().unwrap_or(Ipv4Addr::LOCALHOST)
}

impl Net {
    fn start() -> Net {
        let cfg = &*CONFIG;
        let sock = UdpSocket::bind(("0.0.0.0", cfg.port))
            .or_else(|e| {
                log!("UDP port {} unavailable ({e}); using a random port", cfg.port);
                UdpSocket::bind("0.0.0.0:0")
            })
            .expect("bind UDP socket");
        let local = SocketAddrV4::new(local_ip(cfg.master), sock.local_addr().map(|a| a.port()).unwrap_or(0));
        let r = crate::random_u64().to_le_bytes();
        let net = Net {
            sock,
            inst: u32::from_ne_bytes([10, r[0], r[1], r[2].max(1)]),
            local,
            public: Mutex::new(None),
            master: Mutex::new(cfg.master.or_else(home_master)),
            peers: Mutex::default(),
            vs: Mutex::default(),
            cv: Condvar::new(),
        };
        net.first_hello();
        log!("net up: instance {} local {} public {:?} master {:?} direct {}",
            ip_str(net.inst), net.local, *net.public.lock().unwrap(), net.master(), cfg.direct);
        if cfg.relay {
            log!("relay = true: every peer goes through the master");
        }
        // These block on NET until start() returns.
        std::thread::spawn(|| NET.recv_loop());
        crate::start_heartbeat();
        net
    }

    /// Learn our public endpoint before the game asks for its XNADDR.
    fn first_hello(&self) {
        let Some(master) = self.master() else { return };
        let _ = self.sock.set_read_timeout(Some(Duration::from_millis(700)));
        let mut buf = [0u8; 64];
        for _ in 0..3 {
            self.hello();
            if let Ok((n, from)) = self.sock.recv_from(&mut buf) {
                if from == master && self.on_hello_ack(&buf[..n]) {
                    break;
                }
            }
        }
        let _ = self.sock.set_read_timeout(None);
        if self.public.lock().unwrap().is_none() {
            log!("master {master} did not answer; online play needs it reachable on UDP");
        }
    }

    pub fn master(&self) -> Option<SocketAddr> {
        *self.master.lock().unwrap()
    }

    /// Moves to another master (the server listing a lobby we join): register there so its relay
    /// knows us. Routes learned through the old master's relay are dropped.
    pub fn set_master(&self, m: SocketAddr) {
        let old = self.master.lock().unwrap().replace(m);
        if old != Some(m) {
            log!("master: switching to {m}");
            self.peers.lock().unwrap().retain(|_, a| Some(*a) != old);
            (0..3).for_each(|_| self.hello());
        }
    }

    pub fn hello(&self) {
        if let Some(m) = self.master() {
            let mut p = MAGIC.to_vec();
            p.push(PKT_HELLO);
            p.extend_from_slice(&self.inst.to_le_bytes());
            let _ = self.sock.send_to(&p, m);
        }
    }

    fn on_hello_ack(&self, p: &[u8]) -> bool {
        if p.len() < 9 || p[..2] != MAGIC || p[2] != PKT_HELLO_ACK {
            return false;
        }
        let ip = Ipv4Addr::new(p[3], p[4], p[5], p[6]);
        let port = u16::from_be_bytes([p[7], p[8]]);
        *self.public.lock().unwrap() = Some(SocketAddrV4::new(ip, port));
        true
    }

    fn recv_loop(&self) {
        let mut buf = vec![0u8; 65536];
        loop {
            match self.sock.recv_from(&mut buf) {
                Ok((n, from)) => self.on_packet(&buf[..n], from),
                // Windows reports ICMP port-unreachable as WSAECONNRESET on the next receive.
                Err(_) => std::thread::sleep(Duration::from_millis(1)),
            }
        }
    }

    fn on_packet(&self, p: &[u8], from: SocketAddr) {
        if Some(from) == self.master() && self.on_hello_ack(p) {
            return;
        }
        let Some((src, dst, src_port, dst_port, payload)) = parse_data(p) else { return };
        if dst != self.inst {
            return;
        }
        {
            // Learn the return path; never replace a direct route with the relay.
            let mut peers = self.peers.lock().unwrap();
            if Some(from) != self.master() || !peers.contains_key(&src) {
                peers.insert(src, from);
            }
        }
        self.deliver(src, src_port, dst_port, payload);
    }

    fn deliver(&self, src: u32, src_port: u16, dst_port: u16, data: &[u8]) {
        let mut vs = self.vs.lock().unwrap();
        if let Some(s) = vs.socks.values_mut().find(|s| s.port == dst_port) {
            if s.queue.len() >= QUEUE_MAX {
                s.queue.pop_front();
            }
            s.queue.push_back(Packet { src, src_port, data: data.to_vec() });
            self.cv.notify_all();
        }
    }

    // ---- virtual sockets ----

    pub fn is_virtual(h: usize) -> bool {
        (VSOCK_BASE..VSOCK_BASE + 0x10_0000).contains(&h)
    }

    pub fn create(&self) -> usize {
        let mut vs = self.vs.lock().unwrap();
        vs.next += 1;
        let h = VSOCK_BASE + vs.next * 4;
        vs.socks.insert(h, VSock::default());
        h
    }

    pub fn close(&self, h: usize) -> Result<(), i32> {
        let removed = self.vs.lock().unwrap().socks.remove(&h);
        self.cv.notify_all();
        removed.map(|_| ()).ok_or(WSAENOTSOCK)
    }

    pub fn bind(&self, h: usize, port: u16) -> Result<(), i32> {
        let mut vs = self.vs.lock().unwrap();
        if !vs.socks.contains_key(&h) {
            return Err(WSAENOTSOCK);
        }
        if port == 0 {
            return vs.ensure_bound(h).map(|_| ());
        }
        if !vs.port_free(port) {
            return Err(WSAEADDRINUSE);
        }
        vs.socks.get_mut(&h).unwrap().port = port;
        log!("vsock {h:#x} bound to port {port}");
        Ok(())
    }

    pub fn with<R>(&self, h: usize, f: impl FnOnce(&mut VSock) -> R) -> Result<R, i32> {
        self.vs.lock().unwrap().socks.get_mut(&h).map(f).ok_or(WSAENOTSOCK)
    }

    pub fn send_to(&self, h: usize, dst: u32, dst_port: u16, data: &[u8]) -> Result<usize, i32> {
        let src_port = self.vs.lock().unwrap().ensure_bound(h)?;
        if dst == self.inst || dst == LOOPBACK {
            self.deliver(self.inst, src_port, dst_port, data);
            return Ok(data.len());
        }
        if dst == BROADCAST {
            // shortcut: no System Link LAN discovery; online sessions go through the master instead.
            return Ok(data.len());
        }
        let Some(to) = self.peers.lock().unwrap().get(&dst).copied() else {
            log!("no route to {}:{dst_port}; dropping", ip_str(dst));
            return Ok(data.len());
        };
        let mut p = data_header(self.inst, dst, src_port, dst_port).to_vec();
        p.extend_from_slice(data);
        match self.sock.send_to(&p, to) {
            Ok(_) => Ok(data.len()),
            Err(e) => {
                log!("send to {to} failed: {e}");
                Ok(data.len())
            }
        }
    }

    /// Pops one packet into `buf`: `(bytes, src instance, src port)`.
    pub fn recv_from(&self, h: usize, buf: &mut [u8]) -> Result<(usize, u32, u16), i32> {
        let mut vs = self.vs.lock().unwrap();
        loop {
            let s = vs.socks.get_mut(&h).ok_or(WSAENOTSOCK)?;
            if let Some(p) = s.queue.pop_front() {
                let n = p.data.len().min(buf.len());
                buf[..n].copy_from_slice(&p.data[..n]);
                return if n < p.data.len() { Err(WSAEMSGSIZE) } else { Ok((n, p.src, p.src_port)) };
            }
            if s.nonblocking {
                return Err(WSAEWOULDBLOCK);
            }
            vs = self.cv.wait(vs).unwrap();
        }
    }

    // ---- addresses ----

    pub fn xnaddr(&self) -> XnAddr {
        let public = self.public.lock().unwrap().unwrap_or(self.local);
        let inst = self.inst.to_le_bytes();
        let mut online = [0u8; 20];
        online[..4].copy_from_slice(&inst);
        online[4..8].copy_from_slice(XNADDR_MAGIC);
        online[8..10].copy_from_slice(&self.local.port().to_le_bytes());
        online[10] = CONFIG.direct as u8;
        XnAddr {
            ina: self.local.ip().octets(),
            ina_online: public.ip().octets(),
            port_online: public.port().to_be_bytes(),
            enet: [0x02, 0x4C, inst[0], inst[1], inst[2], inst[3]],
            online,
        }
    }

    /// Where to send packets for the player at `a`, or `None` if it is not one of ours.
    fn route_for(&self, a: &XnAddr) -> Option<(u32, Option<SocketAddr>)> {
        if &a.online[4..8] != XNADDR_MAGIC {
            return None;
        }
        let inst = u32::from_le_bytes(a.online[..4].try_into().unwrap());
        let my_public = self.public.lock().unwrap().map(|p| p.ip().octets());
        let direct = if CONFIG.relay {
            None
        } else if Some(a.ina_online) == my_public {
            // Same public IP: same LAN, use the private address.
            Some(SocketAddr::from((a.ina, u16::from_le_bytes([a.online[8], a.online[9]]))))
        } else if a.online[10] == 1 {
            Some(SocketAddr::from((a.ina_online, u16::from_be_bytes(a.port_online))))
        } else {
            None
        };
        Some((inst, direct.or(self.master())))
    }

    /// XNetXnAddrToInAddr: registers the route and returns the fake address.
    pub fn register(&self, a: &XnAddr) -> Option<u32> {
        let (inst, route) = self.route_for(a)?;
        if inst != self.inst {
            if let Some(r) = route {
                self.peers.lock().unwrap().entry(inst).or_insert(r);
            }
        }
        Some(inst)
    }

    /// Real IP for a fake address, for the game's TCP sockets (which need a direct path).
    pub fn real_ip(&self, inst: u32) -> Option<Ipv4Addr> {
        match self.peers.lock().unwrap().get(&inst)? {
            SocketAddr::V4(a) if Some(SocketAddr::V4(*a)) != self.master() => Some(*a.ip()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vstate_ports() {
        let mut vs = VState::default();
        vs.socks.insert(1, VSock { port: 49152, ..Default::default() });
        vs.socks.insert(2, VSock::default());
        assert_eq!(vs.ensure_bound(2), Ok(49153));
        assert_eq!(vs.ensure_bound(3), Err(WSAENOTSOCK));
    }

    #[test]
    fn loopback_send_and_xnaddr_route() {
        let net = &*NET;
        let a = net.create();
        let b = net.create();
        net.bind(a, 1000).unwrap();
        assert_eq!(net.bind(b, 1000), Err(WSAEADDRINUSE));
        net.with(a, |s| s.nonblocking = true).unwrap();
        assert_eq!(net.recv_from(a, &mut [0; 8]), Err(WSAEWOULDBLOCK));
        net.send_to(b, net.inst, 1000, b"ping").unwrap();
        let mut buf = [0u8; 8];
        let (n, src, _) = net.recv_from(a, &mut buf).unwrap();
        assert_eq!((&buf[..n], src), (&b"ping"[..], net.inst));
        assert_eq!(net.register(&net.xnaddr()), Some(net.inst));
        assert_eq!(net.register(&XnAddr::default()), None);
        net.close(a).unwrap();
        net.close(b).unwrap();
    }
}

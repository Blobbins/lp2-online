//! Lost Planet 2 master server: a session browser (TCP) and packet relay (UDP) on one port.
//! It is not a game server: the hosting player's game runs the match. Nothing is written to disk.
//!
//! Usage: `lp2-master [port]` (default 27900). Open that port for both TCP and UDP.

use lp2_proto::*;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const PEER_TTL: Duration = Duration::from_secs(60);
const SESSION_TTL: Duration = Duration::from_secs(30);
const MAX_PEERS: usize = 10_000;
const MAX_SESSIONS: usize = 1_000;

#[derive(Default)]
struct State {
    /// instance id -> (public endpoint, last hello)
    peers: HashMap<u32, (SocketAddr, Instant)>,
    /// session id -> (session, publisher ip, last publish)
    sessions: HashMap<u64, (Session, IpAddr, Instant)>,
}

impl State {
    fn purge(&mut self) {
        self.peers.retain(|_, (_, t)| t.elapsed() < PEER_TTL);
        self.sessions.retain(|_, (_, _, t)| t.elapsed() < SESSION_TTL);
    }

    /// HELLO: claim `inst` for `from` unless someone else holds it and is still alive.
    fn hello(&mut self, inst: u32, from: SocketAddr) -> bool {
        match self.peers.get(&inst) {
            Some((addr, t)) if *addr != from && t.elapsed() < PEER_TTL => return false,
            None if self.peers.len() >= MAX_PEERS => {
                self.purge();
                if self.peers.len() >= MAX_PEERS {
                    return false;
                }
            }
            _ => {}
        }
        self.peers.insert(inst, (from, Instant::now()));
        true
    }

    /// Where to forward a DATA packet, only for senders that registered from this exact endpoint.
    fn route(&self, src: u32, dst: u32, from: SocketAddr) -> Option<SocketAddr> {
        let (src_addr, _) = self.peers.get(&src)?;
        if *src_addr != from {
            return None;
        }
        self.peers.get(&dst).map(|(a, _)| *a)
    }

    fn handle(&mut self, req: Request, from: IpAddr) -> Response {
        self.purge();
        match req {
            Request::Publish { session } => {
                if let Err(e) = session.validate() {
                    return Response::Error { msg: e.into() };
                }
                match self.sessions.get(&session.id) {
                    Some((_, owner, _)) if *owner != from => {
                        return Response::Error { msg: "session owned by another host".into() }
                    }
                    None if self.sessions.len() >= MAX_SESSIONS => {
                        return Response::Error { msg: "server full".into() }
                    }
                    _ => {}
                }
                self.sessions.insert(session.id, (session, from, Instant::now()));
                Response::Ok
            }
            Request::Remove { id } => {
                if matches!(self.sessions.get(&id), Some((_, owner, _)) if *owner == from) {
                    self.sessions.remove(&id);
                }
                Response::Ok
            }
            Request::List => Response::Sessions {
                sessions: self.sessions.values().map(|(s, _, _)| s.clone()).collect(),
            },
            Request::Get { id } => Response::Sessions {
                sessions: self.sessions.get(&id).map(|(s, _, _)| s.clone()).into_iter().collect(),
            },
        }
    }
}

fn hello_ack(from: SocketAddr) -> Option<Vec<u8>> {
    let SocketAddr::V4(v4) = from else { return None };
    let mut p = MAGIC.to_vec();
    p.push(PKT_HELLO_ACK);
    p.extend_from_slice(&v4.ip().octets());
    p.extend_from_slice(&v4.port().to_be_bytes());
    Some(p)
}

fn udp_loop(sock: UdpSocket, state: Arc<Mutex<State>>) {
    let mut buf = vec![0u8; 65536];
    loop {
        let Ok((n, from)) = sock.recv_from(&mut buf) else { continue };
        let p = &buf[..n];
        if n < 7 || p[..2] != MAGIC {
            continue;
        }
        match p[2] {
            PKT_HELLO => {
                let inst = u32::from_le_bytes(p[3..7].try_into().unwrap());
                let ok = state.lock().unwrap().hello(inst, from);
                if let (true, Some(ack)) = (ok, hello_ack(from)) {
                    let _ = sock.send_to(&ack, from);
                }
            }
            PKT_DATA => {
                let Some((src, dst, ..)) = parse_data(p) else { continue };
                let to = state.lock().unwrap().route(src, dst, from);
                if let Some(to) = to {
                    let _ = sock.send_to(p, to);
                }
            }
            _ => {}
        }
    }
}

fn serve_tcp(stream: TcpStream, state: &Mutex<State>) -> std::io::Result<()> {
    let from = stream.peer_addr()?.ip();
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut line = String::new();
    BufReader::new((&stream).take(MAX_LINE as u64)).read_line(&mut line)?;
    let resp = match serde_json::from_str::<Request>(&line) {
        Ok(req) => state.lock().unwrap().handle(req, from),
        Err(e) => Response::Error { msg: e.to_string() },
    };
    let mut out = serde_json::to_vec(&resp)?;
    out.push(b'\n');
    (&stream).write_all(&out)
}

fn main() -> std::io::Result<()> {
    let port: u16 = match std::env::args().nth(1) {
        Some(a) => a.parse().expect("usage: lp2-master [port]"),
        None => DEFAULT_MASTER_PORT,
    };
    let state = Arc::new(Mutex::new(State::default()));
    let udp = UdpSocket::bind(("0.0.0.0", port))?;
    let tcp = TcpListener::bind(("0.0.0.0", port))?;
    println!("lp2-master listening on TCP+UDP port {port}");

    let s = state.clone();
    thread::spawn(move || udp_loop(udp, s));
    for stream in tcp.incoming().flatten() {
        let s = state.clone();
        thread::spawn(move || {
            if let Err(e) = serve_tcp(stream, &s) {
                eprintln!("tcp client: {e}");
            }
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    fn session(id: u64) -> Session {
        Session { id, host_addr: vec![0; 36], key: vec![0; 16], ..Default::default() }
    }

    #[test]
    fn hello_cannot_steal_live_instance() {
        let mut st = State::default();
        assert!(st.hello(5, addr("1.1.1.1:1")));
        assert!(!st.hello(5, addr("2.2.2.2:2")));
        assert!(st.hello(5, addr("1.1.1.1:1")));
    }

    #[test]
    fn route_requires_registered_sender() {
        let mut st = State::default();
        st.hello(1, addr("1.1.1.1:1"));
        st.hello(2, addr("2.2.2.2:2"));
        assert_eq!(st.route(1, 2, addr("1.1.1.1:1")), Some(addr("2.2.2.2:2")));
        assert_eq!(st.route(1, 2, addr("9.9.9.9:9")), None);
        assert_eq!(st.route(1, 3, addr("1.1.1.1:1")), None);
    }

    #[test]
    fn only_owner_can_change_session() {
        let mut st = State::default();
        let a: IpAddr = "1.1.1.1".parse().unwrap();
        let b: IpAddr = "2.2.2.2".parse().unwrap();
        assert_eq!(st.handle(Request::Publish { session: session(9) }, a), Response::Ok);
        assert!(matches!(st.handle(Request::Publish { session: session(9) }, b), Response::Error { .. }));
        st.handle(Request::Remove { id: 9 }, b);
        let Response::Sessions { sessions } = st.handle(Request::List, b) else { panic!() };
        assert_eq!(sessions.len(), 1);
        st.handle(Request::Remove { id: 9 }, a);
        let Response::Sessions { sessions } = st.handle(Request::List, a) else { panic!() };
        assert!(sessions.is_empty());
    }
}

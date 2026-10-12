//! Wire format shared by the xlive.dll replacement and the master server.
//!
//! UDP (game traffic + presence), every packet starts with `MAGIC` and a kind byte:
//! * `PKT_HELLO`     client -> master: `inst u32`. Registers/keeps alive the client's public endpoint.
//! * `PKT_HELLO_ACK` master -> client: `ip [u8;4], port u16 BE` as the master saw it.
//! * `PKT_DATA`      peer <-> peer or via master: `src_inst u32, dst_inst u32, src_port u16, dst_port u16, payload`.
//!   The master forwards any DATA packet to the endpoint registered for `dst_inst`.
//!
//! TCP (session browser): one JSON `Request` line in, one JSON `Response` line out, then close.

use serde::{Deserialize, Serialize};

pub const MAGIC: [u8; 2] = *b"L2";
pub const PKT_DATA: u8 = 0;
pub const PKT_HELLO: u8 = 1;
pub const PKT_HELLO_ACK: u8 = 2;
pub const DATA_HEADER: usize = 3 + 4 + 4 + 2 + 2;
pub const DEFAULT_MASTER_PORT: u16 = 27900;

/// Largest JSON line either side accepts.
pub const MAX_LINE: usize = 1 << 20;
pub const MAX_QOS: usize = 4096;
pub const MAX_PROPS: usize = 64;
pub const MAX_PROP_BYTES: usize = 1024;

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Property {
    pub id: u32,
    /// XUSER_DATA_TYPE_*.
    pub kind: u8,
    /// Raw value bytes: little-endian number, UTF-16LE string (with NUL) or blob.
    pub data: Vec<u8>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Session {
    /// XNKID, little-endian.
    pub id: u64,
    /// 36-byte XNADDR of the host.
    pub host_addr: Vec<u8>,
    /// 16-byte XNKEY.
    pub key: Vec<u8>,
    pub open_public: u32,
    pub open_private: u32,
    pub filled_public: u32,
    pub filled_private: u32,
    pub contexts: Vec<(u32, u32)>,
    pub properties: Vec<Property>,
    /// Data the host set with XNetQosListen (LP2 puts lobby info here).
    pub qos: Vec<u8>,
    /// Hash of the host's lp2mods.ini overrides (0 = unmodded). Only equal sets can play together.
    #[serde(default)]
    pub mods: u64,
    /// AI bots the host has in the lobby; joining players add the same ones.
    #[serde(default)]
    pub bots: u8,
}

impl Session {
    /// Rejects what a hostile client could use to bloat the master or crash a reader.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.host_addr.len() != 36 || self.key.len() != 16 {
            return Err("bad address or key size");
        }
        if self.qos.len() > MAX_QOS {
            return Err("qos data too large");
        }
        if self.properties.len() > MAX_PROPS || self.contexts.len() > MAX_PROPS {
            return Err("too many properties");
        }
        if self.properties.iter().any(|p| p.data.len() > MAX_PROP_BYTES) {
            return Err("property too large");
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "op")]
pub enum Request {
    Publish { session: Session },
    Remove { id: u64 },
    List,
    Get { id: u64 },
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "op")]
pub enum Response {
    Ok,
    Sessions { sessions: Vec<Session> },
    Error { msg: String },
}

pub fn data_header(src_inst: u32, dst_inst: u32, src_port: u16, dst_port: u16) -> [u8; DATA_HEADER] {
    let mut h = [0u8; DATA_HEADER];
    h[..2].copy_from_slice(&MAGIC);
    h[2] = PKT_DATA;
    h[3..7].copy_from_slice(&src_inst.to_le_bytes());
    h[7..11].copy_from_slice(&dst_inst.to_le_bytes());
    h[11..13].copy_from_slice(&src_port.to_le_bytes());
    h[13..15].copy_from_slice(&dst_port.to_le_bytes());
    h
}

/// `(src_inst, dst_inst, src_port, dst_port, payload)` of a DATA packet.
pub fn parse_data(p: &[u8]) -> Option<(u32, u32, u16, u16, &[u8])> {
    if p.len() < DATA_HEADER || p[..2] != MAGIC || p[2] != PKT_DATA {
        return None;
    }
    let u32_at = |i: usize| u32::from_le_bytes(p[i..i + 4].try_into().unwrap());
    let u16_at = |i: usize| u16::from_le_bytes(p[i..i + 2].try_into().unwrap());
    Some((u32_at(3), u32_at(7), u16_at(11), u16_at(13), &p[DATA_HEADER..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_roundtrip() {
        let mut p = data_header(1, 2, 3, 4).to_vec();
        p.extend_from_slice(b"hi");
        assert_eq!(parse_data(&p), Some((1, 2, 3, 4, &b"hi"[..])));
        assert_eq!(parse_data(&p[..5]), None);
    }

    #[test]
    fn request_json() {
        let r = Request::Get { id: 7 };
        let s = serde_json::to_string(&r).unwrap();
        assert_eq!(serde_json::from_str::<Request>(&s).unwrap(), r);
        assert!(Session::default().validate().is_err());
    }
}

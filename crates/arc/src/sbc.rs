//! `rCollision` resources (`SBC\xff` files): static collision meshes. See research/COLLISION.md.

/// Archive type hash of rCollision.
pub const TYPE: u32 = 0x51FC_779F;
const VERSION: u32 = 0x77BF_B5B8;
/// Attribute mask bit set on every special-purpose surface (invisible walls, kill planes, ...).
pub const SPECIAL: u32 = 0x4000_0000;
/// Floors are triangles whose normal is within 45 degrees of up (Recast's default walkable slope).
pub const FLOOR_NY: f32 = std::f32::consts::FRAC_1_SQRT_2;

pub struct Part {
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub id: u32,
    pub first_tri: u32,
    pub first_vert: u32,
}

pub struct Tri {
    pub normal: [f32; 3],
    /// Absolute indices into `Sbc::verts` (the file stores them relative to the part).
    pub v: [u32; 3],
    pub attr: u16,
}

pub struct Sbc {
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub parts: Vec<Part>,
    pub tris: Vec<Tri>,
    pub verts: Vec<[f32; 3]>,
    /// First four words of each 32-byte attribute: hit mask, 0, 0, material (0xff = none).
    pub attrs: Vec<[u32; 4]>,
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(b[i..i + 4].try_into().unwrap())
}
fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}
fn vec3(b: &[u8], i: usize) -> [f32; 3] {
    [0, 4, 8].map(|o| f32::from_le_bytes(b[i + o..i + o + 4].try_into().unwrap()))
}

pub fn parse(b: &[u8]) -> Result<Sbc, String> {
    if b.len() < 0x40 || &b[..3] != b"SBC" || u32_at(b, 4) != VERSION {
        return Err("not an SBC collision file".into());
    }
    let (np, na) = (u16_at(b, 8) as usize, u16_at(b, 10) as usize);
    let (nt, nv) = (u32_at(b, 12) as usize, u32_at(b, 16) as usize);
    let (to, vo) = (0x40 + np * 48, 0x40 + np * 48 + nt * 32);
    let ao = vo + nv * 16;
    if b.len() < ao + na * 32 {
        return Err("arrays run past the end of the file".into());
    }
    let parts: Vec<Part> = (0..np)
        .map(|i| 0x40 + i * 48)
        .map(|o| Part { min: vec3(b, o), max: vec3(b, o + 16), id: u32_at(b, o + 32), first_tri: u32_at(b, o + 36), first_vert: u32_at(b, o + 40) })
        .collect();
    let mut tris = Vec::with_capacity(nt);
    for (p, part) in parts.iter().enumerate() {
        let end = parts.get(p + 1).map_or(nt, |n| n.first_tri as usize);
        if part.first_tri as usize > end || end > nt || (p == 0 && part.first_tri != 0) {
            return Err(format!("part {p}: bad triangle range"));
        }
        for t in part.first_tri as usize..end {
            let o = to + t * 32;
            let v = [12, 14, 16].map(|k| part.first_vert + u16_at(b, o + k) as u32);
            let attr = u16_at(b, o + 18);
            if v.iter().any(|&i| i as usize >= nv) || attr as usize >= na {
                return Err(format!("triangle {t}: index out of range"));
            }
            tris.push(Tri { normal: vec3(b, o), v, attr });
        }
    }
    if tris.len() != nt {
        return Err("parts do not cover the triangles".into());
    }
    Ok(Sbc {
        min: vec3(b, 0x20),
        max: vec3(b, 0x30),
        parts,
        tris,
        verts: (0..nv).map(|i| vec3(b, vo + i * 16)).collect(),
        attrs: (0..na).map(|i| [0, 4, 8, 12].map(|k| u32_at(b, ao + i * 32 + k))).collect(),
    })
}

impl Sbc {
    pub fn mask(&self, t: &Tri) -> u32 {
        self.attrs[t.attr as usize][0]
    }

    /// Ordinary solid surface facing up enough to stand on.
    pub fn is_floor(&self, t: &Tri) -> bool {
        self.mask(t) & SPECIAL == 0 && t.normal[1] >= FLOOR_NY
    }

    /// Wavefront OBJ (same axes and units as the game: Y up, ~cm) with groups
    /// `floor`, `wall` (other solid triangles) and `special_<mask>`.
    pub fn to_obj(&self) -> String {
        use std::fmt::Write;
        let mut s = String::new();
        for v in &self.verts {
            writeln!(s, "v {} {} {}", v[0], v[1], v[2]).unwrap();
        }
        let group = |t: &Tri| match self.mask(t) {
            m if m & SPECIAL != 0 => format!("special_{m:08x}"),
            _ if self.is_floor(t) => "floor".into(),
            _ => "wall".into(),
        };
        let mut groups: Vec<(String, Vec<&Tri>)> = vec![];
        for t in &self.tris {
            let g = group(t);
            match groups.iter_mut().find(|(n, _)| *n == g) {
                Some((_, ts)) => ts.push(t),
                None => groups.push((g, vec![t])),
            }
        }
        for (name, ts) in groups {
            writeln!(s, "g {name}").unwrap();
            for t in ts {
                writeln!(s, "f {} {} {}", t.v[0] + 1, t.v[1] + 1, t.v[2] + 1).unwrap();
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VARIABLE: &str = r"G:\Lost Planet 2\nativePC\arcXmlTmpPC\variable";

    /// Parses every rCollision in the online maps' `_v` archives; skipped when the game is not installed.
    #[test]
    fn online_maps() {
        let Ok(dir) = std::fs::read_dir(VARIABLE) else { return eprintln!("game not found, skipping") };
        let mut maps = 0;
        for f in dir.map(|e| e.unwrap().path()) {
            let name = f.file_name().unwrap().to_string_lossy().into_owned();
            if !(name.starts_with("aArea_ONLINE_") && name.ends_with("_v.arc")) {
                continue;
            }
            maps += 1;
            for e in crate::read(&std::fs::read(&f).unwrap()).unwrap().into_iter().filter(|e| e.type_hash == TYPE) {
                let c = parse(&e.data).unwrap_or_else(|err| panic!("{name} {}: {err}", e.name));
                for t in &c.tris {
                    let [a, b, d] = t.v.map(|i| c.verts[i as usize]);
                    let (e1, e2) = ([0, 1, 2].map(|k| b[k] - a[k]), [0, 1, 2].map(|k| d[k] - a[k]));
                    let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
                    // Counter-clockwise winding seen from the normal side, as Recast expects.
                    assert!((0..3).map(|k| n[k] * t.normal[k]).sum::<f32>() >= 0.0, "{name} {}: winding", e.name);
                    assert!(a.iter().enumerate().all(|(k, &x)| x >= c.min[k] - 1.0 && x <= c.max[k] + 1.0), "{name} {}: bbox", e.name);
                }
                if e.name == r"scr\Online\online12\atarionline12" {
                    let floor: Vec<_> = c.tris.iter().filter(|t| c.is_floor(t)).flat_map(|t| t.v).map(|i| c.verts[i as usize]).collect();
                    let lo = |k: usize| floor.iter().map(|v| v[k]).fold(f32::MAX, f32::min);
                    let hi = |k: usize| floor.iter().map(|v| v[k]).fold(f32::MIN, f32::max);
                    assert!(lo(0) < -5400.0 && hi(0) > 5400.0 && lo(2) < -4000.0 && hi(2) > 2100.0 && lo(1) == 0.0, "Cube floor extents");
                }
            }
        }
        assert_eq!(maps, 14);
    }
}

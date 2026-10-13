//! Lost Planet 2 AI navigation resources (layouts and evidence: research/NAVMESH.md).
//!
//! - `LNV\0` rAINavigationMeshList (type 0x56CF93D4): navigation meshes (triangles, links, areas, quadtree)
//! - `WAY\0` rAIWayPoint (type 0x5F36B659): way points and their quadtree
//! - `IBL\0` rIntelligenceBox (type 0x37B53731, `tool\AI\AreaAI\..`): AI boxes and a 32x32 grid over them
//!
//! Each type has `parse(&[u8])` and `to_bytes()`; `parse(x)?.to_bytes()? == x` for every shipped file.
//! One `io` function per struct serves both directions, so reader and writer cannot drift apart.

pub const LIST_TYPE: u32 = 0x56CF_93D4;
pub const WAY_TYPE: u32 = 0x5F36_B659;
pub const IBL_TYPE: u32 = 0x37B5_3731;

#[derive(Debug, PartialEq)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

type R = Result<(), Error>;

fn err<T>(msg: impl Into<String>) -> Result<T, Error> {
    Err(Error(msg.into()))
}

/// A byte stream that is either read into the fields or written from them.
enum Io<'a> {
    Read(&'a [u8], usize),
    Write(Vec<u8>),
}

macro_rules! prims {
    ($($name:ident: $t:ty),*) => {$(
        fn $name(&mut self, v: &mut $t) -> R {
            let mut b = v.to_le_bytes();
            self.raw(&mut b)?;
            *v = <$t>::from_le_bytes(b);
            Ok(())
        }
    )*};
}

impl Io<'_> {
    fn raw(&mut self, b: &mut [u8]) -> R {
        match self {
            Io::Read(d, o) => {
                let Some(src) = d.get(*o..*o + b.len()) else { return err(format!("truncated at {:#x}", *o)) };
                b.copy_from_slice(src);
                *o += b.len();
            }
            Io::Write(out) => out.extend_from_slice(b),
        }
        Ok(())
    }

    prims!(u8: u8, i8: i8, u16: u16, u32: u32, i32: i32, f32: f32);

    fn vec3(&mut self, v: &mut [f32; 3]) -> R {
        v.iter_mut().try_for_each(|x| self.f32(x))
    }

    fn magic(&mut self, m: &[u8; 4]) -> R {
        let mut b = *m;
        self.raw(&mut b)?;
        if &b != m { err(format!("not {}", String::from_utf8_lossy(&m[..3]))) } else { Ok(()) }
    }

    /// `n` elements; reading resizes `v` to `n` first. `n` is checked against the bytes left,
    /// since every element takes at least one byte.
    fn items<T: Default>(&mut self, n: usize, v: &mut Vec<T>, mut f: impl FnMut(&mut Self, &mut T) -> R) -> R {
        if let Io::Read(d, o) = self {
            if n > d.len() - *o {
                return err(format!("count {n} at {:#x} runs past the end", *o));
            }
            v.clear();
            v.resize_with(n, T::default);
        }
        v.iter_mut().try_for_each(|x| f(self, x))
    }

    /// The number of elements of `v` in the file, as written by `count`.
    fn len<T>(&mut self, v: &[T], count: impl FnOnce(&mut Self, usize) -> Result<usize, Error>) -> Result<usize, Error> {
        count(self, v.len())
    }

    fn list32<T: Default>(&mut self, v: &mut Vec<T>, f: impl FnMut(&mut Self, &mut T) -> R) -> R {
        let n = self.len(v, |s, n| { let mut c = n as u32; s.u32(&mut c).map(|_| c as usize) })?;
        self.items(n, v, f)
    }

    fn list8<T: Default>(&mut self, v: &mut Vec<T>, f: impl FnMut(&mut Self, &mut T) -> R) -> R {
        if v.len() > 255 {
            return err("more than 255 elements in a u8-counted list");
        }
        let n = self.len(v, |s, n| { let mut c = n as u8; s.u8(&mut c).map(|_| c as usize) })?;
        self.items(n, v, f)
    }

    /// u32 length without the NUL, the bytes, NUL.
    fn string(&mut self, s: &mut String) -> R {
        let mut b = s.clone().into_bytes();
        b.push(0);
        let n = self.len(&b[1..], |io, n| { let mut c = n as u32; io.u32(&mut c).map(|_| c as usize) })?;
        self.items(n + 1, &mut b, |io, x| io.u8(x))?;
        if b.pop() != Some(0) {
            return err("string without NUL");
        }
        *s = String::from_utf8(b).map_err(|_| Error("string is not UTF-8".into()))?;
        Ok(())
    }

    /// A total the game stores after a table (it sizes its pools from it): written from the data,
    /// checked when reading.
    fn total(&mut self, sum: usize, what: &str) -> R {
        let mut v = sum as u32;
        self.u32(&mut v)?;
        if v as usize != sum { err(format!("{what} total {v} != {sum}")) } else { Ok(()) }
    }

    fn total16(&mut self, sum: usize, what: &str) -> R {
        let mut v = sum as u16;
        self.u16(&mut v)?;
        if v as usize != sum { err(format!("{what} total {v} != {sum}")) } else { Ok(()) }
    }

    fn run<T>(d: &[u8], v: &mut T, f: impl FnOnce(&mut Io, &mut T) -> R) -> R {
        let mut io = Io::Read(d, 0);
        f(&mut io, v)?;
        match io {
            Io::Read(d, o) if o != d.len() => err(format!("{} bytes left over", d.len() - o)),
            _ => Ok(()),
        }
    }

    /// Fails on data the format cannot hold (a u8 count above 255, a mismatched list length).
    fn written<T: Clone>(v: &T, f: impl FnOnce(&mut Io, &mut T) -> R) -> Result<Vec<u8>, Error> {
        let mut io = Io::Write(vec![]);
        f(&mut io, &mut v.clone())?;
        let Io::Write(out) = io else { unreachable!() };
        Ok(out)
    }

    fn reading(&self) -> bool {
        matches!(self, Io::Read(..))
    }
}

// ---------------------------------------------------------------- shared quadtree

/// Full quadtree in heap order: cell 0 is the root, the children of cell `i` are `4i+1..=4i+4`
/// (child bit 0 = upper x half, bit 1 = upper z half). Only leaves (the last level) hold entries.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct QuadTree {
    /// Levels including the root: cells = (4^depth - 1) / 3.
    pub depth: u8,
    pub min: [f32; 3],
    pub max: [f32; 3],
    /// Per cell: (polygon or way point index, unknown u32, always 0).
    pub cells: Vec<Vec<(u32, u32)>>,
}

impl QuadTree {
    /// Index of the leaf cell at grid position (ix, iz), each in 0..2^(depth-1).
    pub fn leaf(&self, ix: usize, iz: usize) -> usize {
        (0..self.depth as usize - 1).rev().fold(0, |i, b| 4 * i + 1 + ((ix >> b) & 1 | ((iz >> b) & 1) << 1))
    }

    pub fn first_leaf(&self) -> usize {
        (4usize.pow(self.depth as u32 - 1) - 1) / 3
    }

    /// The LNV form stores min then max as vec4s (w = 0), the WAY form max then min as vec3s.
    fn io(&mut self, s: &mut Io, way: bool) -> R {
        s.u8(&mut self.depth)?;
        let mut zero = 0.0;
        let mut w = |s: &mut Io| -> R {
            s.f32(&mut zero)?;
            if zero != 0.0 { err("quadtree box w != 0") } else { Ok(()) }
        };
        let n = s.len(&self.cells, |s, n| { let mut c = n as u32; s.u32(&mut c).map(|_| c as usize) })?;
        if way {
            s.vec3(&mut self.max)?;
            s.vec3(&mut self.min)?;
        } else {
            s.vec3(&mut self.min)?;
            w(s)?;
            s.vec3(&mut self.max)?;
            w(s)?;
        }
        s.items(n, &mut self.cells, |s, c| s.list32(c, |s, (a, b)| { s.u32(a)?; s.u32(b) }))
    }
}

// ---------------------------------------------------------------- LNV

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NavigationList {
    /// 9 in all shipped files.
    pub version: u32,
    pub entries: Vec<Navigation>,
}

/// rAINavigationMeshList::NavigationList plus the rNavigationMesh stored inline after it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Navigation {
    /// mPath, e.g. `tool\AI\NaviMesh\Train_26\Navi_Train_26`.
    pub path: String,
    /// mName.
    pub name: String,
    pub index: u16,
    pub priority: u8,
    /// -1: no parent.
    pub parent: i8,
    pub links: Vec<u8>,
    pub outline: Vec<[f32; 3]>,
    pub mesh: NavigationMesh,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NavigationMesh {
    pub name: String,
    pub vertices: Vec<[f32; 3]>,
    pub polygons: Vec<Polygon>,
    /// rNavigationMesh::nodeInfo (mNodeOffset + mAABB); empty in all shipped files.
    pub node_info: Vec<NodeInfo>,
    pub areas: Vec<Area>,
    pub tree: QuadTree,
}

/// rNavigationMesh::nodeData.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Polygon {
    /// mIndex: the polygon's own index.
    pub index: i32,
    /// mpAttribute: one u32 bit mask in all shipped files (probably the floor's collision attribute).
    pub attributes: Vec<u32>,
    /// Unknown bool, always 0.
    pub flag: u8,
    /// Unknown, always (0, 0, 1) and 0.
    pub unknown_vec: [f32; 3],
    pub unknown_f32: f32,
    /// Unknown u8-counted list, always empty.
    pub unknown_list: Vec<u32>,
    /// Vertex indices (3 in all shipped files; winding is not consistent).
    pub vertices: Vec<i32>,
    pub links: Vec<Link>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Link {
    /// Neighbouring polygon.
    pub to: i32,
    /// Unknown, always 0.
    pub unknown: u32,
    /// Shared edge: from `vertices[edge]` to `vertices[(edge + 1) % n]`.
    pub edge: u32,
    /// Distance between the two polygons' centroids.
    pub cost: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NodeInfo {
    pub node_offset: u32,
    pub aabb: [f32; 8],
}

/// A named group of polygons (`root` covers all, the others are tiles of it).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Area {
    pub id: u16,
    pub name: String,
    /// Unknown, always 0.
    pub unknown: u32,
    pub shapes: Vec<AreaShape>,
    /// Polygons `first_polygon..end_polygon` belong to the area.
    pub first_polygon: u16,
    pub end_polygon: u16,
    /// 0xFFFF for the root.
    pub parent: u16,
    pub children: Vec<u8>,
    /// Areas whose polygons border this one's.
    pub neighbours: Vec<u8>,
}

/// kind 0: AABB min xyz, max xyz. 1: box half size xyz then a 4x3 matrix (rows x, y, z, position).
/// 2: radius, center xyz. Only kind 0 is shipped.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AreaShape {
    pub kind: u8,
    pub data: Vec<f32>,
}

impl NavigationList {
    pub fn parse(d: &[u8]) -> Result<Self, Error> {
        let mut v = Self::default();
        Io::run(d, &mut v, Self::io)?;
        Ok(v)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        Io::written(self, Self::io)
    }

    fn io(s: &mut Io, l: &mut Self) -> R {
        s.magic(b"LNV\0")?;
        s.u32(&mut l.version)?;
        s.list8(&mut l.entries, |s, e| {
            s.string(&mut e.path)?;
            s.string(&mut e.name)?;
            s.u16(&mut e.index)?;
            s.u8(&mut e.priority)?;
            s.i8(&mut e.parent)?;
            s.list8(&mut e.links, |s, x| s.u8(x))?;
            s.list32(&mut e.outline, |s, p| s.vec3(p))?;
            e.mesh.io(s)
        })
    }
}

impl NavigationMesh {
    fn io(&mut self, s: &mut Io) -> R {
        s.string(&mut self.name)?;
        let len32 = |s: &mut Io, n: usize| -> Result<usize, Error> {
            let mut c = n as u32;
            s.u32(&mut c).map(|_| c as usize)
        };
        let nv = s.len(&self.vertices, len32)?;
        let np = s.len(&self.polygons, len32)?;
        let ni = s.len(&self.node_info, len32)?;
        s.items(nv, &mut self.vertices, |s, v| s.vec3(v))?;
        s.items(np, &mut self.polygons, |s, p| {
            s.i32(&mut p.index)?;
            s.list32(&mut p.attributes, |s, x| s.u32(x))?;
            s.u8(&mut p.flag)?;
            s.vec3(&mut p.unknown_vec)?;
            s.f32(&mut p.unknown_f32)?;
            s.list8(&mut p.unknown_list, |s, x| s.u32(x))?;
            s.list32(&mut p.vertices, |s, x| s.i32(x))?;
            s.list32(&mut p.links, |s, l| {
                s.i32(&mut l.to)?;
                s.u32(&mut l.unknown)?;
                s.u32(&mut l.edge)?;
                s.f32(&mut l.cost)
            })
        })?;
        let mut na = self.areas.len() as u16;
        s.u16(&mut na)?;
        s.items(na as usize, &mut self.areas, |s, a| {
            s.u16(&mut a.id)?;
            s.string(&mut a.name)?;
            s.u32(&mut a.unknown)?;
            s.list32(&mut a.shapes, |s, sh| {
                s.u8(&mut sh.kind)?;
                let n = match sh.kind {
                    0 => 6,
                    1 => 15,
                    2 => 4,
                    k => return err(format!("unknown area shape {k}")),
                };
                s.items(n, &mut sh.data, |s, x| s.f32(x))
            })?;
            s.u16(&mut a.first_polygon)?;
            s.u16(&mut a.end_polygon)?;
            s.u16(&mut a.parent)?;
            s.list8(&mut a.children, |s, x| s.u8(x))?;
            s.list8(&mut a.neighbours, |s, x| s.u8(x))
        })?;
        s.total16(self.areas.iter().map(|a| a.children.len()).sum(), "area children")?;
        s.total16(self.areas.iter().map(|a| a.neighbours.len()).sum(), "area neighbours")?;
        s.total(self.polygons.iter().map(|p| p.links.len()).sum(), "links")?;
        s.total(self.polygons.iter().map(|p| p.attributes.len()).sum(), "attributes")?;
        s.total(self.polygons.iter().map(|p| p.vertices.len()).sum(), "polygon vertices")?;
        s.items(ni, &mut self.node_info, |s, n| {
            s.u32(&mut n.node_offset)?;
            n.aabb.iter_mut().try_for_each(|x| s.f32(x))
        })?;
        self.tree.io(s, false)
    }
}

// ---------------------------------------------------------------- WAY

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WayPoints {
    /// 16 in all shipped files.
    pub version: u32,
    pub nodes: Vec<WayPoint>,
    pub tree: QuadTree,
}

/// rAIWayPoint::Node.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WayPoint {
    pub id: u32,
    pub pos: [f32; 3],
    /// mpAttribute: one u32 bit mask per point in all shipped files (meaning unknown).
    pub attributes: Vec<u32>,
    /// mpLink: explicit links to other way points (rare; symmetric).
    pub links: Vec<u32>,
    /// mpLinkAttribute: one per link (the game forces the count to the link count), always 0.
    pub link_attributes: Vec<u32>,
    /// mType: 0 none, 1 = 16 bytes and 2 = 80 bytes of mpGeometry follow. Only 0 is shipped.
    pub geometry_type: u8,
    pub geometry: Vec<u8>,
}

impl WayPoints {
    pub fn parse(d: &[u8]) -> Result<Self, Error> {
        let mut v = Self::default();
        Io::run(d, &mut v, Self::io)?;
        Ok(v)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        Io::written(self, Self::io)
    }

    fn io(s: &mut Io, w: &mut Self) -> R {
        s.magic(b"WAY\0")?;
        s.u32(&mut w.version)?;
        s.list32(&mut w.nodes, |s, p| {
            s.u32(&mut p.id)?;
            s.vec3(&mut p.pos)?;
            let len32 = |s: &mut Io, n: usize| -> Result<usize, Error> {
                let mut c = n as u32;
                s.u32(&mut c).map(|_| c as usize)
            };
            let na = s.len(&p.attributes, len32)?;
            let nl = s.len(&p.links, len32)?;
            if !s.reading() && p.link_attributes.len() != nl {
                return err("way point link_attributes must have one entry per link");
            }
            s.items(na, &mut p.attributes, |s, x| s.u32(x))?;
            s.items(nl, &mut p.links, |s, x| s.u32(x))?;
            s.items(nl, &mut p.link_attributes, |s, x| s.u32(x))?;
            s.u8(&mut p.geometry_type)?;
            let n = match p.geometry_type { 1 => 16, 2 => 80, _ => 0 };
            s.items(n, &mut p.geometry, |s, x| s.u8(x))
        })?;
        s.total(w.nodes.iter().map(|p| p.links.len()).sum(), "way links")?;
        s.total(w.nodes.iter().map(|p| p.attributes.len()).sum(), "way attributes")?;
        w.tree.io(s, true)
    }
}

// ---------------------------------------------------------------- IBL

#[derive(Clone, Debug, Default, PartialEq)]
pub struct IntelligenceBox {
    /// 36 in all shipped files (the loader rejects anything else).
    pub version: u32,
    /// Grid corners (the loader calls them mGridPos0/1; pos0 is the max corner in shipped files).
    pub grid_pos0: [f32; 3],
    pub grid_pos1: [f32; 3],
    /// 32x32 cells, index iz * 32 + ix over x/z; each lists up to 16 box indices, -1 padded.
    pub grid: Vec<[i32; 16]>,
    pub boxes: Vec<AiBox>,
}

pub const IBL_GRID: usize = 32;

/// One area of AI behaviour (rIntelligenceBox::CONTENTS_DATA_*).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AiBox {
    pub priority: u32,
    /// Shape kind (a bit): 2 quad prism, 4 sphere, 8 capsule, 0x10 AABB, 0x20 OBB, 0x40 u8, 0x80 see
    /// NAVMESH.md; other kinds carry no data.
    pub shape_type: u32,
    pub shape: Vec<u8>,
    pub field: Field,
    pub goal: Goal,
    pub env: Env,
    pub avoid: Avoid,
    pub wait: Wait,
    pub swim: Swim,
    pub straight: Straight,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Field {
    pub control: u32,
    pub unknown: u32,
    pub commands: Vec<Command>,
    pub command_areas: Vec<[f32; 3]>,
}

/// COMMAND_LIST_WORK; no shipped Training map has any.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Command {
    pub command_id: u8,
    pub evaluate_mode: u32,
    pub filter: u32,
    pub free_value: u32,
    pub num: u8,
    pub id: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Goal {
    pub control: u32,
    /// (linkNo: another box's index or -1, pos).
    pub points: [(i32, [f32; 3]); 5],
    pub mode: u8,
    pub value: u8,
}

/// COMMAND_AREA_LIST_WORK.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Env {
    pub control: u32,
    pub danger_level: u8,
    pub hate_up_level: u8,
    pub shot_ratio: f32,
    pub detect_ratio: f32,
    pub hear_range_ratio: f32,
    pub sneak_ratio: f32,
    pub info_common_flag: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Avoid {
    pub control: u32,
    pub mode: u8,
    pub pos: [[f32; 3]; 5],
    pub free_value: u32,
    pub target: u8,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Wait {
    pub control: u32,
    pub check_mode: u8,
    pub pos: [[f32; 3]; 3],
    pub value: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Swim {
    pub control: u32,
    pub pos: [[f32; 3]; 3],
    pub mode: u8,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Straight {
    pub control: u32,
    pub mode: u8,
}

/// Bytes of shape data after the kind (loader 0xFD5D2C jump table).
fn ibl_shape_len(kind: u32) -> usize {
    match kind {
        2 => 44,
        4 => 16,
        8 => 28,
        0x10 => 24,
        0x20 => 76,
        0x40 => 1,
        0x80 => 33,
        _ => 0,
    }
}

impl IntelligenceBox {
    pub fn parse(d: &[u8]) -> Result<Self, Error> {
        let mut v = Self::default();
        Io::run(d, &mut v, Self::io)?;
        Ok(v)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        Io::written(self, Self::io)
    }

    fn io(s: &mut Io, b: &mut Self) -> R {
        s.magic(b"IBL\0")?;
        s.u32(&mut b.version)?;
        let mut n = b.boxes.len() as u32;
        s.u32(&mut n)?;
        s.vec3(&mut b.grid_pos0)?;
        s.vec3(&mut b.grid_pos1)?;
        if !s.reading() && b.grid.len() != IBL_GRID * IBL_GRID {
            return err("IBL grid must have 32x32 cells");
        }
        s.items(IBL_GRID * IBL_GRID, &mut b.grid, |s, c| c.iter_mut().try_for_each(|x| s.i32(x)))?;
        s.items(n as usize, &mut b.boxes, |s, x| {
            s.u32(&mut x.priority)?;
            s.u32(&mut x.shape_type)?;
            s.items(ibl_shape_len(x.shape_type), &mut x.shape, |s, v| s.u8(v))?;
            let f = &mut x.field;
            s.u32(&mut f.control)?;
            s.u32(&mut f.unknown)?;
            s.list32(&mut f.commands, |s, c| {
                s.u8(&mut c.command_id)?;
                s.u32(&mut c.evaluate_mode)?;
                s.u32(&mut c.filter)?;
                s.u32(&mut c.free_value)?;
                s.u8(&mut c.num)?;
                s.u32(&mut c.id)
            })?;
            s.list32(&mut f.command_areas, |s, p| s.vec3(p))?;
            let g = &mut x.goal;
            s.u32(&mut g.control)?;
            g.points.iter_mut().try_for_each(|(l, p)| { s.i32(l)?; s.vec3(p) })?;
            s.u8(&mut g.mode)?;
            s.u8(&mut g.value)?;
            let e = &mut x.env;
            s.u32(&mut e.control)?;
            s.u8(&mut e.danger_level)?;
            s.u8(&mut e.hate_up_level)?;
            for r in [&mut e.shot_ratio, &mut e.detect_ratio, &mut e.hear_range_ratio, &mut e.sneak_ratio] {
                s.f32(r)?;
            }
            s.u32(&mut e.info_common_flag)?;
            let a = &mut x.avoid;
            s.u32(&mut a.control)?;
            s.u8(&mut a.mode)?;
            a.pos.iter_mut().try_for_each(|p| s.vec3(p))?;
            s.u32(&mut a.free_value)?;
            s.u8(&mut a.target)?;
            let w = &mut x.wait;
            s.u32(&mut w.control)?;
            s.u8(&mut w.check_mode)?;
            w.pos.iter_mut().try_for_each(|p| s.vec3(p))?;
            s.u32(&mut w.value)?;
            let sw = &mut x.swim;
            s.u32(&mut sw.control)?;
            sw.pos.iter_mut().try_for_each(|p| s.vec3(p))?;
            s.u8(&mut sw.mode)?;
            s.u32(&mut x.straight.control)?;
            s.u8(&mut x.straight.mode)
        })
    }
}

// ---------------------------------------------------------------- OBJ export

/// Wavefront OBJ: object `navmesh` (one face per polygon) and object `waypoints` (a point per way
/// point, a line per explicit link).
pub fn to_obj(list: &NavigationList, way: Option<&WayPoints>) -> String {
    use std::fmt::Write;
    let mut o = String::new();
    let mut base = 1;
    for e in &list.entries {
        let _ = writeln!(o, "o navmesh_{}", e.name.replace(' ', "_"));
        for v in &e.mesh.vertices {
            let _ = writeln!(o, "v {} {} {}", v[0], v[1], v[2]);
        }
        for p in &e.mesh.polygons {
            o.push('f');
            for &i in &p.vertices {
                let _ = write!(o, " {}", base + i as usize);
            }
            o.push('\n');
        }
        base += e.mesh.vertices.len();
    }
    if let Some(w) = way {
        o.push_str("o waypoints\n");
        for p in &w.nodes {
            let _ = writeln!(o, "v {} {} {}", p.pos[0], p.pos[1], p.pos[2]);
        }
        for (i, p) in w.nodes.iter().enumerate() {
            let _ = writeln!(o, "p {}", base + i);
            for &l in p.links.iter().filter(|&&l| l as usize > i) {
                let _ = writeln!(o, "l {} {}", base + i, base + l as usize);
            }
        }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn small_roundtrip_and_errors() {
        let mut w = WayPoints { version: 16, ..Default::default() };
        w.nodes = vec![
            WayPoint { id: 0, pos: [1.0, 2.0, 3.0], attributes: vec![7], links: vec![1], link_attributes: vec![0], ..Default::default() },
            WayPoint { id: 1, attributes: vec![7], links: vec![0], link_attributes: vec![0], ..Default::default() },
        ];
        w.tree = QuadTree { depth: 1, cells: vec![vec![(0, 0), (1, 0)]], ..Default::default() };
        let b = w.to_bytes().unwrap();
        assert_eq!(WayPoints::parse(&b).unwrap(), w);
        assert!(WayPoints::parse(&b[..b.len() - 1]).is_err());
        assert!(WayPoints::parse(&[&b[..], &[0]].concat()).is_err(), "trailing byte");
        let mut bad = b.clone();
        bad[8] = 0xFF; // node count far beyond the data
        assert!(WayPoints::parse(&bad).is_err());
        assert!(NavigationList::parse(&b).is_err(), "wrong magic");
        w.nodes[0].link_attributes.clear();
        assert!(w.to_bytes().is_err(), "one link attribute per link");
    }

    /// The four Training stages, read from the game (LP2_DIR, default G:\Lost Planet 2).
    /// Skipped when the game is not there.
    fn training_maps() -> Option<Vec<(String, Vec<crate::Entry>)>> {
        let root = std::env::var("LP2_DIR").unwrap_or_else(|_| r"G:\Lost Planet 2".into());
        let dir = std::path::Path::new(&root).join(r"nativePC\arcXmlTmpPC");
        let mut out = vec![];
        for n in ["26", "31", "41", "46"] {
            let Ok(f) = std::fs::read(dir.join(format!("aArea_TUTOREAL_{n}.arc"))) else {
                eprintln!("skipped: no Training stages under {}", dir.display());
                return None;
            };
            out.push((n.to_string(), crate::read(&f).unwrap()));
        }
        Some(out)
    }

    fn one(entries: &[crate::Entry], t: u32) -> &[u8] {
        let mut it = entries.iter().filter(|e| e.type_hash == t);
        let e = it.next().expect("resource present");
        assert!(it.next().is_none());
        &e.data
    }

    #[test]
    fn training_navigation_meshes() {
        let Some(maps) = training_maps() else { return };
        for (n, entries) in &maps {
            let d = one(entries, LIST_TYPE);
            let l = NavigationList::parse(d).unwrap();
            assert_eq!(l.to_bytes().unwrap(), d, "{n}: roundtrip");
            assert_eq!((l.version, l.entries.len()), (9, 1));
            let e = &l.entries[0];
            assert_eq!((e.index, e.parent, e.links.len(), e.outline.len()), (0, -1, 0, 0));
            let m = &e.mesh;
            let (nv, np) = (m.vertices.len(), m.polygons.len());
            assert!(m.node_info.is_empty());
            let mut one_way = 0;
            for (i, p) in m.polygons.iter().enumerate() {
                assert_eq!(p.index as usize, i);
                assert_eq!((p.vertices.len(), p.attributes.len()), (3, 1));
                assert!(p.vertices.iter().all(|&v| (v as usize) < nv), "{n}: vertex index");
                assert_eq!((p.flag, p.unknown_vec, p.unknown_f32, p.unknown_list.len()), (0, [0.0, 0.0, 1.0], 0.0, 0));
                for k in &p.links {
                    assert!((k.to as usize) < np && k.to as usize != i && (k.edge as usize) < 3 && k.unknown == 0);
                    let q = &m.polygons[k.to as usize];
                    if !q.links.iter().any(|b| b.to as usize == i) {
                        one_way += 1;
                        continue;
                    }
                    let a = p.vertices[k.edge as usize];
                    let b = p.vertices[(k.edge as usize + 1) % 3];
                    assert!(q.vertices.contains(&a) && q.vertices.contains(&b), "{n}: link {i}->{} shares its edge", k.to);
                }
            }
            // Only map 46 has one-way links (14, all with edge 0 and to far-away polygons).
            assert!(one_way <= 14, "{n}: {one_way} one-way links");
            // Areas: root covers all polygons; its children tile them in order (except a tail).
            let root = &m.areas[0];
            assert_eq!((root.name.as_str(), root.parent, root.first_polygon, root.end_polygon as usize), ("root", 0xFFFF, 0, np));
            let mut next = 0;
            for &c in &root.children {
                let a = &m.areas[c as usize];
                assert_eq!((a.id, a.parent, a.first_polygon), (c as u16, 0, next));
                next = a.end_polygon;
                for &b in &a.neighbours {
                    assert!(m.areas[b as usize].neighbours.contains(&(c)), "{n}: neighbours symmetric");
                }
            }
            // Maps 31 and 41 leave their last 6 / 9 polygons (linked connectors) in the root only.
            assert!(np - next as usize <= 9, "{n}: {} polygons outside the areas", np - next as usize);
            // Quadtree: 7 levels, only leaves used, every polygon in at least one leaf.
            let t = &m.tree;
            assert_eq!((t.depth, t.cells.len()), (7, 5461));
            assert!(t.cells[..t.first_leaf()].iter().all(Vec::is_empty));
            let seen: HashSet<u32> = t.cells.iter().flatten().map(|&(p, z)| { assert_eq!(z, 0); p }).collect();
            assert_eq!(seen.len(), np);
            assert!(seen.iter().all(|&p| (p as usize) < np));
        }
    }

    #[test]
    fn training_way_points() {
        let Some(maps) = training_maps() else { return };
        for (n, entries) in &maps {
            let d = one(entries, WAY_TYPE);
            let w = WayPoints::parse(d).unwrap();
            assert_eq!(w.to_bytes().unwrap(), d, "{n}: roundtrip");
            assert_eq!(w.version, 16);
            let np = w.nodes.len();
            for (i, p) in w.nodes.iter().enumerate() {
                assert_eq!((p.id as usize, p.attributes.len(), p.geometry_type), (i, 1, 0));
                assert!(p.link_attributes.iter().all(|&a| a == 0));
                for &l in &p.links {
                    assert!((l as usize) < np && w.nodes[l as usize].links.contains(&(i as u32)), "{n}: link {i}-{l}");
                }
            }
            // Quadtree: 5 levels; each point in exactly the leaf that contains it; box = points +-50.
            let t = &w.tree;
            assert_eq!((t.depth, t.cells.len()), (5, 341));
            for a in 0..3 {
                let lo = w.nodes.iter().map(|p| p.pos[a]).fold(f32::MAX, f32::min);
                let hi = w.nodes.iter().map(|p| p.pos[a]).fold(f32::MIN, f32::max);
                assert!((lo - t.min[a] - 50.0).abs() < 0.01 && (t.max[a] - hi - 50.0).abs() < 0.01, "{n}: box");
            }
            let mut count = vec![0; np];
            let side = 16.0;
            for (c, cell) in t.cells.iter().enumerate() {
                for &(p, _) in cell {
                    count[p as usize] += 1;
                    let pos = w.nodes[p as usize].pos;
                    let ix = ((pos[0] - t.min[0]) / (t.max[0] - t.min[0]) * side) as usize;
                    let iz = ((pos[2] - t.min[2]) / (t.max[2] - t.min[2]) * side) as usize;
                    assert_eq!(t.leaf(ix, iz), c, "{n}: point {p} leaf");
                }
            }
            assert!(count.iter().all(|&c| c == 1));
        }
    }

    #[test]
    fn training_intelligence_boxes() {
        let Some(maps) = training_maps() else { return };
        for (n, entries) in &maps {
            let d = one(entries, IBL_TYPE);
            let b = IntelligenceBox::parse(d).unwrap();
            assert_eq!(b.to_bytes().unwrap(), d, "{n}: roundtrip");
            assert_eq!(b.version, 36);
            let nb = b.boxes.len() as i32;
            for c in &b.grid {
                let used = c.iter().take_while(|&&x| x >= 0).count();
                assert!(c[used..].iter().all(|&x| x == -1), "{n}: -1 padded");
                assert!(c[..used].iter().all(|&x| x < nb));
            }
            for x in &b.boxes {
                assert!(matches!(x.shape_type, 2 | 4 | 0x20), "{n}: shape {:#x}", x.shape_type);
                assert!(x.goal.points.iter().all(|&(l, _)| l >= -1 && l < nb));
                assert!(x.field.commands.is_empty() && x.field.command_areas.is_empty());
            }
        }
    }
}

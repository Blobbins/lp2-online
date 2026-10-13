//! Generates a stage's AI navigation data (LNV mesh list + WAY points, see navi.rs) from its walking
//! collision (sbc.rs) with Recast (vendored C++ in `recast/`). Parameters and results:
//! research/NAVMESH.md, "Generator".

use crate::navi::{Area, AreaShape, Error, IntelligenceBox, Link, Navigation, NavigationList, NavigationMesh, Polygon, QuadTree, WayPoint, WayPoints, IBL_GRID};
use crate::sbc::Sbc;
use std::collections::HashMap;

/// Recast settings in game units (~cm; matched against the Training navmeshes, see NAVMESH.md).
#[derive(Clone, Copy, Debug)]
pub struct Params {
    pub cell: f32,
    pub cell_height: f32,
    pub slope_deg: f32,
    pub agent_height: f32,
    pub agent_climb: f32,
    pub agent_radius: f32,
    pub max_edge: f32,
    pub max_error: f32,
    /// Regions smaller than this many cells (squared side) are dropped.
    pub min_region: i32,
    pub merge_region: i32,
    /// Height detail: sample spacing and the allowed height error of the triangles.
    pub detail_sample: f32,
    pub detail_error: f32,
    /// Way point grid spacing; widened in steps of 100 until there are at most `max_way_points`.
    pub way_spacing: f32,
    pub max_way_points: usize,
}

pub const PARAMS: Params = Params {
    cell: 20.0,
    cell_height: 10.0,
    slope_deg: 60.0,
    agent_height: 180.0,
    agent_climb: 50.0,
    agent_radius: 20.0,
    max_edge: 600.0,
    max_error: 20.0,
    min_region: 8,
    merge_region: 20,
    detail_sample: 120.0,
    detail_error: 10.0,
    way_spacing: 300.0,
    max_way_points: 1500,
};

#[repr(C)]
struct RcParams {
    cs: f32,
    ch: f32,
    slope: f32,
    walkable_height: i32,
    walkable_climb: i32,
    walkable_radius: i32,
    max_edge_len: i32,
    max_error: f32,
    min_region_area: i32,
    merge_region_area: i32,
    detail_sample_dist: f32,
    detail_max_error: f32,
    bmin: [f32; 3],
    bmax: [f32; 3],
}

#[repr(C)]
struct RcMesh {
    nmeshes: i32,
    nverts: i32,
    ntris: i32,
    meshes: *const u32,
    verts: *const f32,
    tris: *const u8,
    handle: *mut std::ffi::c_void,
}

extern "C" {
    fn lp2_recast_build(verts: *const f32, nverts: i32, tris: *const i32, ntris: i32, walkable: *const u8, p: *const RcParams, out: *mut RcMesh) -> i32;
    fn lp2_recast_free(m: *mut RcMesh);
}

/// A triangle mesh in world space.
pub struct TriMesh {
    pub vertices: Vec<[f32; 3]>,
    pub tris: Vec<[u32; 3]>,
}

/// Recast's detail mesh over the collision's ordinary solid triangles, vertices welded.
/// Special surfaces (invisible walls, kill volumes) only block.
pub fn walkable_mesh(c: &Sbc, p: &Params) -> Result<TriMesh, Error> {
    let verts: Vec<f32> = c.verts.iter().flatten().copied().collect();
    let tris: Vec<i32> = c.tris.iter().flat_map(|t| t.v.map(|i| i as i32)).collect();
    // Huge triangles are catch planes/boxes around the level (map 01's floor at y=-3000 spans
    // 30000 x 20000 in 4 triangles); real floors are modelled in far smaller pieces.
    let huge = |t: &crate::sbc::Tri| {
        let [a, b, d] = t.v.map(|i| c.verts[i as usize]);
        ((b[0] - a[0]) * (d[2] - a[2]) - (d[0] - a[0]) * (b[2] - a[2])).abs() / 2.0 > MAX_FLOOR_TRI
    };
    let walkable: Vec<u8> = c.tris.iter().map(|t| (c.mask(t) & crate::sbc::SPECIAL == 0 && !huge(t)) as u8).collect();
    // Bounds of the floors: walls and kill volumes reach far outside the play area.
    let (mut bmin, mut bmax) = ([f32::MAX; 3], [f32::MIN; 3]);
    for v in c.tris.iter().zip(&walkable).filter(|(t, &w)| w == 1 && c.is_floor(t)).flat_map(|(t, _)| t.v) {
        for k in 0..3 {
            bmin[k] = bmin[k].min(c.verts[v as usize][k]);
            bmax[k] = bmax[k].max(c.verts[v as usize][k]);
        }
    }
    if bmin[0] > bmax[0] {
        return Err(Error("collision has no floor".into()));
    }
    bmin[1] -= p.agent_climb;
    bmax[1] += p.agent_height;
    let rp = RcParams {
        cs: p.cell,
        ch: p.cell_height,
        slope: p.slope_deg,
        walkable_height: (p.agent_height / p.cell_height).ceil() as i32,
        walkable_climb: (p.agent_climb / p.cell_height).floor() as i32,
        walkable_radius: (p.agent_radius / p.cell).ceil() as i32,
        max_edge_len: (p.max_edge / p.cell) as i32,
        max_error: p.max_error / p.cell,
        min_region_area: p.min_region * p.min_region,
        merge_region_area: p.merge_region * p.merge_region,
        detail_sample_dist: p.detail_sample,
        detail_max_error: p.detail_error,
        bmin,
        bmax,
    };
    let mut m = RcMesh { nmeshes: 0, nverts: 0, ntris: 0, meshes: std::ptr::null(), verts: std::ptr::null(), tris: std::ptr::null(), handle: std::ptr::null_mut() };
    // SAFETY: the arrays outlive the call and their lengths are passed; on success `m` points into
    // Recast's mesh, which is copied out below and then freed.
    let step = unsafe { lp2_recast_build(verts.as_ptr(), c.verts.len() as i32, tris.as_ptr(), c.tris.len() as i32, walkable.as_ptr(), &rp, &mut m) };
    if step != 0 {
        return Err(Error(format!("Recast failed at step {step}")));
    }
    // SAFETY: Recast allocated these lengths (rcPolyMeshDetail).
    let (meshes, dv, dt) = unsafe {
        (
            std::slice::from_raw_parts(m.meshes, m.nmeshes as usize * 4).to_vec(),
            std::slice::from_raw_parts(m.verts, m.nverts as usize * 3).to_vec(),
            std::slice::from_raw_parts(m.tris, m.ntris as usize * 4).to_vec(),
        )
    };
    unsafe { lp2_recast_free(&mut m) };
    // Each polygon has its own detail vertices; neighbours compute their shared edge's vertices
    // identically, so welding by position restores the adjacency.
    let mut vertices: Vec<[f32; 3]> = vec![];
    let mut index: HashMap<[i32; 3], u32> = HashMap::new();
    let mut tris = vec![];
    for sub in meshes.chunks(4) {
        let (vb, tb, nt) = (sub[0] as usize, sub[2] as usize, sub[3] as usize);
        for t in dt[tb * 4..(tb + nt) * 4].chunks(4) {
            let tri = [0, 1, 2].map(|k| {
                let v = &dv[(vb + t[k] as usize) * 3..][..3];
                let v = [v[0], v[1], v[2]];
                *index.entry(v.map(|x| (x * 4.0).round() as i32)).or_insert_with(|| {
                    vertices.push(v);
                    vertices.len() as u32 - 1
                })
            });
            if tri[0] != tri[1] && tri[1] != tri[2] && tri[0] != tri[2] {
                tris.push(tri);
            }
        }
    }
    Ok(TriMesh { vertices, tris })
}

fn bounds<'a>(pts: impl Iterator<Item = &'a [f32; 3]>) -> ([f32; 3], [f32; 3]) {
    pts.fold(([f32::MAX; 3], [f32::MIN; 3]), |(lo, hi), v| ([0, 1, 2].map(|k| lo[k].min(v[k])), [0, 1, 2].map(|k| hi[k].max(v[k]))))
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f32>().sqrt()
}

/// Fills a quadtree's leaves: `items` are (index, xz min, xz max); each goes into every leaf it overlaps.
fn fill_tree(t: &mut QuadTree, items: impl Iterator<Item = (u32, [f32; 2], [f32; 2])>) {
    let side = 1usize << (t.depth - 1);
    t.cells = vec![vec![]; (4usize.pow(t.depth as u32) - 1) / 3];
    let cell = |x: f32, k: usize| (((x - t.min[k]) / (t.max[k] - t.min[k]) * side as f32) as isize).clamp(0, side as isize - 1) as usize;
    let mut leaves = vec![vec![]; side * side];
    for (i, lo, hi) in items {
        for iz in cell(lo[1], 2)..=cell(hi[1], 2) {
            for ix in cell(lo[0], 0)..=cell(hi[0], 0) {
                leaves[iz * side + ix].push((i, 0));
            }
        }
    }
    for (n, l) in leaves.into_iter().enumerate() {
        let leaf = t.leaf(n % side, n / side);
        t.cells[leaf] = l;
    }
}

/// Floor triangles larger than this (x/z area) are not walkable.
pub const MAX_FLOOR_TRI: f32 = 25e6;

/// Tiles per axis for the area split (the Training meshes have 6-22 tiles).
const TILES: usize = 4;

/// The navigation mesh resource for `mesh`. `dir` names the folder, e.g. `Online_05`.
pub fn navigation_list(mesh: &TriMesh, dir: &str) -> Result<NavigationList, Error> {
    let v = &mesh.vertices;
    let centre = |t: &[u32; 3]| [0, 1, 2].map(|k| t.iter().map(|&i| v[i as usize][k]).sum::<f32>() / 3.0);
    let (lo, hi) = bounds(v.iter());
    // Sort triangles into a TILES x TILES grid of areas by centroid; polygons of an area are contiguous.
    let tile = |t: &[u32; 3]| {
        let c = centre(t);
        let a = |k: usize| (((c[k] - lo[k]) / (hi[k] - lo[k]).max(1.0) * TILES as f32) as usize).min(TILES - 1);
        a(2) * TILES + a(0)
    };
    let mut tris = mesh.tris.clone();
    tris.sort_by_key(tile);
    if tris.len() > u16::MAX as usize {
        return Err(Error(format!("{} triangles, the area ranges hold 65535", tris.len())));
    }
    // Links: triangles sharing an edge (the welded mesh shares vertex indices along edges).
    let mut edges: HashMap<(u32, u32), Vec<(usize, u32)>> = HashMap::new();
    for (i, t) in tris.iter().enumerate() {
        for e in 0..3 {
            let (a, b) = (t[e], t[(e + 1) % 3]);
            edges.entry((a.min(b), a.max(b))).or_default().push((i, e as u32));
        }
    }
    let centres: Vec<_> = tris.iter().map(centre).collect();
    let mut polygons: Vec<Polygon> = tris
        .iter()
        .enumerate()
        .map(|(i, t)| Polygon {
            index: i as i32,
            attributes: vec![0],
            unknown_vec: [0.0, 0.0, 1.0],
            vertices: t.iter().map(|&x| x as i32).collect(),
            ..Default::default()
        })
        .collect();
    for users in edges.values() {
        for &(a, ea) in users {
            for &(b, _) in users.iter().filter(|u| u.0 != a) {
                polygons[a].links.push(Link { to: b as i32, unknown: 0, edge: ea, cost: dist(centres[a], centres[b]) });
            }
        }
    }
    for p in &mut polygons {
        p.links.sort_by_key(|l| (l.edge, l.to));
    }
    // Areas: root, then one per non-empty tile.
    let aabb = |lo: [f32; 3], hi: [f32; 3]| AreaShape { kind: 0, data: [lo, hi].concat() };
    let mut areas = vec![Area { id: 0, name: "root".into(), shapes: vec![aabb(lo, hi)], end_polygon: tris.len() as u16, parent: 0xFFFF, ..Default::default() }];
    let mut area_of = vec![0u8; tris.len()];
    let mut start = 0;
    while start < tris.len() {
        let key = tile(&tris[start]);
        let end = start + tris[start..].iter().take_while(|t| tile(t) == key).count();
        let id = areas.len() as u16;
        let (alo, ahi) = bounds(tris[start..end].iter().flatten().map(|&i| &v[i as usize]));
        areas.push(Area { id, name: "area_aabb".into(), shapes: vec![aabb(alo, ahi)], first_polygon: start as u16, end_polygon: end as u16, parent: 0, ..Default::default() });
        area_of[start..end].fill(id as u8);
        start = end;
    }
    areas[0].children = (1..areas.len() as u8).collect();
    for (i, p) in polygons.iter().enumerate() {
        for l in &p.links {
            let (a, b) = (area_of[i], area_of[l.to as usize]);
            if a != b && !areas[a as usize].neighbours.contains(&b) {
                areas[a as usize].neighbours.push(b);
            }
        }
    }
    for a in &mut areas {
        a.neighbours.sort();
    }
    let mut tree = QuadTree { depth: 7, min: [lo[0] - 1.0, lo[1] - 1000.0, lo[2] - 1.0], max: [hi[0] + 1.0, hi[1] + 1000.0, hi[2] + 1.0], cells: vec![] };
    fill_tree(
        &mut tree,
        tris.iter().enumerate().map(|(i, t)| {
            let (a, b) = bounds(t.iter().map(|&x| &v[x as usize]));
            (i as u32, [a[0], a[2]], [b[0], b[2]])
        }),
    );
    let name = format!("Navi_{dir}");
    Ok(NavigationList {
        version: 9,
        entries: vec![Navigation {
            path: format!(r"tool\AI\NaviMesh\{dir}\{name}"),
            name: name.clone(),
            parent: -1,
            mesh: NavigationMesh { name, vertices: v.clone(), polygons, node_info: vec![], areas, tree },
            ..Default::default()
        }],
    })
}

/// Finds the floor heights of a triangle mesh at x/z positions.
pub struct Floor<'a> {
    mesh: &'a TriMesh,
    lo: [f32; 3],
    hi: [f32; 3],
    bucket: f32,
    buckets: HashMap<(i64, i64), Vec<usize>>,
}

impl<'a> Floor<'a> {
    pub fn new(mesh: &'a TriMesh) -> Self {
        let (lo, hi) = bounds(mesh.vertices.iter());
        let mut f = Floor { mesh, lo, hi, bucket: 200.0, buckets: HashMap::new() };
        for (i, t) in mesh.tris.iter().enumerate() {
            let (a, b) = bounds(t.iter().map(|&x| &mesh.vertices[x as usize]));
            let ((x0, z0), (x1, z1)) = (f.key(a[0], a[2]), f.key(b[0], b[2]));
            for z in z0..=z1 {
                for x in x0..=x1 {
                    f.buckets.entry((x, z)).or_default().push(i);
                }
            }
        }
        f
    }

    fn key(&self, x: f32, z: f32) -> (i64, i64) {
        (((x - self.lo[0]) / self.bucket).floor() as i64, ((z - self.lo[2]) / self.bucket).floor() as i64)
    }

    /// Heights of the floors above/below (x, z), at least `gap` apart.
    pub fn heights(&self, x: f32, z: f32, gap: f32) -> Vec<f32> {
        let v = &self.mesh.vertices;
        let mut ys: Vec<f32> = vec![];
        for &i in self.buckets.get(&self.key(x, z)).into_iter().flatten() {
            let [a, b, c] = self.mesh.tris[i].map(|k| v[k as usize]);
            // Barycentric coordinates in x/z.
            let d = (b[2] - c[2]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[2] - c[2]);
            if d.abs() < 1e-6 {
                continue;
            }
            let l1 = ((b[2] - c[2]) * (x - c[0]) + (c[0] - b[0]) * (z - c[2])) / d;
            let l2 = ((c[2] - a[2]) * (x - c[0]) + (a[0] - c[0]) * (z - c[2])) / d;
            let l3 = 1.0 - l1 - l2;
            if l1 >= 0.0 && l2 >= 0.0 && l3 >= 0.0 {
                let y = l1 * a[1] + l2 * b[1] + l3 * c[1];
                if ys.iter().all(|&o| (o - y).abs() > gap) {
                    ys.push(y);
                }
            }
        }
        ys
    }

    /// Floor points on a `spacing` grid over the mesh bounds.
    pub fn grid(&self, spacing: f32, gap: f32) -> Vec<[f32; 3]> {
        let (nx, nz) = (((self.hi[0] - self.lo[0]) / spacing) as usize, ((self.hi[2] - self.lo[2]) / spacing) as usize);
        let mut pos = vec![];
        for iz in 0..=nz {
            for ix in 0..=nx {
                let (x, z) = (self.lo[0] + (ix as f32 + 0.5) * spacing, self.lo[2] + (iz as f32 + 0.5) * spacing);
                pos.extend(self.heights(x, z, gap).into_iter().map(|y| [x, y, z]));
            }
        }
        pos
    }
}

/// Way points on a `spacing` grid wherever the mesh has floor (one per floor level).
pub fn way_points(mesh: &TriMesh, spacing: f32) -> WayPoints {
    let pos = Floor::new(mesh).grid(spacing, 100.0);
    let nodes: Vec<WayPoint> = pos
        .iter()
        .enumerate()
        .map(|(i, &p)| WayPoint { id: i as u32, pos: p, attributes: vec![0xFFFF_FFFF], ..Default::default() })
        .collect();
    let (plo, phi) = if pos.is_empty() { ([0.0; 3], [0.0; 3]) } else { bounds(pos.iter()) };
    let mut tree = QuadTree { depth: 5, min: plo.map(|x| x - 50.0), max: phi.map(|x| x + 50.0), cells: vec![] };
    fill_tree(&mut tree, pos.iter().enumerate().map(|(i, p)| (i as u32, [p[0], p[2]], [p[0], p[2]])));
    WayPoints { version: 16, nodes, tree }
}

/// An AreaAI file without any boxes, its grid spanning `mesh` (there is nothing to generate the
/// hand-made zones from).
pub fn empty_intelligence_box(mesh: &TriMesh) -> IntelligenceBox {
    let (lo, hi) = bounds(mesh.vertices.iter());
    IntelligenceBox { version: 36, grid_pos0: hi, grid_pos1: lo, grid: vec![[-1; 16]; IBL_GRID * IBL_GRID], boxes: vec![] }
}

/// LNV, WAY and empty IBL resources for a stage's walking collision.
pub fn generate(c: &Sbc, dir: &str, p: &Params) -> Result<(NavigationList, WayPoints, IntelligenceBox), Error> {
    let mesh = walkable_mesh(c, p)?;
    // The Training maps have 333-1269 way points (300 apart on Cube, 500 on map 41).
    let way = (0..).map(|k| way_points(&mesh, p.way_spacing + 100.0 * k as f32)).find(|w| w.nodes.len() <= p.max_way_points).unwrap();
    Ok((navigation_list(&mesh, dir)?, way, empty_intelligence_box(&mesh)))
}

/// The three archive entries for stage `stage` (e.g. aArea_ONLINE_05):
/// `tool\AI\NaviMesh\<dir>\List_<dir>`, `..\Way_<dir>` and `tool\AI\AreaAI\<stage>`.
pub fn entries(stage: &str, dir: &str, list: &NavigationList, way: &WayPoints, ibl: &IntelligenceBox) -> Result<[crate::Entry; 3], Error> {
    use crate::navi::{IBL_TYPE, LIST_TYPE, WAY_TYPE};
    // Size flag 2, as the Training stages store these files.
    let e = |name: String, type_hash, data| crate::Entry { name, type_hash, flags: 0x4000_0000, data };
    Ok([
        e(format!(r"tool\AI\NaviMesh\{dir}\List_{dir}"), LIST_TYPE, list.to_bytes()?),
        e(format!(r"tool\AI\NaviMesh\{dir}\Way_{dir}"), WAY_TYPE, way.to_bytes()?),
        e(format!(r"tool\AI\AreaAI\{stage}"), IBL_TYPE, ibl.to_bytes()?),
    ])
}

/// The stage's walking collision (`scr\..\atari<stage>`) among the entries of its `_v` archive.
pub fn walking_collision(entries: &[crate::Entry]) -> Option<&crate::Entry> {
    entries.iter().find(|e| {
        let n = e.name.to_ascii_lowercase();
        e.type_hash == crate::sbc::TYPE && n.starts_with(r"scr\") && n.rsplit('\\').next().is_some_and(|f| f.starts_with("atari"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::navi::NavigationMesh;
    use std::path::PathBuf;

    fn dir() -> Option<PathBuf> {
        let root = std::env::var("LP2_DIR").unwrap_or_else(|_| r"G:\Lost Planet 2".into());
        let d = std::path::Path::new(&root).join(r"nativePC\arcXmlTmpPC");
        d.exists().then_some(d).or_else(|| { eprintln!("skipped: game not found"); None })
    }

    fn collision(d: &std::path::Path, map: &str) -> Sbc {
        let entries = crate::read(&std::fs::read(d.join(format!(r"variable\aArea_ONLINE_{map}_v.arc"))).unwrap()).unwrap();
        crate::sbc::parse(&walking_collision(&entries).expect("atari").data).unwrap()
    }

    fn tri_mesh(m: &NavigationMesh) -> TriMesh {
        TriMesh { vertices: m.vertices.clone(), tris: m.polygons.iter().map(|p| [0, 1, 2].map(|k| p.vertices[k] as u32)).collect() }
    }

    /// Fraction of `a`'s floor (sampled every 50 units) that `b` also covers within 100 units of height.
    fn covered(a: &TriMesh, b: &TriMesh) -> f32 {
        let (fa, fb) = (Floor::new(a), Floor::new(b));
        let pts = fa.grid(50.0, 100.0);
        pts.iter().filter(|p| fb.heights(p[0], p[2], 0.0).iter().any(|y| (y - p[1]).abs() < 100.0)).count() as f32 / pts.len() as f32
    }

    fn stats(l: &NavigationList) -> (usize, usize, usize) {
        let m = &l.entries[0].mesh;
        (m.vertices.len(), m.polygons.len(), m.polygons.iter().map(|p| p.links.len()).sum())
    }

    /// Generated vs shipped navmesh on the four maps that have a Training twin.
    #[test]
    fn matches_training() {
        let Some(d) = dir() else { return };
        let mut worst = 1f32;
        for (online, train) in [("12", "26"), ("10", "31"), ("01", "41"), ("09", "46")] {
            let real = crate::read(&std::fs::read(d.join(format!("aArea_TUTOREAL_{train}.arc"))).unwrap()).unwrap();
            let real = NavigationList::parse(&real.iter().find(|e| e.type_hash == crate::navi::LIST_TYPE).unwrap().data).unwrap();
            let t = std::time::Instant::now();
            let (gen, way, _) = generate(&collision(&d, online), "Test", &PARAMS).unwrap();
            let ms = t.elapsed().as_millis();
            let (rm, gm) = (tri_mesh(&real.entries[0].mesh), tri_mesh(&gen.entries[0].mesh));
            let (recall, precision) = (covered(&rm, &gm), covered(&gm, &rm));
            eprintln!(
                "{online}/{train}: {ms} ms, generated {:?} way {} | real {:?} | real covered {recall:.3}, generated on real {precision:.3}",
                stats(&gen),
                way.nodes.len(),
                stats(&real)
            );
            worst = worst.min(recall).min(precision);
            let (g, r) = (stats(&gen), stats(&real));
            // Cube's real mesh also covers its block objects, which the collision does not have, so
            // its flat floor gets far fewer triangles.
            if online != "12" {
                assert!((r.1 / 3..r.1 * 3).contains(&g.1), "{online}: triangle count");
                assert!((2.0..3.5).contains(&(g.2 as f32 / g.1 as f32)), "{online}: links per triangle");
            }
        }
        assert!(worst > 0.6, "coverage {worst}");
    }

    /// Every online map generates, and the output re-parses to the same data.
    #[test]
    fn every_online_map() {
        let Some(d) = dir() else { return };
        let mut n = 0;
        for f in std::fs::read_dir(d.join("variable")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()) {
            let Some(map) = f.strip_prefix("aArea_ONLINE_").and_then(|s| s.strip_suffix("_v.arc")) else { continue };
            let t = std::time::Instant::now();
            let (l, w, b) = generate(&collision(&d, map), &format!("Online_{map}"), &PARAMS).unwrap();
            eprintln!("{map}: {} ms, {:?}, {} way points, {} areas", t.elapsed().as_millis(), stats(&l), w.nodes.len(), l.entries[0].mesh.areas.len());
            assert_eq!(NavigationList::parse(&l.to_bytes().unwrap()).unwrap(), l);
            assert_eq!(WayPoints::parse(&w.to_bytes().unwrap()).unwrap(), w);
            assert_eq!(IntelligenceBox::parse(&b.to_bytes().unwrap()).unwrap(), b);
            let m = &l.entries[0].mesh;
            assert!(m.polygons.len() > 100 && w.nodes.len() > 20, "{map}: too small");
            for (i, p) in m.polygons.iter().enumerate() {
                for k in &p.links {
                    let q = &m.polygons[k.to as usize];
                    assert!(q.links.iter().any(|b| b.to as usize == i), "{map}: links symmetric");
                    let (a, b) = (p.vertices[k.edge as usize], p.vertices[(k.edge as usize + 1) % 3]);
                    assert!(q.vertices.contains(&a) && q.vertices.contains(&b), "{map}: shared edge");
                }
            }
            n += 1;
        }
        assert_eq!(n, 14);
    }
}

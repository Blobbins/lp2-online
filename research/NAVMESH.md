# AI navigation files (DX11 v1.0.0.129)

Training stages `aArea_TUTOREAL_26/31/41/46` carry three AI resources that the online maps lack
(SYSTEMS.md, "Versus bots"). All three are plain little-endian binary streams, not XFS.

| File | Class (type hash = ~crc32(name) & 0x7fffffff) | Magic, version | Saver | Loader |
|---|---|---|---|---|
| `tool\AI\NaviMesh\Train_NN\List_Train_NN` | rAINavigationMeshList 0x56CF93D4 | `LNV\0`, 9 | 0x6E7410 (mesh 0x763D60, areas 0x767CC0) | 0x6E7840 |
| `tool\AI\NaviMesh\Train_NN\Way_Train_NN` | rAIWayPoint 0x5F36B659 | `WAY\0`, 16 | 0x6E5790 | 0x6E6090 |
| `tool\AI\AreaAI\aArea_TUTOREAL_NN` | rIntelligenceBox 0x37B53731 | `IBL\0`, 36 | (none found) | 0xFD5C70 (block 0xFD2BC0) |

Each loader rejects any other magic/version (`cmp eax, 9` at 0x6E7915, `cmp eax, 0x10` at 0x6E6159,
`cmp [esi+4], 0x24` at 0xFD5CD9). The layouts below come from the save functions (the game's own
writers; the stream vtable at 0x16F747C has +4/+0x10 u16, +8/+0x14 u32, +0x1C f32, +0x24 bytes)
and from the loader for IBL. Field names come from the class property tables
(research/dx11/fields.json; its entries are shifted between neighbouring structs, so names were
matched by offset and type).

**Proof of completeness:** `crates/arc/src/navi.rs` parses every byte of the 12 shipped files and
writes them back byte-identical (`cargo test -p lp2-arc`). Fields marked *unknown* hold one
constant value in every file.

**Coordinates:** world space, Y up, the same units as the collision mesh (about cm; Cube's floor
is y=0, ceiling 3500, x ±5731; see the collision notes). The OBJ export opens with Y up.

## Common

- `str`: u32 length without the NUL, the bytes, NUL.
- `vec3`: 3 × f32.
- **Quadtree** (LNV and WAY): a full quadtree in heap order. Cell 0 is the root, the children of
  cell `i` are `4i+1 .. 4i+4`, child bit 0 = upper x half, bit 1 = upper z half (y is ignored).
  Only the last level holds entries. `cells = (4^depth - 1) / 3`.

  ```
  u8   depth                 LNV: 7 (5461 cells, 64x64 leaves)   WAY: 5 (341 cells, 16x16 leaves)
  u32  cell count
  box                        LNV: vec3 min, f32 0, vec3 max, f32 0   WAY: vec3 max, vec3 min
  cell × count: u32 n, n × (u32 index, u32 unknown = 0)
  ```

  The saver writes `item.ptr->[+4]` (the polygon's / point's index) then the item's own u32.
  Evidence for the layout: every LNV entry's triangle XZ bounds touch its leaf in that heap order
  (19767/19767 on map 26; 7 other child orders give < 4%), and every way point lies in exactly the
  leaf that contains it.

## LNV: rAINavigationMeshList

```
"LNV\0"  u32 version = 9
u8 count                                   (1 in all shipped files)
count × NavigationList:
  str  mPath        tool\AI\NaviMesh\Train_26\Navi_Train_26   (written via sprintf)
  str  mName        "new Navigation", "Train31", ...
  u16  mNavigationIndex  0
  u8   mPriority         0
  i8   mParentIndex      -1
  u8   mNumberOfLink, then that many u8 mpLinkIndex      (0)
  u32  mNumberOfOutLine, then that many vec3 mpOutLine   (0)
  rNavigationMesh (inline, 0x763D60):
```

### rNavigationMesh

```
str  name                                  same as mName
u32  vertex count  V     [+0x88]
u32  polygon count P     [+0x90]
u32  nodeInfo count I    [+0x98]          0 in all files
V × vec3 vertex
P × nodeData (polygon):
  i32  mIndex             = its own position
  u32  mNumberOfAttribute (1), then that many u32 mpAttribute
         a bit mask: mostly 0; 0x400, 0x800, 0x40000, 0x20000, 0x800000, 0x4000000 ... per map.
         Probably the floor's collision attribute (unknown; compare with the SBC attribute bits).
  u8   unknown bool                [+0x10]   always 0
  vec3 unknown                     [+0x20]   always (0, 0, 1)
  f32  unknown                     [+0x30]   always 0
  u8   n, then n × u32 unknown     [+0x34]   always n = 0
  u32  vertex count (3), then that many i32 vertex indices         [+0x3C]
         winding is not consistent (cross(v1-v0, v2-v0).y > 0 for 68..99%)
  u32  link count, then link count × Link                           [+0x44]
         i32 to     neighbouring polygon (the saver skips entries < -1 but keeps the count)
         u32 unknown, always 0
         u32 edge   the shared edge is vertices[edge] -> vertices[(edge+1) % 3]
         f32 cost   distance between the two triangles' centroids (all 71468 links)
u16  area count A, then A × Area (0x767CC0):
  u16  id                          [+0x10]  = its position
  str  name                        "root", then "area_aabb" × (A-1)
  u32  unknown                     [+0x18]  always 0
  u32  shape count (1), then shapes:
         u8 kind: 0 AABB: vec3 min, vec3 max
                  1 box:  vec3 half size [+0x50], then 4 × vec3 rows [+0x10],[+0x20],[+0x30],[+0x40]
                  2 sphere: f32 radius [+0x1C], vec3 center [+0x10]
         (only kind 0 is shipped; the root's box is sometimes garbage, e.g. map 31)
  u16  first polygon               [+0x12]
  u16  end polygon (exclusive)     [+0x14]
  u16  parent                      [+0x1C]  0xFFFF for root, 0 for the tiles
  u8   n, n × u8 children          [+0x20]  root: 1..A-1; tiles: none
  u8   n, n × u8 neighbours        [+0x28]  tiles whose polygons border this one (symmetric)
u16  total children,  u16 total neighbours           (sums over the areas)
u32  total links, u32 total attributes, u32 total polygon vertices   (sums over the polygons)
I × nodeInfo: u32 mNodeOffset, 8 × f32 mAABB         (none shipped)
quadtree (LNV form)
```

Polygons are sorted by area: the tiles' ranges are contiguous from 0. Maps 31 and 41 end with 6 / 9
polygons that only the root covers (connectors linked to tiles). Links are symmetric and share
the stated edge, except 14 one-way links in map 46 (all `edge 0`, to polygons 2000-9700 units
away, several to polygon 0): probably jump/drop links, or leftovers. Several links can use the
same edge (T-junctions), so a triangle can have up to 6.

The LNV quadtree box is the vertex bounds grown by ~1-1.6 in x/z and 1000 in y (map 31: 1284 up).
A leaf lists every polygon whose XZ bounds overlap it; boundary ties differ for < 0.3% of
entries, so a generator can use a plain bounds-overlap test (a superset is harmless).

## WAY: rAIWayPoint

```
"WAY\0"  u32 version = 16
u32 count N
N × Node:
  u32  mID                 = its position             [+0x04]
  vec3 mPos                                           [+0x10]   (mNormal [+0x20] is not stored)
  u32  mNumberOfAttribute                             [+0x40]   1 in all files
  u32  mNumberOfLink  L                               [+0x30]
  mNumberOfAttribute × u32 mpAttribute                          a bit mask, meaning unknown;
         often the two u16 halves are equal (0xFFFFFFFF, 0xC03AC03A, ...)
  L × u32 mpLink                                       other way points (symmetric), rare:
         0 link ends on map 26, 16 / 24 / 150 on maps 31 / 41 / 46
  L × u32 mpLinkAttribute                              always 0; the saver forces
         mNumberOfLinkAttribute [+0x38] to L before writing
  u8   mType                                           [+0x48]
         1: 16 bytes of mpGeometry follow, 2: 80 bytes, else none (only 0 shipped)
u32  total links, u32 total attributes
quadtree (WAY form): box = point bounds ± 50; each point in exactly one leaf
```

The points lie on a regular x/z grid (300 apart on Cube, 500 on map 41) at floor height, so they
look like a sampled cover/position grid rather than a path graph; the navmesh does path finding.

## IBL: rIntelligenceBox (`tool\AI\AreaAI`)

```
"IBL\0"  u32 version = 36   u32 box count N
vec3 mGridPos0   (the max corner in all files)  vec3 mGridPos1 (min corner)
1024 × 16 × i32 mGridIdList      32x32 cells, index iz*32 + ix over the two corners; each lists
                                  the boxes overlapping the cell, -1 padded (at most 13 used)
N × box (0x260 bytes in memory):
  u32 mPriority                    4..7
  u32 shape kind, then its data (jump table at 0xFD6264/0xFD6284):
      0x02  4 × (i32 x, i32 z) quad corners, f32 base y, f32 height, f32 unknown (0)   44 bytes
      0x04  vec3 center, f32 radius                                                     16
      0x08  vec3, vec3, f32  (capsule?)                                                 28
      0x10  vec3 min, vec3 max                                                          24
      0x20  4 × vec4 matrix rows (last = position), vec3 half size                      76
      0x40  u8                                                                           1
      0x80  vec3, vec3, f32, u32, u8                                                    33
      other kinds: no data.   Shipped: 0x02 (most), 0x04 (maps 26, 31), 0x20 (map 46).
  FIELD (0xFD3AB0):  u32 mControlFlag, u32 unknown,
                     u32 n, n × {u8 mCommandId, u32 mEvaluateMode, u32 mFilter, u32 mFreeValue,
                                 u8 Num, u32 mID}       (COMMAND_LIST_WORK; none shipped)
                     u32 n, n × vec3 command area       (none shipped)
  GOAL (0xFD2760):   u32 mControlFlag, 5 × (i32 linkNo = another box or -1, vec3 pos),
                     u8 mGoalMode (0/4/5), u8 mGoalValue
  ENV (0xFD2860, COMMAND_AREA_LIST_WORK): u32 mControlFlag, u8 mDangerLevel, u8 mHateUpLevel,
                     f32 mShotRatio, f32 mDetectRaio, f32 mHearRangeRatio, f32 mSneekRatio,
                     u32 mInfoCommonFlag
  AVOID (0xFD2910):  u32 mControlFlag, u8 mAvoidMode, 5 × vec3 mAvoidPos, u32 mFreeValue, u8 mTarget
  WAIT (0xFD29F0):   u32 mControlFlag, u8 mCheckMode, 3 × vec3 mWaitPos, u32 mWaitValue
  SWIM (0xFD2AA0):   u32 mControlFlag, 3 × vec3 mSwimPos, u8 mSwimMode
  STRAIGHT (0xFD2B60): u32 mControlFlag, u8 mStraightMode
```

The `mControlFlag`s are read as u32 and stored as `!= 0`. The boxes are hand-placed zones: goal
points chain boxes together (`linkNo`), so this is the bots' high-level route/behaviour layer.

## Tools

```bash
cargo test -p lp2-arc
```

The `training_*` tests read the four Training stages from `LP2_DIR` (default `G:\Lost Planet 2`)
and skip when it is missing: byte-exact roundtrip of all three files, indices in range, links
symmetric and sharing their edge, area tiling, quadtree placement, IBL grid padding.

```bash
cargo run -p lp2-arc --release --bin lp2-navi -- obj "G:\Lost Planet 2\nativePC\arcXmlTmpPC\aArea_TUTOREAL_26.arc" navi26.obj
```

writes object `navmesh_<name>` (a face per polygon) and object `waypoints` (a point per way point,
a line per explicit link). `lp2-navi info <arc>` prints the counts.

| Map | Vertices | Polygons | Links | Areas | Way points | AI boxes |
|---|---|---|---|---|---|---|
| 26 (Cube) | 2075 | 3182 | 8612 | 7 | 1269 | 52 |
| 31 | 6505 | 10377 | 28648 | 15 | 913 | 59 |
| 41 | 5128 | 7902 | 21212 | 23 | 709 | 61 |
| 46 | 3443 | 4976 | 12996 | 13 | 333 | 88 |

## What a generator has to write

Build the structs in `lp2_arc::navi` and call `to_bytes()`; the totals are computed on write, and invalid data (a u8 list over 255, a link without its link attribute) is an error.

1. **LNV**: one NavigationList (path `tool\AI\NaviMesh\<dir>\Navi_<name>`, index 0, priority 0,
   parent -1, no links/outline) with a mesh:
   - triangles from the walkable collision (Recast/Detour polygons triangulated), one attribute
     u32 each (0 unless the meaning is found), unknowns as shipped: flag 0, vec (0,0,1), 0, empty list;
   - links for every pair of triangles that share an edge, both directions, `edge` = the edge's
     index in the source triangle, `cost` = centroid distance, unknown 0;
   - areas: `root` (AABB of everything, parent 0xFFFF, children 1..n, range 0..P) and tiles
     `area_aabb` with parent 0, a box, a contiguous polygon range (sort triangles by tile) and
     symmetric neighbour lists; u8 indices limit this to 255 areas, u16 ranges to 65535 polygons;
   - quadtree depth 7 over the vertex bounds (grow y by 1000), each leaf listing the triangles whose
     XZ bounds overlap it, pairs `(index, 0)`; no nodeInfo.
2. **WAY**: points on a 300-500 grid on walkable floor, one attribute each (0xFFFFFFFF is the most
   common value; meaning unknown), no links, mType 0, quadtree depth 5 over the bounds ± 50.
3. **IBL**: at least the grid corners and the 32x32 grid; boxes (zones with goals) are hand-made
   and map specific. Untested: whether bots move with zero boxes, or with boxes from another map.
4. Put them in the stage archive with the script-list entries mType 0x1D/0x1E/0x1F, as
   `crates/xlive/src/botmaps.rs` already does for the copied files.

Open questions: the meaning of the polygon and way point attribute masks, the WAY `mType`
geometries, IBL shape 0x08/0x80, and whether the game needs the area tiles (one root area may do).

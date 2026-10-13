# Map collision (rCollision / SBC)

How Lost Planet 2 stores a stage's static collision, and how to export it as input for a
navmesh generator. Parser: `crates/arc/src/sbc.rs`. Exporter: `lp2-arc obj`.

## Where it is

Every online stage has two `rCollision` resources (type hash `0x51fc779f`, file magic `SBC\xff`).
They are not in `aArea_ONLINE_XX.arc` itself: they sit in `arcXmlTmpPC\variable\aArea_ONLINE_XX_v.arc`
(also duplicated in `arcLp2Xml\autoArc\aArea_ONLINE_XX_scr.arc` and the Training `_v` archives).
The stage's `rScrList` (`Tool\ScrList\aArea_ONLINE_XX`, in the main archive) names them:

| name | meaning | use |
|---|---|---|
| `scr\...\atari<stage>` | 当たり "hit": character/vehicle movement collision | **navmesh input** |
| `scr\...\tama<stage>` | 弾 "bullet": shot collision, denser, includes decoration | not for navmesh |
| `collision\om\omXXXX\omXXXX_atari` | collision of one placed object (OM) | see "Objects" below |

The `rHkCollision` `havokdata\hav<stage>` beside them is the Havok physics copy (ragdolls, debris); not needed.

| map | atari name | parts | tris | verts |
|---|---|---|---|---|
| 00 | `scr\Online\online00\atarionline00` | 16 | 17626 | 10993 |
| 01 | `SCR\ONLINE\ONLINE01\ATARIONLINE01` | 112 | 13982 | 9197 |
| 02 | `scr\Online\online02\atarionline02` | 152 | 18099 | 13201 |
| 03 | `scr\Online\online03\atarionline03` | 87 | 14501 | 9700 |
| 04 | `scr\Online\online04\atarionline04` | 121 | 14076 | 11565 |
| 05 | `scr\Online\online05\atarionline05` | 1 | 9867 | 5200 |
| 06 | `scr\area4\a405\ataria405` | 18 | 6253 | 5153 |
| 07 | `scr\Area5\a502\ataria502` | 21 | 10815 | 8279 |
| 08 | `scr\Online\online08\atarionline08` | 10 | 11240 | 7686 |
| 09 | `scr\Online\online09\atarionline09` | 245 | 8318 | 5590 |
| 10 | `scr\online\online10\atarionline10` | 99 | 13528 | 8548 |
| 11 | `scr\Online\online11\atarionline11` | 138 | 40015 | 23648 |
| 12 (Cube) | `scr\Online\online12\atarionline12` | 2 | 298 | 157 |
| 15 | `scr\Area1\a114\ataria114` | 9 | 23703 | 13976 |

Map 06/07/15 reuse campaign stages (a405, a502, a114). The Training twins (`aArea_TUTOREAL_26..65_v`)
carry the same files (Cube's atari in TUTOREAL_26 is byte-identical).

## Layout

Little-endian, all offsets in bytes. Arrays follow each other with no padding.

```
0x00  char[4] "SBC\xff"
0x04  u32     version 0x77bfb5b8 (checked by the loader)
0x08  u16     part count     P
0x0a  u16     attribute count A
0x0c  u32     triangle count  T
0x10  u32     vertex count    V
0x14  u32     extra count 1 (array of u32 after the attributes; 0 in every file)
0x18  u32     extra count 2 (array of u32 after that; 0 in every file)
0x1c  u32     0
0x20  vec4    bbox min   (w = 0)
0x30  vec4    bbox max
0x40  Part[P]      48 bytes each
      Tri[T]       32 bytes each
      vec4[V]      vertices (x, y, z, w=0)
      Attr[A]      32 bytes each
      u32[extra1], u32[extra2]
      BVH[P]       one "BVHC" tree per part, then one more over the parts
```

`Part` (48 bytes): `vec4 min, vec4 max, u32 id (hash), u32 first_tri, u32 first_vert, i32 -1`.
A part owns triangles `first_tri ..` the next part's `first_tri`; its triangles index vertices
relative to `first_vert`. Parts are the separate meshes the artists made (floor, walls, a bridge, ...).

`Tri` (32 bytes): `f32 normal[3], u16 v[3] (relative to part.first_vert), u16 attr, 12 bytes 0`
(filled at run time). Winding is counter-clockwise seen from the normal side:
`(v1-v0) x (v2-v0)` points along the stored normal for every triangle in every online map (tested),
which is the convention Recast uses.

`Attr` (32 bytes): `u32 mask, u32 0, u32 0, u32 material, u32 0, 3 x u32 tool garbage`.
Triangles point at one with `Tri.attr`. `material` low byte is `0xff` (none) on stage meshes and a
small number on objects; it does not matter for walking.

`mask` is the hit-group mask a collision query tests against its own mask
(the query passes `attr` and its mask to a virtual filter, `0x4d2330`→`[vtbl+0x3c]`).
Values seen in the atari meshes:

| mask | where | reading |
|---|---|---|
| `19f8f6ff` | 90-99 % of every stage | ordinary solid ground and walls |
| `19feffff` | all OM object meshes; default collider mask at `0x83f46c` | ordinary solid (objects) |
| `40001000`, `40002000`, `40080000`, `40102000`, `40100000`, `40003000`, `40000400` | a few hundred triangles per map, mostly vertical, around play-area edges and ramps | invisible walls that stop only some colliders |
| `40004000` | maps 05 and 15, large mostly-flat areas | special floor (exact effect unknown) |
| `44000000`, `e0000000` | maps 04/06/07/08/09; `e0000000` spans far outside the play area | out-of-bounds / kill volumes |

Bit 30 (`0x40000000`) is set on every special surface and never on ordinary solid. Which single bits
mean "stops players", "stops vehicles (VS)" or "stops Akrid" was not pinned down (the masks the
movement queries pass were not traced), so the exporter keeps all special triangles in their own
OBJ groups rather than guessing.

BVH (`cBVHCollision`, loader `0x74ece0`): 0x40-byte header `"BVHC"`, version `0x77b17b24`, kind,
bbox, counts, then 4-wide SIMD nodes (bounding boxes stored as 4 min-x, 4 min-y, ... floats; leaf
marker bytes `PPPP`) and a `u16` triangle order list. A generator does not need it; rebuild it only if
writing SBC files.

### Evidence

- Loader `0x66e7b0` (LP2DX11.exe 1.0.0.129, image base 0x400000): reads 0x40 header bytes, checks
  `'S','B','C'` and `0x77bfb5b8`, stores `u16 [8]` (parts), `u16 [0xa]` (attrs), `u32 [0xc]` (tris),
  `u32 [0x10]` (verts), `u32 [0x14]`, `u32 [0x18]`, then streams parts `P*48`, tris `T*32`,
  verts `V*16`, attrs `A*32`, `[0x14]*4`, `[0x18]*4`, then one `cBVHCollision` per part
  (each 0x7a0 bytes, stored at part+0x20) and one for the whole mesh.
- `0x4d2330`/`0x4b9040`: a triangle hit is resolved as `tris + (part.first_tri + i)*32`,
  attribute `attrs + tri[0x12]*32`, vertex `verts + (part.first_vert + tri[0xc + 2k])*16`.
- Data: header counts reproduce every file's array boundaries exactly (the byte after the
  attributes is always `BVHC`), all indices are in range, every map parses (`cargo test -p lp2-arc`).

## Units and axes

Y up, game units are probably centimetres (Cube is 3500 floor to ceiling). Coordinates are world space: stage
meshes have no transform. Cube's atari mesh spans x -5731..5731, z -4231..4231, y 0..3500 (ceiling);
its floor is y = 0 and reaches past the in-game player positions (x ±5400, z -4000..2100).

## Export

```
cargo build -p lp2-arc --release
target\release\lp2-arc.exe obj "G:\Lost Planet 2\nativePC\arcXmlTmpPC\variable\aArea_ONLINE_12_v.arc" scr\Online\online12\atarionline12 cube.obj
```

The OBJ keeps game coordinates and has groups:

- `floor`: ordinary solid (`mask` bit 30 clear) with `normal.y >= cos 45°`
- `wall`: other ordinary solid (walls, ceilings, steep slopes)
- `special_<mask>`: every triangle with bit 30 set, one group per mask

## What Recast takes

Recast wants one triangle soup in world space with Y up, CCW winding, and does its own walkable
slope test (`walkableSlopeAngle`). So feed it **`floor` + `wall`** (all ordinary solid triangles:
walls are needed so it can carve them out and respect `walkableClimb`/height clearance), with
`walkableSlopeAngle` 45 and agent height ~180, radius ~40-60, climb ~50 in these units (to be
matched against the shipped Training navmeshes). Leave out `special_*` at first, or add the
`special_4000xxxx` walls as obstacles if bots walk through invisible walls. The `tama` mesh and the
`havokdata` copy are not inputs.

## Not covered

- **Objects.** Doors, crates, lifts, Cube's blocks (`om8360`..`om8382`) etc. are OM objects with
  their own `collision\om\...\*_atari` SBC files (same format, local coordinates), placed by the
  stage's `rObjSet` (`tool\ObjSet\Online\aArea_ONLINE_XX-<mode>-NN`, binary `OSF\0`, not decoded).
  A navmesh that should route around them needs that placement decoded; most are small.
- The exact meaning of each special mask bit (see above).

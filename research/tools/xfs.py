# MT Framework XFS (version 8, Lost Planet 2 PC) <-> JSON, e.g. the bots' AI files
# (tool\AI\Think\HumanBotBt = rThinkPlanAI, Tool\AI\Pef_Hm40 = rAIPositionEvaluationFuncList).
#   xfs.py dump FILE.xfs OUT.json    xfs.py pack IN.json OUT.xfs    xfs.py test FILE...
# Layout (all little endian; offsets in the definition block count from 0x10):
#   "XFS\0", u16 version 8, u16 unknown (1), u32 class count, u32 definition block size
#   u32 offset[count] -> class: u32 type hash (~crc32(name) & 0x7fffffff), u32 field count,
#     fields of 24 bytes: u32 name offset, u8 type, u8 attr, u16 size, 16 bytes (zero in LP2)
#   field names (NUL-terminated, each once, in order of first use), zero padding to 4 bytes,
#   then the data (at 0x10 + size):
#   object = u16 class index << 1 | 1, u16 object id, u32 size (counts itself), then per field
#     u32 count and count values: class/classref (1, 2) an object, bool/u8 (3, 4) 1 byte,
#     u32/s32/f32 (6, 0xa, 0xc) 4, u64 (7) 8, string (0xe) NUL-terminated, rect (0x12) 4 x s32.
import json, struct, sys, zlib

ONE = {3: "?", 4: "B", 6: "I", 7: "Q", 0xA: "i", 0xC: "f"}

def class_names():
    import os
    path = os.path.join(os.path.dirname(__file__), "..", "dx11", "classes.json")
    return {~zlib.crc32(c["name"].encode()) & 0x7FFFFFFF: c["name"] for c in json.load(open(path))}

def f32(x):  # shortest decimal that reads back as the same f32
    for p in range(1, 10):
        s = float("%.*g" % (p, x))
        if struct.pack("<f", s) == struct.pack("<f", x):
            return s
    return x

def load(d):
    names = class_names()
    assert d[:4] == b"XFS\0", "not XFS"
    ver, unk, n, size = struct.unpack_from("<HHII", d, 4)
    assert ver == 8, "XFS version %d" % ver
    cstr = lambda o: d[o:d.index(b"\0", o)].decode()
    classes = []
    for o in struct.unpack_from("<%dI" % n, d, 16):
        h, c = struct.unpack_from("<II", d, 16 + o)
        fields = []
        for k in range(c):
            no, t, a, sz = struct.unpack_from("<IBBH", d, 24 + o + 24 * k)
            extra = d[32 + o + 24 * k:48 + o + 24 * k]
            f = {"name": cstr(16 + no), "type": t, "attr": a, "size": sz}
            if any(extra):
                f["extra"] = extra.hex()
            fields.append(f)
        classes.append({"class": names.get(h, "0x%08x" % h), "hash": "0x%08x" % h, "fields": fields})
    pos = 16 + size

    def obj():
        nonlocal pos
        tag, oid, osz = struct.unpack_from("<HHI", d, pos)
        assert tag & 1, "object tag %#x at %#x" % (tag, pos)
        start, cls = pos + 4, classes[tag >> 1]
        pos += 8
        out = {"$class": cls["class"], "$id": oid}
        for f in cls["fields"]:
            cnt, = struct.unpack_from("<I", d, pos)
            pos += 4
            vals = [value(f["type"]) for _ in range(cnt)]
            out[f["name"]] = vals[0] if cnt == 1 else vals
        assert pos == start + osz, "%s size %#x, read %#x" % (cls["class"], osz, pos - start)
        return out

    def value(t):
        nonlocal pos
        if t in (1, 2):
            return obj()
        if t in ONE:
            v, = struct.unpack_from("<" + ONE[t], d, pos)
            pos += struct.calcsize(ONE[t])
            return f32(v) if t == 0xC else v
        if t == 0xE:
            e = d.index(b"\0", pos)
            v, pos = d[pos:e].decode("utf-8", "surrogateescape"), e + 1
            return v
        if t == 0x12:
            v = list(struct.unpack_from("<4i", d, pos))
            pos += 16
            return v
        raise ValueError("field type %#x at %#x" % (t, pos))

    root = obj()
    assert pos == len(d), "%d trailing bytes" % (len(d) - pos)
    return {"unknown_u16": unk, "classes": classes, "root": root}

def save(j):
    classes = j["classes"]
    index = {c["class"]: i for i, c in enumerate(classes)}
    strings, strpos = bytearray(), {}
    for c in classes:
        for f in c["fields"]:
            if f["name"] not in strpos:
                strpos[f["name"]] = len(strings)
                strings += f["name"].encode() + b"\0"
    n = len(classes)
    table, defs = [], bytearray()
    base = 4 * n
    for c in classes:
        table.append(base + len(defs))
        defs += struct.pack("<II", int(c["hash"], 16), len(c["fields"]))
        for f in c["fields"]:
            defs += struct.pack("<IBBH", 0, f["type"], f["attr"], f["size"])
            defs += bytes.fromhex(f.get("extra", "00" * 16))
    sbase = base + len(defs)
    # name offsets are only known once the definitions' size is
    k = 0
    for c in classes:
        k += 8
        for f in c["fields"]:
            struct.pack_into("<I", defs, k, sbase + strpos[f["name"]])
            k += 24
    head = struct.pack("<%dI" % n, *table) + defs + strings
    head += bytes(-len(head) % 4)  # the data starts 4-aligned

    def obj(o):
        cls = classes[index[o["$class"]]]
        body = bytearray()
        for f in cls["fields"]:
            v = o[f["name"]]
            flat = f["type"] == 0x12 and v and not isinstance(v[0], list)
            vals = [v] if not isinstance(v, list) or flat else v
            body += struct.pack("<I", len(vals))
            for x in vals:
                body += value(f["type"], x)
        return struct.pack("<HHI", index[o["$class"]] << 1 | 1, o["$id"], len(body) + 4) + body

    def value(t, x):
        if t in (1, 2):
            return obj(x)
        if t in ONE:
            return struct.pack("<" + ONE[t], x)
        if t == 0xE:
            return x.encode("utf-8", "surrogateescape") + b"\0"
        if t == 0x12:
            return struct.pack("<4i", *x)
        raise ValueError("field type %#x" % t)

    return b"XFS\0" + struct.pack("<HHII", 8, j["unknown_u16"], n, len(head)) + head + obj(j["root"])

def dumps(x, ind=""):  # JSON with objects that hold no other objects on one line
    if isinstance(x, dict) and any(isinstance(v, (dict, list)) and "{" in json.dumps(v) for v in x.values()):
        inner = ind + "  "
        return "{\n" + ",\n".join(inner + json.dumps(k) + ": " + dumps(v, inner) for k, v in x.items()) + "\n" + ind + "}"
    if isinstance(x, list) and any(isinstance(v, (dict, list)) for v in x) and "{" in json.dumps(x):
        inner = ind + "  "
        return "[\n" + ",\n".join(inner + dumps(v, inner) for v in x) + "\n" + ind + "]"
    return json.dumps(x)

def arc_files(path, types):  # (name, data) of the entries of an ARC v8 whose type hash is in `types`
    d = open(path, "rb").read()
    n, = struct.unpack_from("<H", d, 6)
    for k in range(n):
        name, t, csize, _, off = struct.unpack_from("<64sIIII", d, 8 + 80 * k)
        if t in types:
            yield name.split(b"\0")[0].decode(), zlib.decompress(d[off:off + csize])

def plans(j):  # rThinkPlanAI as one line per plan plus its filter/evaluation/move functions
    items = lambda a: a["mpArray"] if isinstance(a["mpArray"], list) else [a["mpArray"]]
    fn = lambda o: o["$class"].replace("cAIPlan", "") + "#%d(" % o["$id"] + ", ".join(
        "%s=%s" % (k[1:], fn(v) if isinstance(v, dict) else v) for k, v in o.items() if not k.startswith("$")) + ")"
    out = []
    for i, p in enumerate(items(j["root"]["mSaveInfo.itemAIInfoArray"])):
        out.append("[%d] %s #%d attr=%#x root=%#x tired=%g frustration=%g recover=%g next=%s pre=%s" % (
            i, p["mName"], p["$id"], p["mAttr"], p["mRootAttr"], p["mTiredPoint"], p["mFrustrationPoint"],
            p["mTiredRecoverPoint"], [c["mValue"] for c in items(p["mItemNextIndexArray"])],
            [c["mValue"] for c in items(p["mItemPreIndexArray"])]))
        for k in ("mFilterFuncList", "mEvaluationFuncList", "mMoveFuncList"):
            out += ["    %-6s %s" % (k[1:5], fn(o)) for o in items(p[k])]
    return "\n".join(out)

GAME = r"G:\Lost Planet 2\nativePC"
AI_TYPES = {0x0DE13237, 0x20A81BF0}  # rThinkPlanAI, rAIPositionEvaluationFuncList

if __name__ == "__main__":
    cmd, args = sys.argv[1], sys.argv[2:]
    if cmd == "dump":
        open(args[1], "w", encoding="utf-8").write(dumps(load(open(args[0], "rb").read())) + "\n")
    elif cmd == "pack":
        open(args[1], "wb").write(save(json.load(open(args[0], encoding="utf-8"))))
    elif cmd == "plans":
        print(plans(load(open(args[0], "rb").read())))
    elif cmd == "test":  # every file must survive dump -> text -> pack byte for byte
        import glob, os
        game = args[0] if args else GAME  # read only: archives are opened, never written
        arcs = sorted(glob.glob(os.path.join(game, "arcLp2Xml", "autoArc", "uHm6?.arc")))
        arcs += sorted(glob.glob(os.path.join(game, "arcXmlTmpPC", "aArea_TUTOREAL_*.arc")))
        count = 0
        for a in arcs:
            for name, d in arc_files(a, AI_TYPES):
                assert save(json.loads(dumps(load(d)))) == d, "%s %s: round trip differs" % (a, name)
                count += 1
        assert count >= 15, "found only %d AI files under %s" % (count, game)
        print("ok: %d files from %d archives round-trip byte for byte" % (count, len(arcs)))
    else:
        sys.exit("usage: xfs.py dump FILE OUT.json | pack IN.json FILE | plans FILE | test [nativePC dir]")

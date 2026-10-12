# Usage: tuning_defaults.py EXE tuning.json -> fills `default` from `mov dword ptr [addr], imm32` (C7 05) in .text
import sys, pefile, json, struct, re
exe, tj = sys.argv[1], sys.argv[2]
pe = pefile.PE(exe, fast_load=True); base = pe.OPTIONAL_HEADER.ImageBase
text = next(s for s in pe.sections if s.Name.startswith(b".text")); code = text.get_data()
rows = json.load(open(tj)); want = {r["addr"]: r for r in rows}
hits = {}
for m in re.finditer(b"\xC7\x05", code):
    i = m.start()
    if i + 10 > len(code): break
    a, v = struct.unpack_from("<II", code, i + 2)
    if a in want: hits.setdefault(a, []).append(v)
for r in rows:
    vs = hits.get(r["addr"])
    if r["as_u32"] is None and vs:
        r["code_writes"] = vs
        r["default_f32"] = round(struct.unpack("<f", struct.pack("<I", vs[0]))[0], 6)
json.dump(rows, open(tj, "w"), indent=0)
n = sum(1 for r in rows if r["as_u32"] is not None or r.get("code_writes"))
print(f"defaults known for {n}/{len(rows)}")

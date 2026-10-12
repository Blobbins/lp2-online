# Usage: dti.py EXE DTI_CTOR_VA OUT.json
# Finds every `push a; push b; push size; push parent; push name; mov ecx, dti; call DTI_CTOR` and the
# DTI vtable stored right after. Output: [{name, dti, parent, parent_name, size, flags, dti_vtable}]
import sys, pefile, json
from capstone import Cs, CS_ARCH_X86, CS_MODE_32
exe, ctor, out = sys.argv[1], int(sys.argv[2], 16), sys.argv[3]
pe = pefile.PE(exe, fast_load=True); base = pe.OPTIONAL_HEADER.ImageBase
text = next(s for s in pe.sections if s.Name.startswith(b".text"))
code = text.get_data(); tva = base + text.VirtualAddress
def cstr(va):
    try: b = pe.get_data(va - base, 128)
    except Exception: return None
    s = b.split(b"\0")[0]
    return s.decode("ascii") if s and all(32 <= c < 127 for c in s) else None
md = Cs(CS_ARCH_X86, CS_MODE_32); md.skipdata = True
# Scan for E8 rel32 calls to ctor, then disassemble a window before each.
import struct
res = {}
for i in range(len(code) - 5):
    if code[i] != 0xE8: continue
    tgt = (tva + i + 5 + struct.unpack_from("<i", code, i + 1)[0]) & 0xFFFFFFFF
    if tgt != ctor: continue
    win = list(md.disasm(code[i - 40:i + 25], tva + i - 40))
    k = next((j for j, x in enumerate(win) if x.address == tva + i), None)
    if k is None or k < 6: continue
    seq = win[k - 6:k]
    if seq[-1].mnemonic != "mov" or not seq[-1].op_str.startswith("ecx, 0x"): continue
    pushes = [x for x in seq[:-1] if x.mnemonic == "push"]
    if len(pushes) != 5: continue
    vals = [int(p.op_str, 16) if p.op_str.startswith("0x") else int(p.op_str) for p in pushes]
    flags2, flags, size, parent, name_va = vals
    name = cstr(name_va)
    if not name: continue
    dti = int(seq[-1].op_str.split(", ")[1], 16)
    after = [x for x in win[k + 1:] if x.mnemonic == "mov" and x.op_str.startswith(f"dword ptr [{dti:#x}], 0x")]
    vt = int(after[0].op_str.split(", ")[1], 16) if after else None
    res[dti] = dict(name=name, dti=dti, parent=parent, size=size, flags=flags, dti_vtable=vt)
for r in res.values():
    p = res.get(r["parent"]); r["parent_name"] = p["name"] if p else None
json.dump(sorted(res.values(), key=lambda r: r["name"]), open(out, "w"), indent=0)
print(len(res), "classes")

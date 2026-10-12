# Usage: tuning.py EXE REGISTER_FN_VA OUT.json
# Every `push 0; push 0; push addr; push name; mov ecx, esi; call REGISTER` -> name, address, default value.
import sys, pefile, struct, json
from capstone import Cs, CS_ARCH_X86, CS_MODE_32
exe, fn, out = sys.argv[1], int(sys.argv[2], 16), sys.argv[3]
pe = pefile.PE(exe, fast_load=True); base = pe.OPTIONAL_HEADER.ImageBase
text = next(s for s in pe.sections if s.Name.startswith(b".text"))
code = text.get_data(); tva = base + text.VirtualAddress
md = Cs(CS_ARCH_X86, CS_MODE_32)
def cstr(va):
    s = pe.get_data(va - base, 96).split(b"\0")[0]
    return s.decode("ascii", "replace")
rows = []
for i in range(len(code) - 5):
    if code[i] != 0xE8: continue
    if (tva + i + 5 + struct.unpack_from("<i", code, i + 1)[0]) & 0xFFFFFFFF != fn: continue
    win = list(md.disasm(code[i - 30:i + 5], tva + i - 30))
    k = next((j for j, x in enumerate(win) if x.address == tva + i), None)
    if k is None: continue
    pushes = [x for x in win[:k] if x.mnemonic == "push"][-4:]
    if len(pushes) != 4 or not all(p.op_str.startswith("0x") or p.op_str.isdigit() for p in pushes): continue
    a3, a2, addr, name = [int(p.op_str, 0) for p in pushes]
    raw = pe.get_data(addr - base, 4)
    init = len(raw) == 4  # shorter = .bss: the game sets the default in code at runtime
    rows.append(dict(name=cstr(name), addr=addr, arg3=a2, arg4=a3,
                     as_f32=round(struct.unpack("<f", raw)[0], 6) if init else None,
                     as_u32=struct.unpack("<I", raw)[0] if init else None))
json.dump(rows, open(out, "w"), indent=0)
print(len(rows), "tuning entries")

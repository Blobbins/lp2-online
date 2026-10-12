# Usage: singleton.py EXE classes.json NAME... -> constructor and global instance pointer of each cSystem singleton.
# DTI vtable slot 1 (newInstance) allocates and calls the constructor; the constructor stores `this`
# into a global (`mov ds:G, reg`), and the class vtable via `mov [reg], imm`.
import sys, json, pefile, struct
from capstone import Cs, CS_ARCH_X86, CS_MODE_32
exe, cj, names = sys.argv[1], sys.argv[2], sys.argv[3:]
pe = pefile.PE(exe, fast_load=True); base = pe.OPTIONAL_HEADER.ImageBase
cls = {c["name"]: c for c in json.load(open(cj))}
md = Cs(CS_ARCH_X86, CS_MODE_32)
def dis(va, n=0x200): return list(md.disasm(pe.get_data(va - base, n), va))
def u32(va): return struct.unpack("<I", pe.get_data(va - base, 4))[0]
for n in names:
    c = cls[n]; new = u32(c["dti_vtable"] + 4)
    calls = [int(i.op_str, 16) for i in dis(new, 0x80) if i.mnemonic == "call" and i.op_str.startswith("0x")]
    found = []
    for ctor in calls:
        for i in dis(ctor, 0x300):
            if i.mnemonic == "mov" and i.op_str.startswith("dword ptr [0x") and ", e" in i.op_str and "ptr [0x" in i.op_str:
                found.append((ctor, i.op_str.split("[")[1].split("]")[0]))
            if i.mnemonic == "ret": break
    print(n, "newInstance", hex(new), "globals written by ctor:", [(hex(a), g) for a, g in found][:6])

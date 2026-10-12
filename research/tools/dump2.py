# Usage: dump2.py file.dmp -> registers at the exception, words around them, and return addresses on the stack
import sys, struct
from minidump.minidumpfile import MinidumpFile
path = sys.argv[1]; md = MinidumpFile.parse(path); r = md.get_reader(); raw = open(path, "rb").read()
exc = md.exception.exception_records[0]
loc = exc.ThreadContext
rva = getattr(loc, "Rva", None) or getattr(loc, "rva")
ctx = raw[rva:rva + 0xcc]
names = {"Edi":0x9c,"Esi":0xa0,"Ebx":0xa4,"Edx":0xa8,"Ecx":0xac,"Eax":0xb0,"Ebp":0xb4,"Eip":0xb8,"Esp":0xc4}
regs = {k: struct.unpack_from("<I", ctx, o)[0] for k, o in names.items()}
print(" ".join(f"{k}={v:08x}" for k, v in regs.items()))
mods = [(m.baseaddress, m.baseaddress + m.size, m.name.split("\\")[-1]) for m in md.modules.modules]
def mod(a):
    for b, e, n in mods:
        if b <= a < e: return f"{n}+{a-b:#x}"
def rd(a, n):
    try: return r.read(a, n)
    except Exception: return None
for k in ["Edi", "Ebx", "Esi", "Ebp"]:
    b = rd(regs[k], 0x20); print(f"[{k}]", b.hex(" ", 4) if b else "unreadable")
data = rd(regs["Esp"], 0x800) or b""
print("stack:", len(data), "bytes")
for i in range(0, len(data) - 3, 4):
    v = struct.unpack_from("<I", data, i)[0]; m = mod(v)
    if m and ("LP2DX" in m or "xlive" in m.lower()): print(f"  esp+{i:#05x}: {v:08x} {m}")

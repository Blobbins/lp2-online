# Usage: xref.py EXE name [name...]  -> code sites that reference each string (push/mov imm32), with context.
import sys, pefile, struct, re
from capstone import Cs, CS_ARCH_X86, CS_MODE_32
exe = sys.argv[1]; names = sys.argv[2:]
pe = pefile.PE(exe, fast_load=True); base = pe.OPTIONAL_HEADER.ImageBase
data = pe.__data__
text = next(s for s in pe.sections if s.Name.startswith(b".text"))
tdata = text.get_data(); tva = base + text.VirtualAddress
def off2va(o):
    for s in pe.sections:
        if s.PointerToRawData <= o < s.PointerToRawData + s.SizeOfRawData:
            return base + s.VirtualAddress + o - s.PointerToRawData
md = Cs(CS_ARCH_X86, CS_MODE_32)
for n in names:
    for m in re.finditer(re.escape(n.encode()) + b"\x00", data):
        if m.start() and data[m.start()-1] != 0: continue
        va = off2va(m.start())
        if not va: continue
        for x in re.finditer(re.escape(struct.pack("<I", va)), tdata):
            site = tva + x.start() - 1
            start = max(0, x.start() - 40)
            print(f"== {n} @{va:#x} ref ~{site:#x}")
            for ins in md.disasm(tdata[start:x.start()+40], tva + start):
                if ins.address > site + 30: break
                if ins.address >= site - 24: print(f"   {ins.address:#x}: {ins.mnemonic} {ins.op_str}")
